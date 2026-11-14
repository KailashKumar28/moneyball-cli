//! The workspace the app has open: the one remembered from last time,
//! or the folder the user picks. A folder without a workspace config is
//! an error; a new client is made in `onboarding`.

use std::path::{Path, PathBuf};

use moneyball_core::onboarding::clients;
use moneyball_core::{text, workspace, AppConfig};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Workspace {
    /// The folder's name.
    pub name: String,
    /// Absolute.
    pub path: String,
    /// "~/DEV/moneyball-test".
    pub display_path: String,
    /// Configured product names, in config order.
    pub products: Vec<String>,
}

pub fn view(cfg: &AppConfig) -> Workspace {
    let root = &cfg.data_root;
    let products = cfg.workspace.as_ref().map(|w| w.product_names());
    Workspace {
        name: root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into(),
        ),
        path: root.display().to_string(),
        display_path: text::tilde(root),
        products: products
            .unwrap_or_default()
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

/// Open the workspace at `dir` and make it the app's.
pub fn open(state: &AppState, dir: &Path) -> Result<Workspace, AppError> {
    let cfg = workspace::open(dir, None)?;
    let shown = view(&cfg);
    state.open(cfg);
    Ok(shown)
}

/// Open the folder the user picked and remember it for the next launch.
/// A failed `remember` never fails the pick: by then the workspace is
/// open, and a rejection would leave the interface on the old workspace
/// while every command answers for this one. Only the next launch
/// forgets it.
pub fn pick(
    state: &AppState,
    dir: &Path,
    remember: impl FnOnce(&Path) -> moneyball_core::Result<()>,
) -> Result<Workspace, AppError> {
    let shown = open(state, dir)?;
    let _ = remember(dir);
    Ok(shown)
}

/// Remember `dir` for the next launch and keep it on the client list;
/// both are tried, whichever fails.
pub fn keep(dir: &Path) -> moneyball_core::Result<()> {
    workspace::remember(dir).and(clients::add(dir))
}

/// Refused while a refresh runs: its events go to the screen of the
/// workspace that started it.
pub fn not_while_refreshing(state: &AppState, doing: &str) -> Result<(), AppError> {
    match state.refresh_running() {
        true => Err(AppError::busy(&format!(
            "A refresh is running. Wait for it to finish, or cancel it, before {}.",
            doing
        ))),
        false => Ok(()),
    }
}

/// The native folder dialog; None when cancelled.
pub fn pick_folder(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    picked
        .into_path()
        .map(Some)
        .map_err(|e| AppError::new("io", format!("That folder cannot be opened: {}", e)))
}

/// The open workspace as it reads on disk now (its name and products
/// may have changed in the terminal); before one is open, the
/// remembered one, opened. None on a first run.
#[tauri::command(async)]
pub fn workspace_current(state: State<'_, AppState>) -> Result<Option<Workspace>, AppError> {
    if let Ok(cfg) = state.workspace() {
        return Ok(Some(view(&cfg)));
    }
    match workspace::last() {
        Some(dir) => open(&state, &dir).map(Some),
        None => Ok(None),
    }
}

/// The native folder dialog; None when cancelled.
#[tauri::command(async)]
pub fn workspace_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Workspace>, AppError> {
    not_while_refreshing(&state, "opening another workspace")?;
    let Some(dir) = pick_folder(&app)? else {
        return Ok(None);
    };
    pick(&state, &dir, keep).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_that_cannot_be_remembered_still_opens_the_workspace() {
        let root = std::env::temp_dir().join(format!("mb-desktop-pick-{}", std::process::id()));
        let workspace = |name: &str| {
            let dir = root.join(name);
            std::fs::create_dir_all(dir.join(".moneyball")).unwrap();
            let config = r#"{"products": [{"name": "P", "ad_account": "act_1"}]}"#;
            std::fs::write(dir.join(".moneyball/config.json"), config).unwrap();
            dir
        };
        let (a, b) = (workspace("a"), workspace("b"));
        let state = AppState::default();
        pick(&state, &a, |_| Ok(())).unwrap();
        let forgets = |_: &Path| {
            Err(moneyball_core::Error::Config(
                "config.json: not json".into(),
            ))
        };
        let shown = pick(&state, &b, forgets).unwrap();
        assert_eq!(shown.name, "b");
        assert_eq!(view(&state.workspace().unwrap()), shown);
        // A folder that is no workspace is still refused, and B stays open.
        let err = pick(&state, &root, |_| Ok(())).unwrap_err();
        assert_eq!(err.kind, "no_workspace_config");
        assert_eq!(
            state.workspace().unwrap().data_root,
            b.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
