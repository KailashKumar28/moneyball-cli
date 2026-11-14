//! Onboarding a client (plans/desktop/onboarding-plan.md), as plain
//! functions both front-ends call: connect Meta once per Mac, make the
//! client from its ad accounts, then a checklist of what is left. Where
//! each step stands is derived from what is on disk at every call, never
//! stored (codex's onboarding), so leaving and coming back resumes.

mod accounts;
mod checklist;
pub mod clients;
mod create;

pub use accounts::{accounts, AccountChoice};
pub use checklist::{checklist, Check, Item};
pub use create::create_client;

use std::path::Path;

use serde::Serialize;

use crate::config::WorkspaceConfig;

/// Where a step stands (codex's `StepState`): Hidden while a step before
/// it is not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Hidden,
    InProgress,
    Complete,
}

/// The two steps a client needs (D4): Meta connected, then the client
/// made with its products.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Steps {
    pub meta: StepState,
    pub client: StepState,
}

/// The steps for the client in `client` (None: no client yet), from the
/// stored Meta token and the client's config as they are now.
pub fn steps(client: Option<&Path>) -> Steps {
    derive(
        crate::secrets::load_meta_token().is_some(),
        client.is_some_and(has_products),
    )
}

fn has_products(dir: &Path) -> bool {
    WorkspaceConfig::load(dir).is_ok_and(|w| !w.products.is_empty())
}

fn derive(token: bool, client: bool) -> Steps {
    use StepState::*;
    Steps {
        meta: if token { Complete } else { InProgress },
        // The client is made from the accounts Meta lists: it waits on
        // Meta, unless it was made already (a token cleared since).
        client: match (client, token) {
            (true, _) => Complete,
            (false, true) => InProgress,
            (false, false) => Hidden,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StepState::*;

    #[test]
    fn steps_follow_the_token_and_the_client() {
        let s = |meta, client| Steps { meta, client };
        assert_eq!(derive(false, false), s(InProgress, Hidden), "no token");
        assert_eq!(
            derive(true, false),
            s(Complete, InProgress),
            "token, no client"
        );
        assert_eq!(derive(true, true), s(Complete, Complete), "both");
        assert_eq!(
            derive(false, true),
            s(InProgress, Complete),
            "token cleared"
        );
    }

    #[test]
    fn a_client_counts_once_its_config_has_products() {
        let dir = std::env::temp_dir().join(format!("mb-steps-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        assert!(!has_products(&dir), "no folder");
        crate::config::update(&dir, |_| ()).unwrap();
        assert!(!has_products(&dir), "no products");
        crate::config::update(&dir, |w| {
            w.products = vec![crate::config::Product {
                name: "P".into(),
                ad_account: "111111".into(),
            }]
        })
        .unwrap();
        assert!(has_products(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }
}
