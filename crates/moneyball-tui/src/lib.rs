//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

mod app;
pub mod chat;
mod commands;
mod event;
pub(crate) mod markdown;
mod render;
mod setup;
pub mod widgets;
pub(crate) use app::snapshot_load;
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

use moneyball_core::session::SessionLog;
use moneyball_core::AppConfig;

// Two chrono types collide on import name; alias the plain Utc.

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> Result<Tui> {
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
    disable_raw_mode()?;
    // Best-effort: release the mouse even if leaving the alt screen fails.
    let _ = execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
    execute!(io::stdout(), LeaveAlternateScreen)?;
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
            app.session = Some(log);
            app.replay(items);
        }
        // Fresh session: appended items persist from the first turn.
        // A create failure degrades to an unpersisted session with a
        // visible warning, never a dead REPL.
        None => match SessionLog::create(app.cfg.data_root.clone()) {
            Ok(log) => app.session = Some(log),
            Err(e) => app.status = Some(format!("session log unavailable: {}", e)),
        },
    }
    if matches!(app.view, View::Brief) {
        app.load_brief();
    }
    let mut terminal = init()?;
    let res = event_loop(&mut terminal, &mut app);
    restore()?;
    res
}
