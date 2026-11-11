//! Event loop: input handling, palette keys and scroll; each tick drains
//! the workers (the LLM stream, the refresh, the CRM connect).

use crate::*;

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn event_loop(t: &mut Tui, app: &mut App) -> Result<()> {
    let tick = Duration::from_millis(100);
    loop {
        t.draw(|f| render::render(f, app))?;
        if app.quit {
            break;
        }
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(k) => handle_key(app, k),
                // crossterm 0.28 emits Event::Paste for clipboard pastes; route
                // to whichever input field is currently focused.
                Event::Paste(text) => handle_paste(app, text),
                // Wheel scrolls the transcript (chat view only).
                Event::Mouse(m) => {
                    if matches!(app.view, View::Brief) {
                        match m.kind {
                            crossterm::event::MouseEventKind::ScrollUp => app.chat.scroll_up(3),
                            crossterm::event::MouseEventKind::ScrollDown => app.chat.scroll_down(3),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        crate::stream_drain::drain(app);
        crate::refresh_flow::drain(app);
        crate::crm_connect::drain(app);
    }
    Ok(())
}

/// Route a pasted string into the currently focused input field. Without this,
/// clipboard pastes (e.g. the Meta access token in the setup wizard) are
/// silently dropped because crossterm emits Event::Paste, not a stream of
/// KeyEvents.
fn handle_paste(app: &mut App, text: String) {
    if text.is_empty() {
        return;
    }
    match &mut app.view {
        View::Setup(state) => {
            // A paste is typing, one character at a time, through the same
            // insert every step uses (a per-step list here dropped the model
            // key). Newlines and controls go: a wrapped token still pastes.
            for c in text.chars().filter(|c| !c.is_control()) {
                setup::insert_setup(state, c);
            }
        }
        View::Brief => {
            // One atomic insert at the cursor; newlines become spaces so
            // a multi-line paste can never auto-submit; controls drop.
            let clean: String = text
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                .filter(|c| !c.is_control())
                .collect();
            app.input.insert_str(app.cursor, &clean);
            app.cursor += clean.len();
        }
    }
}

fn handle_key(app: &mut App, k: KeyEvent) {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && k.code == KeyCode::Char('o')
        && matches!(app.view, View::Brief)
    {
        return crate::refresh_flow::open_report(app);
    }
    if crate::crm_connect::on_key(app, k) || crate::schedule_view::on_key(app, k) {
        return;
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        // Double-press to quit: a single stray Ctrl+C clears the input
        // instead of killing the session (and the wizard) instantly.
        let armed = app
            .ctrl_c_armed
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2));
        if armed {
            app.quit = true;
        } else {
            app.ctrl_c_armed = Some(std::time::Instant::now());
            app.input.clear();
            app.cursor = 0;
            app.completions.clear();
            app.completion_idx = None;
            app.status = Some("press ctrl+c again to quit".into());
        }
        return;
    }
    app.ctrl_c_armed = None;
    match &app.view.clone() {
        View::Setup(state) => setup::handle_setup_key(app, state.clone(), k),
        View::Brief => handle_brief_key(app, k),
    }
}

// ---------- brief-view keys ----------

fn handle_brief_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => handle_esc(app),
        KeyCode::Tab => {
            arm_cancel(app);
            if app.input.starts_with('/') && app.completions.is_empty() {
                app.completions = commands::completions(&app.input);
                app.completion_idx = if app.completions.is_empty() {
                    None
                } else {
                    Some(0)
                };
            } else if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            }
            apply_completion(app);
        }
        KeyCode::Backspace => {
            backspace(app);
            refresh_completions(app);
            arm_cancel(app);
        }
        KeyCode::Char(c) => {
            insert(app, c);
            refresh_completions(app);
            arm_cancel(app);
        }
        KeyCode::Enter => {
            arm_cancel(app);
            // Codex-style palette: Enter on a partial slash command runs
            // the SELECTED entry instead of submitting the raw prefix
            // (which would fall through to the LLM as free-form chat).
            if !app.completions.is_empty() && app.input.starts_with('/') && !app.input.contains(' ')
            {
                apply_completion(app);
            }
            commands::submit(app);
        }
        KeyCode::Up => {
            arm_cancel(app);
            if !app.completions.is_empty() {
                let n = app.completions.len();
                let i = app.completion_idx.unwrap_or(0);
                app.completion_idx = Some((i + n - 1) % n);
            } else {
                // No palette open: Up scrolls the transcript.
                app.chat.scroll_up(1);
            }
        }
        KeyCode::Down => {
            arm_cancel(app);
            if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            } else {
                app.chat.scroll_down(1);
            }
        }
        KeyCode::PageUp => {
            arm_cancel(app);
            app.chat.scroll_up(10);
        }
        KeyCode::PageDown => {
            arm_cancel(app);
            app.chat.scroll_down(10);
        }
        KeyCode::Home => {
            arm_cancel(app);
            app.chat.scroll_to_top();
        }
        KeyCode::End => {
            arm_cancel(app);
            app.chat.scroll_to_bottom();
        }
        // Char-aware moves: the cursor is a byte index, so stepping by 1
        // through a multibyte char would panic the next insert.
        KeyCode::Left => {
            arm_cancel(app);
            if let Some(p) = app.input.split_at(app.cursor).0.chars().next_back() {
                app.cursor -= p.len_utf8();
            }
        }
        KeyCode::Right => {
            arm_cancel(app);
            if let Some(n) = app.input.split_at(app.cursor).1.chars().next() {
                app.cursor += n.len_utf8();
            }
        }
        _ => {
            arm_cancel(app);
        }
    }
}

