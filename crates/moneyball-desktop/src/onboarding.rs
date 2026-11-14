//! Onboarding a client (plans/desktop/onboarding-plan.md Steps 0 to 3):
//! where the first-launch screen stands, Connect Meta, and Add a Client.
//! Thin wrappers over core's `onboarding` and `meta::check_token`. The
//! sheet is where each of these errors is fixed, so none carries a fix.

use std::path::{Path, PathBuf};

use moneyball_core::config::Product;
use moneyball_core::onboarding::{self, clients, StepState};
use moneyball_core::{meta, secrets, text, AppConfig, TokenCheck};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::state::AppState;
use crate::workspace::{self, Workspace};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardingState {
    pub meta: StepState,
    /// Known clients whose folders still hold a workspace.
    pub clients: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Folder {
    pub path: String,
    /// "~/Moneyball/Fincity".
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountChoice {
    /// "act_123", as Meta names it.
    pub id: String,
    pub name: String,
    /// Core's word: "active", "disabled", ...
    pub status: &'static str,
    /// Core's words, "used by Fincity", when a known client uses it.
    pub used_by: Option<String>,
    /// Pre-ticked: the account's name contains the client's.
    pub suggested: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProduct {
    account_id: String,
    name: String,
}

/// A core error as the sheet shows it: core's sentence, and no terminal
/// fix, since the sheet itself is the fix.
fn in_sheet(e: moneyball_core::Error) -> AppError {
    AppError {
        fix: None,
        ..e.into()
    }
}

pub fn folder(path: &Path) -> Folder {
    Folder {
        path: path.display().to_string(),
        display: text::tilde(path),
    }
}

/// Check `token` with Meta, and store it only once Meta accepts it.
pub fn connect(
    token: &str,
    check: impl FnOnce(&str) -> moneyball_core::Result<TokenCheck>,
    store: impl FnOnce(&str) -> moneyball_core::Result<()>,
) -> Result<TokenCheck, AppError> {
    let token = token.trim();
    let checked = check(token).map_err(in_sheet)?;
    store(token).map_err(in_sheet)?;
    Ok(checked)
}

/// The picker's rows: core's choices under Meta's own ids, the accounts
/// named like the client ticked.
pub fn account_choices(name: &str, all: Vec<onboarding::AccountChoice>) -> Vec<AccountChoice> {
    let name = name.trim().to_lowercase();
    all.into_iter()
        .map(|a| AccountChoice {
            suggested: !name.is_empty() && a.name.to_lowercase().contains(&name),
            id: format!("act_{}", a.id),
            name: a.name,
            status: a.status,
            used_by: a.used_by,
        })
        .collect()
}

/// Asked for accounts with no token stored: the sheet goes back to
/// Connect Meta, as for a token Meta stopped accepting.
pub fn not_connected() -> AppError {
    AppError::new(
        "meta_auth",
        "Meta is not connected on this Mac yet: paste the system user's token first.",
    )
}

/// Make the client and open it as a picked folder is opened. Refused
/// while a refresh runs, before anything is written.
pub fn create(
    state: &AppState,
    make: impl FnOnce() -> moneyball_core::Result<AppConfig>,
    remember: impl FnOnce(&Path) -> moneyball_core::Result<()>,
) -> Result<Workspace, AppError> {
    workspace::not_while_refreshing(state, "adding a client")?;
    let cfg = make().map_err(in_sheet)?;
    workspace::pick(state, &cfg.data_root, remember)
}

/// From disk only: is a token stored, how many clients are known.
#[tauri::command(async)]
pub fn onboarding_state() -> OnboardingState {
    OnboardingState {
        meta: onboarding::steps(None).meta,
        clients: clients::list().len(),
    }
}

/// Core's Meta calls block on a client of their own, which panics when it
/// is dropped on one of the async runtime's threads (where an `async`
/// command runs): a command that calls Meta runs them on a blocking one.
async fn calling_meta<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| {
            AppError::new(
                "io",
                format!("The call to Meta stopped unexpectedly: {}", e),
            )
        })?
}

#[tauri::command]
pub async fn meta_connect(token: String) -> Result<TokenCheck, AppError> {
    calling_meta(move || connect(&token, meta::check_token, secrets::store_meta_token)).await
}

/// Where `client_create` puts `name` unless the user picks a folder.
#[tauri::command]
pub fn client_folder(name: String) -> Result<Folder, AppError> {
    let dir = onboarding::default_folder(&name)
        .ok_or_else(|| AppError::new("io", "This Mac has no home folder to put the client in."))?;
    Ok(folder(&dir))
}

#[tauri::command(async)]
pub fn client_folder_pick(app: AppHandle) -> Result<Option<Folder>, AppError> {
    Ok(workspace::pick_folder(&app)?.map(|dir| folder(&dir)))
}

#[tauri::command]
pub async fn client_accounts(name: String) -> Result<Vec<AccountChoice>, AppError> {
    calling_meta(move || {
        let token = secrets::load_meta_token().ok_or_else(not_connected)?;
        let all = onboarding::accounts(&token, &clients::list()).map_err(in_sheet)?;
        Ok(account_choices(&name, all))
    })
    .await
}

#[tauri::command(async)]
pub fn client_create(
    state: State<'_, AppState>,
    name: String,
    folder: Option<String>,
    products: Vec<NewProduct>,
) -> Result<Workspace, AppError> {
    let products = products
        .into_iter()
        .map(|p| Product {
            name: p.name,
            ad_account: p.account_id,
        })
        .collect();
    let folder = folder.map(PathBuf::from);
    let make = || onboarding::create_client(folder.as_deref(), &name, products);
    create(&state, make, workspace::keep)
}

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
