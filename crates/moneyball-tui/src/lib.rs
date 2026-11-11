//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

mod agent_turn;
mod app;
mod brief_flow;
pub mod chat;
mod commands;
mod connect_view;
mod creative_flow;
mod crm_connect;
mod crm_flow;
mod diagnose_flow;
mod event;
mod funnel_flow;
mod landing;
mod ledger_view;
pub(crate) mod markdown;
mod refresh_cell;
mod refresh_flow;
mod render;
mod report;
mod schedule_view;
mod setup;
mod status_view;
mod stream_drain;
pub mod widgets;
pub(crate) use app::View;
pub use app::{App, StreamEvent};
use event::event_loop;
pub use setup::SetupState;

use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use moneyball_core::agent::Session;
use moneyball_core::session::SessionLog;
use moneyball_core::AppConfig;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> Result<Tui> {
    // Panic hook FIRST (codex-tui pattern): a panic anywhere in the
    // event loop unwinds past the normal restore() call, and without
    // this the user's shell is left in raw mode + alt screen with the
    // panic message invisible until they run `reset`.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        prev(info);
    }));
    enable_raw_mode()?;
    let mut out = io::stdout();
    // Mouse capture lets the wheel scroll the transcript instead of the
    // terminal window (text selection becomes Shift+drag - standard TUI
    // trade-off, same as codex).
    // Bracketed paste: a multi-line paste arrives as ONE Event::Paste
    // instead of raw keys - without it every pasted newline acted as
    // Enter and fired real LLM calls.
    execute!(
        out,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

pub fn restore() -> Result<()> {
    // Every step best-effort: one failure must not skip the rest, or
    // the terminal stays half-broken (e.g. raw mode off but still in
    // the alt screen).
    let raw = disable_raw_mode();
    let _ = execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
    let alt = execute!(io::stdout(), LeaveAlternateScreen);
    raw?;
    alt?;
    Ok(())
}

// ---------- slash-command surface ----------

// ---------- app state ----------

// ---------- main entry ----------

pub fn run() -> Result<()> {
    run_with(None)
}

/// Entry point that optionally resumes a saved session (log handle +
/// replayed transcript from `SessionLog::open`).
pub fn run_with(resume: Option<(SessionLog, Vec<moneyball_core::agent::Item>)>) -> Result<()> {
    run_with_cfg(resume, None)
}

pub fn run_with_cfg(
    resume: Option<(SessionLog, Vec<moneyball_core::agent::Item>)>,
    cfg_override: Option<AppConfig>,
) -> Result<()> {
    let cfg = match cfg_override {
        Some(c) => c,
        None => AppConfig::resolve_optional(None, None),
    };
    let mut app = App::new(cfg);
    match resume {
        Some((log, items)) => {
            app.replay(&items);
            app.session = Session::new(Some(log), items);
        }
        // Fresh session: appended items persist from the first turn.
        // A create failure degrades to an unpersisted session with a
        // visible warning, never a dead REPL.
        None => match SessionLog::create(app.cfg.data_root.clone(), app.cfg.sessions_root()) {
            Ok(log) => app.session = Session::new(Some(log), Vec::new()),
            Err(e) => app.status = Some(format!("session log unavailable: {}", e)),
        },
    }
    if matches!(app.view, View::Brief) {
        app.load_brief();
        app.push_landing();
    }
    let mut terminal = init()?;
    let res = event_loop(&mut terminal, &mut app);
    restore()?;
    res
}
