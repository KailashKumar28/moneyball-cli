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
use app::{load_session_into, save_current_session};
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

use moneyball_core::session::Session;
use moneyball_core::AppConfig;

// Two chrono types collide on import name; alias the plain Utc.

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    // Mouse capture lets the wheel scroll the transcript instead of the
    // terminal window (text selection becomes Shift+drag - standard TUI
    // trade-off, same as codex).
    execute!(
        out,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    // Best-effort: release the mouse even if leaving the alt screen fails.
    let _ = execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

// ---------- slash-command surface ----------

// ---------- app state ----------

// ---------- main entry ----------

pub fn run() -> Result<()> {
    run_with(None)
}

/// Entry point that optionally pre-loads a saved session into the chat log.
pub fn run_with(resume_session: Option<Session>) -> Result<()> {
    run_with_cfg(resume_session, None)
}

pub fn run_with_cfg(
    resume_session: Option<Session>,
    cfg_override: Option<AppConfig>,
) -> Result<()> {
    let cfg = match cfg_override {
        Some(c) => c,
        None => AppConfig::resolve_optional(None, None),
    };
    let mut app = App::new(cfg);
    if let Some(s) = resume_session {
        load_session_into(&mut app, s);
    }
    if matches!(app.view, View::Brief) {
        app.load_brief();
    }
    let mut terminal = init()?;
    let res = event_loop(&mut terminal, &mut app);
    restore()?;
    // Auto-save the chat log so /quit or Ctrl-C still persists.
    if let Err(e) = save_current_session(&app) {
        eprintln!("warning: session save failed: {}", e);
    }
    res
}