/// Esc on the chat view is the universal "let me rethink" gesture:
///   - Input non-empty: clear the input, drop completions. Status hint.
///   - Input empty: show a hint pointing to /exit. Never quits.
///   - Use /exit (or /quit, /q) to leave moneyball.
fn handle_esc(app: &mut App) {
    // Esc during an agent turn: set the cancel flag - the worker aborts
    // between SSE events (real HTTP cancel) and sends TurnAborted, which
    // records the partial items. The receiver stays alive for that.
    if app.session.interrupt() {
        app.status = Some("interrupting...".into());
        return;
    }
    // Esc during /report: it can't be cut mid-write; say so honestly
    // instead of pretending (the result is discarded).
    if app.stream.take().is_some() {
        app.chat.finish_streaming();
        app.status = Some("stopped waiting - the job finishes in the background".into());
        return;
    }
    if !app.input.is_empty() {
        app.input.clear();
        app.cursor = 0;
        app.completions.clear();
        app.completion_idx = None;
        app.status = Some("input cleared".into());
        return;
    }
    // A refresh stops after the step in flight (a Meta pull can't be cut).
    if crate::refresh_flow::cancel(app) {
        return;
    }
    app.status = Some("esc clears the input. use /exit to leave moneyball.".into());
}

fn arm_cancel(_app: &mut App) {
    // Kept as a no-op for now so callers don't break. Esc no longer arms a
    // quit shortcut in this build (user request: /exit is the only way out).
}

fn insert(app: &mut App, c: char) {
    app.input.insert(app.cursor, c);
    app.cursor += c.len_utf8();
}

fn backspace(app: &mut App) {
    if app.cursor == 0 {
        return;
    }
    let prev = app
        .input
        .split_at(app.cursor)
        .0
        .chars()
        .next_back()
        .unwrap();
    app.cursor -= prev.len_utf8();
    app.input.remove(app.cursor);
}

fn refresh_completions(app: &mut App) {
    if app.input.starts_with('/') {
        app.completions = commands::completions(&app.input);
        app.completion_idx = if app.completions.is_empty() {
            None
        } else {
            Some(0)
        };
    } else {
        app.completions.clear();
        app.completion_idx = None;
    }
}

fn apply_completion(app: &mut App) {
    if let Some(i) = app.completion_idx {
        if let Some(&c) = app.completions.get(i) {
            // Replace the current token (up to cursor) with completion.
            // Cursor and the space found by rfind are char boundaries.
            let (upto, after) = app.input.split_at(app.cursor);
            let before = upto.rfind(' ').map(|n| n + 1).unwrap_or(0);
            let head = upto.split_at(before).0;
            let mut new = String::with_capacity(head.len() + c.len() + after.len());
            new.push_str(head);
            new.push_str(c);
            new.push_str(after);
            app.input = new;
            app.cursor = before + c.len();
        }
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
