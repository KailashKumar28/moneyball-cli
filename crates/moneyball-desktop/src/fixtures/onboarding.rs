//! The onboarding shapes. Meta's answers and the home folder are built
//! by hand and go through the same functions the commands call, so the
//! words are core's and nothing reaches the network or the real home.

use std::path::Path;

use moneyball_core::config::{Product, WorkspaceConfig};
use moneyball_core::onboarding::{self, StepState};
use moneyball_core::{config, meta, AdAccount, AppConfig, Error, TokenCheck};
use serde_json::{json, Value};

use super::Pins;
use crate::onboarding::{
    account_choices, create, folder, in_sheet, not_connected, OnboardingState,
};

/// Where the fixtures say the home folder is.
const SHOWN_HOME: &str = "/Users/demo";

/// A token Meta accepted, as `meta_connect` returns it (core's
/// `check_token` reads `debug_token` so): with what Moneyball asks for,
/// and one that expires and can also write.
fn connected(scopes: Value, expires_at: i64) -> TokenCheck {
    let debug = json!({"is_valid": true, "expires_at": expires_at, "scopes": scopes});
    TokenCheck::read("Acme Homes System User", 5, &debug).unwrap()
}

/// The accounts the demo's token sees, beside a known client (Fincity)
/// that already uses one of them.
fn accounts() -> Value {
    let account = |id: &str, name: &str, status: u32| AdAccount {
        id: format!("act_{}", id),
        name: name.into(),
        account_status: Some(status),
    };
    let fincity = AppConfig {
        data_root: "/Users/demo/Moneyball/Fincity".into(),
        date: None,
        workspace: Some(WorkspaceConfig {
            brand: Some("Fincity".into()),
            products: vec![Product {
                name: "Fincity Plots".into(),
                ad_account: "303".into(),
            }],
            ..Default::default()
        }),
        agent: false,
    };
    let listed = vec![
        account("101", "Acme Homes - Namma Mane", 1),
        account("202", "Acme Homes - Cityville", 1),
        account("303", "Fincity Plots", 1),
        account("404", "Acme Homes Archive", 2),
        account("505", "Lakeside Villas", 3),
    ];
    let all = onboarding::choices(listed, &[fincity]);
    serde_json::to_value(account_choices("Acme Homes", all)).unwrap()
}

pub fn shapes(pins: &mut Pins, cfg: &AppConfig) {
    let first = OnboardingState {
        meta: StepState::InProgress,
        clients: 0,
    };
    pins.add("onboarding_state", cfg, first);
    pins.add("meta_connected", cfg, connected(json!(meta::SCOPES), 0));
    // Noon UTC on Mon 11 Jan 2027: the same day in every zone the demo runs in.
    let writes = json!(["ads_read", "ads_management", "business_management"]);
    let warned = connected(writes, 1799668800);
    pins.add("meta_connected_warnings", cfg, warned);
    pins.add("client_accounts", cfg, accounts());
    let home = config::home_dir().unwrap_or_else(|| SHOWN_HOME.into());
    let at = folder(&home.join("Moneyball").join("Acme Homes"));
    pins.add_at("client_folder", &home, SHOWN_HOME, at);
}

/// The refusals the sheet shows, keyed as errors.json keys them.
pub fn errors(state: &crate::state::AppState) -> [(&'static str, Value); 3] {
    // Declined on Facebook: the broker's words (broker/src/index.js),
    // passed on as they come.
    let declined = "Facebook sign-in was cancelled, so nothing was connected.";
    let declined = in_sheet(Error::Authorization(declined.into()));
    // Refused before anything is written: no account was ticked.
    let make = || onboarding::create_client(None, "Acme Homes", vec![]);
    let none_ticked = create(state, make, |_: &Path| Ok(())).unwrap_err();
    [
        ("meta_declined", json!(declined)),
        ("meta_auth", json!(not_connected())),
        ("client_create", json!(none_ticked)),
    ]
}
