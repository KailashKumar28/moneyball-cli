//! The workspace the app has open: the one remembered from last time,
//! or the folder the user picks. No setup here (v1): a folder without a
//! workspace config is an error that names the terminal.

use std::path::Path;

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

/// The remembered workspace, opened; None on a first run.
#[tauri::command(async)]
pub fn workspace_current(state: State<'_, AppState>) -> Result<Option<Workspace>, AppError> {
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
    // Its events go to the screen of the workspace that started it.
    if state.refresh_running() {
        return Err(AppError::busy(
            "A refresh is running. Wait for it to finish, or cancel it, before opening another workspace.",
        ));
    }
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let dir = picked
        .into_path()
        .map_err(|e| AppError::new("io", format!("That folder cannot be opened: {}", e)))?;
    let shown = open(&state, &dir)?;
    // The workspace is open either way; only next launch forgets it.
    workspace::remember(&dir)?;
    Ok(Some(shown))
}
