//! The advisor's sessions: the list, the one on screen as a snapshot a
//! reload can re-read, opening a saved one and starting a new one. A
//! session's file is created by its first turn, so opening the app or
//! pressing New leaves no empty file behind.

use moneyball_core::agent::{view_items, Session, ViewItem};
use moneyball_core::session::{self, SessionSummary};
use moneyball_core::{AppConfig, SessionLog};
use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::state::{lock, AppState, Live, SharedSession};

/// This front-end's name in the header of a session file it writes.
/// The app lists and opens only these: a session the terminal app wrote
/// holds slash commands this app must never show or hand to the model,
/// and may be open there right now.
pub const FRONT_END: &str = "desktop";

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    /// None until the session's first turn creates its file.
    pub id: Option<String>,
    pub items: Vec<ViewItem>,
    /// True after a reload mid-turn: show "working" and ask again.
    pub turn_active: bool,
    /// How the last turn failed or died, until the next one starts; the
    /// session file has no item for it.
    pub failed: Option<String>,
}

pub fn snapshot(session: Option<&SharedSession>) -> SessionSnapshot {
    let Some(session) = session else {
        return SessionSnapshot {
            id: None,
            items: Vec::new(),
            turn_active: false,
            failed: None,
        };
    };
    let live = lock(session);
    SessionSnapshot {
        id: live.session.id().map(String::from),
        items: view_items(live.session.history()),
        turn_active: live.session.turn_active(),
        failed: live.failed.clone(),
    }
}

/// A session with its file, for a first turn about to start.
pub fn new_session(cfg: &AppConfig) -> SharedSession {
    // Without a file the chat still works; it is just not saved.
    let log = SessionLog::create(cfg.data_root.clone(), cfg.sessions_root(), FRONT_END).ok();
    Live::shared(Session::new(log, Vec::new()))
}

/// Put `next` on screen unless a turn is running: leaving a session
/// mid-turn would orphan its stream. Checked and swapped under the one
/// slot lock.
fn replace_if_idle(state: &AppState, next: Option<SharedSession>) -> Result<(), AppError> {
    let mut slot = state.session_slot();
    if slot.as_ref().is_some_and(|s| lock(s).session.turn_active()) {
        return Err(AppError::busy(
            "The advisor is still answering. Wait for it to finish, or stop it first.",
        ));
    }
    *slot = next;
    Ok(())
}

/// The sessions this app wrote, most recently used first.
pub fn list(cfg: &AppConfig) -> Vec<SessionSummary> {
    // No sessions folder yet reads as no sessions.
    let all = session::summaries(cfg.sessions_root()).unwrap_or_default();
    let ours = |s: &SessionSummary| s.front_end.as_deref() == Some(FRONT_END);
    all.into_iter().filter(ours).collect()
}

/// Open a saved session of this app's by id. The id names a file, so
/// only the characters a session id is made of are let through.
pub fn open(state: &AppState, cfg: &AppConfig, id: &str) -> Result<SessionSnapshot, AppError> {
    let named = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let opened = named
        .then(|| SessionLog::open(id, cfg.sessions_root()).ok())
        .flatten()
        .filter(|(log, _)| log.meta.front_end.as_deref() == Some(FRONT_END));
    let Some((log, items)) = opened else {
        return Err(AppError::new(
            "session",
            "That session could not be opened.",
        ));
    };
    let session = Live::shared(Session::new(Some(log), items));
    replace_if_idle(state, Some(session.clone()))?;
    Ok(snapshot(Some(&session)))
}

#[tauri::command(async)]
pub fn sessions_list(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, AppError> {
    Ok(list(&state.workspace()?))
}

#[tauri::command]
pub fn session_current(state: State<'_, AppState>) -> SessionSnapshot {
    snapshot(state.session().as_ref())
}

#[tauri::command(async)]
pub fn session_open(state: State<'_, AppState>, id: String) -> Result<SessionSnapshot, AppError> {
    open(&state, &state.workspace()?, &id)
}

#[tauri::command]
pub fn session_new(state: State<'_, AppState>) -> Result<SessionSnapshot, AppError> {
    replace_if_idle(&state, None)?;
    Ok(snapshot(None))
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
