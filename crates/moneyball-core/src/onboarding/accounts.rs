//! The ad accounts a client can be made from (plan Step 2): what Meta
//! lists for the token, each with its state in a word and the clients
//! already using it.

use serde::Serialize;

use super::clients;
use crate::config::AppConfig;
use crate::error::Result;
use crate::meta::{self, AdAccount};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountChoice {
    /// As a product stores it: Meta's id without "act_".
    pub id: String,
    pub name: String,
    /// Meta's `account_status` in a word: "active", "disabled", ...
    pub status: &'static str,
    /// "used by Fincity" when a known client's products already use it.
    pub used_by: Option<String>,
}

/// Every ad account the token can see, for the account picker. `known`
/// are the other clients (`clients::list()` without the one being set up).
pub fn accounts(token: &str, known: &[AppConfig]) -> Result<Vec<AccountChoice>> {
    Ok(choices(meta::list_ad_accounts(token)?, known))
}

fn choices(accounts: Vec<AdAccount>, known: &[AppConfig]) -> Vec<AccountChoice> {
    accounts
        .into_iter()
        .map(|a| {
            let id = meta::account_id_for_storage(&a.id);
            let users: Vec<String> = known
                .iter()
                .filter(|c| {
                    c.workspace.as_ref().is_some_and(|w| {
                        w.products
                            .iter()
                            .any(|p| meta::account_id_for_storage(&p.ad_account) == id)
                    })
                })
                .map(clients::name)
                .collect();
            AccountChoice {
                status: status_word(a.account_status),
                used_by: (!users.is_empty()).then(|| format!("used by {}", users.join(", "))),
                id,
                name: a.name,
            }
        })
        .collect()
}

/// Meta's AdAccount `account_status` codes.
fn status_word(code: Option<u32>) -> &'static str {
    match code {
        Some(1) => "active",
        Some(2) => "disabled",
        Some(3) => "unsettled",
        Some(7) => "pending risk review",
        Some(8) => "pending settlement",
        Some(9) => "in grace period",
        Some(100) => "pending closure",
        Some(101) => "closed",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Product, WorkspaceConfig};

    fn account(id: &str, status: Option<u32>) -> AdAccount {
        AdAccount {
            id: id.into(),
            name: format!("Account {}", id),
            account_status: status,
        }
    }

    fn client(folder: &str, brand: Option<&str>, accounts: &[&str]) -> AppConfig {
        AppConfig {
            data_root: format!("/clients/{}", folder).into(),
            date: None,
            workspace: Some(WorkspaceConfig {
                brand: brand.map(String::from),
                products: accounts
                    .iter()
                    .map(|a| Product {
                        name: format!("P{}", a),
                        ad_account: a.to_string(),
                    })
                    .collect(),
                ..Default::default()
            }),
            agent: false,
        }
    }

    #[test]
    fn every_status_has_its_word() {
        let codes = [
            (Some(1), "active"),
            (Some(2), "disabled"),
            (Some(3), "unsettled"),
            (Some(7), "pending risk review"),
            (Some(8), "pending settlement"),
            (Some(9), "in grace period"),
            (Some(100), "pending closure"),
            (Some(101), "closed"),
            (Some(4), "unknown"),
            (None, "unknown"),
        ];
        for (code, word) in codes {
            assert_eq!(status_word(code), word, "{:?}", code);
        }
    }

    #[test]
    fn an_account_a_client_uses_says_which() {
        let known = [
            client("fincity", Some("Fincity"), &["111", "act_222"]),
            client("disha", None, &["222"]),
        ];
        let got = choices(
            vec![
                account("act_111", Some(1)),
                account("act_222", Some(2)),
                account("act_333", Some(101)),
            ],
            &known,
        );
        assert_eq!(
            got[0],
            AccountChoice {
                id: "111".into(),
                name: "Account act_111".into(),
                status: "active",
                used_by: Some("used by Fincity".into()),
            }
        );
        assert_eq!(got[1].status, "disabled");
        assert_eq!(got[1].used_by.as_deref(), Some("used by Fincity, disha"));
        assert_eq!((got[2].status, got[2].used_by.as_deref()), ("closed", None));
    }
}
