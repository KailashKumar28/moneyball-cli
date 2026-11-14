//! What the app holds between commands (`tauri::State`): the open
//! workspace, the advisor session and the refresh in flight. The webview
//! holds none of it, so a reload re-reads a snapshot of each.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use moneyball_core::agent::Session;
use moneyball_core::{workspace, AppConfig};

use crate::error::AppError;
use crate::refresh::RefreshRun;

/// The session on screen, and what its last turn left that its file
/// does not hold.
pub struct Live {
    pub session: Session,
    /// How the last turn failed or died, until the next one starts: a
    /// window reloaded mid-turn reads it from the snapshot.
    pub failed: Option<String>,
}

impl Live {
    pub fn shared(session: Session) -> SharedSession {
        Arc::new(Mutex::new(Live {
            session,
            failed: None,
        }))
    }
}

/// A session shared with the forwarder thread of its turn in flight.
pub type SharedSession = Arc<Mutex<Live>>;

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
    /// The cancel flag of the latest Continue with Facebook.
    pub connecting: Mutex<Arc<AtomicBool>>,
}

impl AppState {
    /// The open workspace as its config reads on disk now, or the error
    /// that says none is open. Read on every call (one small file): setup
    /// runs in the terminal, and what it fixes must hold here without a
    /// relaunch. A config that will not read just now leaves the last one
    /// that did.
    pub fn workspace(&self) -> Result<AppConfig, AppError> {
        let mut slot = lock(&self.workspace);
        let cfg = slot.as_mut().ok_or_else(AppError::no_workspace)?;
        if let Ok(fresh) = workspace::open(&cfg.data_root, cfg.date.as_deref()) {
            *cfg = fresh;
        }
        Ok(cfg.clone())
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
                lock(&old).session.interrupt();
            }
        }
    }

    pub fn session(&self) -> Option<SharedSession> {
        lock(&self.session).clone()
    }

    /// The session slot, held: whoever reads it and then fills or
    /// replaces it does both under this one lock, so two commands landing
    /// together cannot each make a session or swap one out mid-start.
    /// Lock order: this slot, then the session in it.
    pub fn session_slot(&self) -> MutexGuard<'_, Option<SharedSession>> {
        lock(&self.session)
    }

    pub fn refresh_running(&self) -> bool {
        lock(&self.refresh).running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{"products": [{"name": "P", "ad_account": "act_1"}]}"#;

    /// The fix the app itself hands out ("In Terminal: moneyball, then its
    /// setup") must take effect without a relaunch.
    #[test]
    fn the_workspace_is_read_again_on_every_call_and_survives_a_bad_read() {
        let root = std::env::temp_dir().join(format!("mb-desktop-state-{}", std::process::id()));
        let config = root.join(".moneyball/config.json");
        std::fs::create_dir_all(root.join(".moneyball")).unwrap();
        std::fs::write(&config, CONFIG).unwrap();
        let state = AppState::default();
        assert_eq!(state.workspace().unwrap_err().kind, "no_workspace");
        state.open(workspace::open(&root, None).unwrap());
        let names = || {
            let cfg = state.workspace().unwrap();
            let w = cfg.workspace.unwrap();
            let products: Vec<String> = w.product_names().into_iter().map(String::from).collect();
            (products, w.model)
        };
        assert_eq!(names(), (vec!["P".to_string()], None));
        // Setup in the terminal adds a product and a model.
        let added = CONFIG.replace(
            "]}",
            r#", {"name": "Q", "ad_account": "act_2"}], "model": "m-1"}"#,
        );
        std::fs::write(&config, added).unwrap();
        let after = (
            vec!["P".to_string(), "Q".to_string()],
            Some("m-1".to_string()),
        );
        assert_eq!(names(), after);
        // Half-written: the last good read stands.
        std::fs::write(&config, "{ not json").unwrap();
        assert_eq!(names(), after);
        std::fs::remove_dir_all(&root).ok();
    }
}
