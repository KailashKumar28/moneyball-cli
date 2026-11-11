//! What the app holds between commands (`tauri::State`): the open
//! workspace, the advisor session and the refresh in flight. The webview
//! holds none of it, so a reload re-reads a snapshot of each.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use moneyball_core::agent::Session;
use moneyball_core::AppConfig;

use crate::error::AppError;
use crate::refresh::RefreshRun;

/// A session shared with the forwarder thread of its turn in flight.
pub type SharedSession = Arc<Mutex<Session>>;

/// Lock without panicking: a holder that panicked leaves plain data
/// behind, and a dead command must not take every later one with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
pub struct AppState {
    workspace: Mutex<Option<AppConfig>>,
    session: Mutex<Option<SharedSession>>,
    /// Shared with the refresh forwarder, which keeps it current.
    pub refresh: Arc<Mutex<RefreshRun>>,
}

impl AppState {
    /// The open workspace, or the error that says there is none.
    pub fn workspace(&self) -> Result<AppConfig, AppError> {
        lock(&self.workspace)
            .clone()
            .ok_or_else(AppError::no_workspace)
    }

    /// Make `cfg` the open workspace. The old session goes with the old
    /// workspace: a turn still running is stopped, and its forwarder
    /// (which holds the session) finishes writing its file.
    pub fn open(&self, cfg: AppConfig) {
        let mut workspace = lock(&self.workspace);
        let moved = workspace
            .as_ref()
            .is_none_or(|old| old.data_root != cfg.data_root);
        *workspace = Some(cfg);
        if moved {
            if let Some(old) = lock(&self.session).take() {
                lock(&old).interrupt();
            }
        }
    }

    pub fn session(&self) -> Option<SharedSession> {
        lock(&self.session).clone()
    }

    pub fn set_session(&self, session: Option<SharedSession>) {
        *lock(&self.session) = session;
    }

    pub fn refresh_running(&self) -> bool {
        lock(&self.refresh).running
    }
}
