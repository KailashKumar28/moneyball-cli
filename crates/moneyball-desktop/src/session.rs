//! The advisor's sessions: the list, the one on screen as a snapshot a
//! reload can re-read, opening a saved one and starting a new one. A
//! session's file is created by its first turn, so opening the app or
//! pressing New leaves no empty file behind.

use std::sync::{Arc, Mutex};

use moneyball_core::agent::{view_items, Session, ViewItem};
use moneyball_core::session::{self, SessionSummary};
use moneyball_core::{AppConfig, SessionLog};
use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::state::{lock, AppState, SharedSession};

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    /// None until the session's first turn creates its file.
    pub id: Option<String>,
    pub items: Vec<ViewItem>,
    /// True after a reload mid-turn: show "working" and ask again.
    pub turn_active: bool,
}

pub fn snapshot(session: Option<&SharedSession>) -> SessionSnapshot {
    let Some(session) = session else {
        return SessionSnapshot {
            id: None,
            items: Vec::new(),
            turn_active: false,
        };
    };
    let s = lock(session);
    SessionSnapshot {
        id: s.id().map(String::from),
        items: view_items(s.history()),
        turn_active: s.turn_active(),
    }
}

/// The session on screen, created (with its file) when there is none.
pub fn current_or_new(state: &AppState, cfg: &AppConfig) -> SharedSession {
    if let Some(session) = state.session() {
        return session;
    }
    // Without a file the chat still works; it is just not saved.
    let log = SessionLog::create(cfg.data_root.clone(), cfg.sessions_root()).ok();
    let session = Arc::new(Mutex::new(Session::new(log, Vec::new())));
    state.set_session(Some(session.clone()));
    session
}

/// Leaving a session mid-turn would orphan its stream.
fn idle(state: &AppState) -> Result<(), AppError> {
    match state.session() {
        Some(s) if lock(&s).turn_active() => Err(AppError::busy(
            "The advisor is still answering. Wait for it to finish, or stop it first.",
        )),
        _ => Ok(()),
    }
}

/// Open a saved session by id. The id names a file, so only the
/// characters a session id is made of are let through.
pub fn open(state: &AppState, cfg: &AppConfig, id: &str) -> Result<SessionSnapshot, AppError> {
    idle(state)?;
    let named = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let opened = named
        .then(|| SessionLog::open(id, cfg.sessions_root()).ok())
        .flatten();
    let Some((log, items)) = opened else {
        return Err(AppError::new(
            "session",
            "That session could not be opened.",
        ));
    };
    let session = Arc::new(Mutex::new(Session::new(Some(log), items)));
    state.set_session(Some(session.clone()));
    Ok(snapshot(Some(&session)))
}

#[tauri::command(async)]
pub fn sessions_list(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, AppError> {
    let cfg = state.workspace()?;
    // No sessions folder yet reads as no sessions.
    Ok(session::summaries(cfg.sessions_root()).unwrap_or_default())
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
    idle(&state)?;
    state.set_session(None);
    Ok(snapshot(None))
}
