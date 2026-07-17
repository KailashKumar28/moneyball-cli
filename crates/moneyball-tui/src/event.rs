//! Event loop: input handling, palette keys, scroll, and the
//! LLM stream drain.

use crate::*;

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

// Two chrono types collide on import name; alias the plain Utc.

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
        drain_stream(app);
    }
    Ok(())
}

/// Drain any pending worker events (LLM stream deltas, fetch results)
/// into the app. Runs every tick, so text appears as it arrives
/// (~10 redraws/sec). The receiver borrow is released before each event
/// is handled because fetch completion chains into a NEW stream
/// (`on_fetch_done` -> call_agent).
fn drain_stream(app: &mut App) {
    use std::sync::mpsc::TryRecvError;
    loop {
        let ev = {
            let Some(rx) = &app.stream else { return };
            rx.try_recv()
        };
        match ev {
            Ok(StreamEvent::Agent(aev)) => {
                use moneyball_core::agent::Ev;
                match aev {
                    Ev::AssistantDelta(d) => app.chat.append_assistant(&d),
                    Ev::AssistantDone { .. } => app.chat.finish_streaming(),
                    Ev::ToolBegin { name, args, .. } => {
                        app.chat.finish_streaming();
                        app.chat.push(chat::Cell::ToolCall(chat::cells::ToolCall {
                            name,
                            args: crate::app::compact_args(&args),
                            status: chat::cells::ToolStatus::Running,
                        }));
                    }
                    Ev::ToolEnd { output, ok, .. } => {
                        app.chat
                            .push(chat::Cell::ToolResult(chat::cells::ToolResult {
                                name: "tool".into(),
                                output: output.lines().map(String::from).collect(),
                                success: ok,
                                duration_ms: 0,
                            }));
                    }
                    Ev::TurnComplete {
                        items,
                        ms,
                        provider,
                    } => {
                        for item in items {
                            app.record(item);
                        }
                        app.chat.finish_streaming();
                        // Latency/provider is status metadata, never
                        // part of the persisted message text.
                        app.status = Some(format!("{}ms via {}", ms, provider));
                        app.turn_active = false;
                        app.stream = None;
                        return;
                    }
                    Ev::TurnAborted { items } => {
                        for item in items {
                            app.record(item);
                        }
                        app.chat.finish_streaming();
                        app.chat.push(chat::Cell::System(chat::cells::System(
                            "(turn interrupted)".into(),
                        )));
                        app.turn_active = false;
                        app.stream = None;
                        return;
                    }
                    Ev::Failed { error, items } => {
                        for item in items {
                            app.record(item);
                        }
                        app.chat
                            .append_assistant(&format!("llm call failed: {}", error));
                        app.chat.finish_streaming();
                        app.turn_active = false;
                        app.stream = None;
                        return;
                    }
                }
            }
            Ok(StreamEvent::FetchDone { report, days, ms }) => {
                app.stream = None;
                commands::on_fetch_done(app, report, days, ms);
                return;
            }
            Ok(StreamEvent::FetchFailed { err, days, ms }) => {
                app.stream = None;
                commands::on_fetch_failed(app, err, days, ms);
                return;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // The worker died without a final event (panic in a
                // tool or the forwarder). Never a silent dead turn:
                // say so, and release the turn so Esc/submit work.
                app.chat.finish_streaming();
                if app.turn_active {
                    app.chat.push(chat::Cell::System(chat::cells::System(
                        "agent worker stopped unexpectedly - resend your message".into(),
                    )));
                }
                app.turn_active = false;
                app.cancel
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                app.stream = None;
                return;
            }
        }
    }
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
            // Strip whitespace and newlines so paste of a token works even if
            // it was wrapped or had trailing whitespace in the clipboard.
            let clean: String = text.chars().filter(|c| !c.is_control()).collect();
            match (state.step, state.meta_substep) {
                (1, 0) => state.meta_input.push_str(&clean),
                (1, 2) => state.meta_rename_input.push_str(&clean),
                (2, _) => state.product_input.push_str(&clean),
                (3, _) => state.goals_input.push_str(&clean),
                (0, _) => state.workspace_path.push_str(&clean),
                _ => {}
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
            if let Some(p) = app.input[..app.cursor].chars().next_back() {
                app.cursor -= p.len_utf8();
            }
        }
        KeyCode::Right => {
            arm_cancel(app);
            if let Some(n) = app.input[app.cursor..].chars().next() {
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
    if app.turn_active {
        app.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        app.status = Some("interrupting...".into());
        return;
    }
    // Esc during a fetch: the pull cannot be stopped mid-request; say so
    // honestly instead of pretending (the result is discarded).
    if app.stream.take().is_some() {
        app.chat.finish_streaming();
        app.status = Some("fetch result discarded (the Meta pull itself completes)".into());
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
    let prev = app.input[..app.cursor].chars().next_back().unwrap();
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
            let before = app.input[..app.cursor]
                .rfind(' ')
                .map(|n| n + 1)
                .unwrap_or(0);
            let after = app.input[app.cursor..].to_string();
            let mut new = String::with_capacity(c.len() + after.len() + (app.cursor - before));
            new.push_str(&app.input[..before]);
            new.push_str(c);
            new.push_str(&after);
            app.input = new;
            app.cursor = before + c.len();
        }
    }
}

#[cfg(test)]
mod paste_tests {
    use super::*;
    use std::path::PathBuf;

    fn make_state(step: usize, substep: u8) -> SetupState {
        let mut s = SetupState::new(PathBuf::from("/tmp/mb-test"));
        s.workspace_path = "/tmp/mb-test".into();
        s.step = step;
        s.meta_substep = substep;
        s
    }

    fn app_with_setup(state: SetupState) -> App {
        let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
        let mut app = App::new_for_test(cfg);
        app.force_setup_for_test(state);
        app
    }

    #[test]
    fn paste_meta_token_into_substep0() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "EAA12345abcdefghij".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_input, "EAA12345abcdefghij");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_strips_newlines_and_control_chars() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        // Real clipboard often wraps a token with a trailing newline.
        handle_paste(&mut app, "EAA12345\n".into());
        handle_paste(&mut app, "abc\tdef\r".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_input, "EAA12345abcdef");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_workspace_step() {
        let s = make_state(0, 0);
        let mut app = app_with_setup(s);
        // Clear the default workspace path before pasting.
        match &mut app.view {
            View::Setup(state) => state.workspace_path.clear(),
            _ => unreachable!(),
        }
        handle_paste(&mut app, "/tmp/pasted-workspace".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.workspace_path, "/tmp/pasted-workspace");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_product_input() {
        let s = make_state(2, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "FincityOfficial act_1".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.product_input, "FincityOfficial act_1");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_goals_input() {
        let s = make_state(3, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "Namma Mane=12".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.goals_input, "Namma Mane=12");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_multi_select_is_ignored() {
        // substep 1 has no text input; paste should not change selection.
        let s = make_state(1, 1);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "should be dropped".into());
        match app.view {
            View::Setup(state) => {
                assert!(state.meta_input.is_empty());
                assert!(state.meta_rename_input.is_empty());
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_rename_substep() {
        let s = make_state(1, 2);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "1=BrandName".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_rename_input, "1=BrandName");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_brief_input() {
        let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
        let mut app = App::new_for_test(cfg);
        // resolve_optional yields a setup view when no workspace config exists,
        // so we override that here for the brief-view paste test.
        app.force_welcome_for_test();
        handle_paste(&mut app, "what is my best product?".into());
        assert_eq!(app.input, "what is my best product?");
    }

    #[test]
    fn empty_paste_is_noop() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "".into());
        match app.view {
            View::Setup(state) => assert!(state.meta_input.is_empty()),
            _ => panic!("expected Setup view"),
        }
    }

    /// Esc on the multi-select step (substep 1) returns to the token paste
    /// (substep 0) WITHOUT nuking the discovered accounts or the user's
    /// per-row selections. The token input is restored as N bullets so the
    /// box doesn't look empty. Regression test for the bug where going
    /// back from substep 1 dropped all of `meta_discovered` / selections.
    #[test]
    fn esc_from_multi_select_preserves_state() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use moneyball_core::meta::AdAccount;
        let mut s = make_state(1, 1);
        s.meta_token_len = 124;
        s.meta_discovered = vec![
            AdAccount {
                id: "act_1".into(),
                name: "Acme".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_2".into(),
                name: "Beta".into(),
                account_status: Some(1),
            },
        ];
        s.meta_selections = vec![true, false];
        s.meta_selected = vec![0];
        s.meta_highlight = 1;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_substep, 0, "should be back on token paste");
                assert_eq!(state.meta_discovered.len(), 2, "discovered list preserved");
                assert_eq!(
                    state.meta_selections,
                    vec![true, false],
                    "selections preserved"
                );
                assert_eq!(state.meta_selected, vec![0], "selected indices preserved");
                // Token input restored as N bullets.
                assert_eq!(state.meta_input.chars().count(), 124);
                assert!(state.meta_input.chars().all(|c| c == '\u{2022}'));
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Esc on the rename step (substep 2) returns to the multi-select
    /// (substep 1) WITHOUT nuking selections; only the rename input is
    /// cleared so the user can re-type.
    #[test]
    fn esc_from_rename_preserves_selections() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(1, 2);
        s.meta_rename_input = "1=Acme".into();
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_substep, 1, "should be back on multi-select");
                assert!(state.meta_rename_input.is_empty(), "rename input cleared");
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Regression test: the LLM provider picker must NOT eat Enter /
    /// Esc / Backspace / Char keys. Only Up/Down/Home/End belong to
    /// the picker. If the picker consumed Enter, advance_setup never
    /// ran and the user was stuck. This is the bug the user hit in
    /// the initial setup.
    #[test]
    fn llm_picker_does_not_consume_enter() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(4, 0);
        s.llm_highlight = 0;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                // Enter on the provider picker should pick "openai"
                // (the first preset) and advance to substep 1 (key paste).
                assert_eq!(state.llm_substep, 1, "should advance to key paste");
                assert_eq!(state.llm_provider_id, "openai");
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Picker nav (Up/Down) should still work and be consumed by the
    /// picker (so it doesn't fall through to backspace/insert).
    #[test]
    fn llm_picker_down_arrow_advances_highlight() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(4, 0);
        s.llm_highlight = 0;
        s.llm_scroll = 0;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.llm_highlight, 1, "Down should move highlight");
            }
            _ => panic!("expected Setup view"),
        }
    }

}
