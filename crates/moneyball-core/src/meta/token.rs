//! A Meta token in words (onboarding Step 1): what Meta's `debug_token`
//! says about the token "Continue with Facebook" brought back. No network
//! here; `meta::check_token` makes the calls.

use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::Value;

use super::connect::SCOPES;
use crate::error::{Error, Result};

/// A token Meta accepted, as the Connect Meta page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenCheck {
    /// The Meta user or system user the token belongs to.
    pub name: String,
    /// Ad accounts it can see.
    pub accounts: usize,
    /// "Never expires" or "Expires Mon 12 Jan".
    pub expiry: String,
    /// ads_read and no scope that can change anything.
    pub read_only: bool,
    /// Shown, never blocking: an expiry date, a write scope Moneyball did
    /// not ask for, no ads_read.
    pub warnings: Vec<String>,
}

impl TokenCheck {
    /// From `/me`'s name, the number of ad accounts and `debug_token`'s
    /// `data`. A token `debug_token` calls invalid is refused as a 190 is.
    pub fn read(name: &str, accounts: usize, debug: &Value) -> Result<TokenCheck> {
        if debug["is_valid"] == Value::Bool(false) {
            return Err(Error::MetaAuth(
                "Meta says this connection is not valid: it expired or was revoked. \
                 Connect Meta again."
                    .into(),
            ));
        }
        let scopes: Vec<&str> = debug["scopes"]
            .as_array()
            .map(|all| all.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // ads_management, business_management, pages_manage_*, *_publish...
        let writes = |s: &&str| s.contains("manage") || s.contains("publish");
        let unasked: Vec<&str> = scopes
            .iter()
            .copied()
            .filter(|s| writes(s) && !SCOPES.contains(s))
            .collect();
        let reads = scopes.contains(&"ads_read");
        let mut warnings = Vec::new();
        // Meta's 0 is a token that never expires.
        let expires = debug["expires_at"].as_i64().filter(|t| *t > 0);
        let expiry = match expires.and_then(|t| Local.timestamp_opt(t, 0).single()) {
            None => "Never expires".to_string(),
            Some(at) => {
                let day = at.format("%a %d %b");
                warnings.push(format!(
                    "Meta ends this connection on {}, and refreshes stop then. Connect \
                     Meta again before that day.",
                    day
                ));
                format!("Expires {}", day)
            }
        };
        if !unasked.is_empty() {
            warnings.push(format!(
                "This connection can also change ads or settings ({}), which Moneyball \
                 never asks for or does.",
                unasked.join(", ")
            ));
        }
        if !reads && !scopes.contains(&"ads_management") {
            warnings.push(
                "This connection has no ads_read permission, so refreshes cannot read \
                 the ads. Connect Meta again and allow Moneyball to read them."
                    .into(),
            );
        }
        Ok(TokenCheck {
            name: name.to_string(),
            accounts,
            expiry,
            read_only: reads && !scopes.iter().any(writes),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_read_only_token_that_never_expires_has_no_warnings() {
        let debug =
            json!({"is_valid": true, "expires_at": 0, "scopes": ["ads_read", "public_profile"]});
        let got = TokenCheck::read("Acme System User", 26, &debug).unwrap();
        assert_eq!(
            got,
            TokenCheck {
                name: "Acme System User".into(),
                accounts: 26,
                expiry: "Never expires".into(),
                read_only: true,
                warnings: vec![],
            }
        );
    }

    #[test]
    fn what_moneyball_asks_for_is_not_warned_about_nor_called_read_only() {
        let debug = json!({"is_valid": true, "expires_at": 0, "scopes": SCOPES});
        let got = TokenCheck::read("Acme", 2, &debug).unwrap();
        assert!(got.warnings.is_empty(), "{:?}", got.warnings);
        assert!(
            !got.read_only,
            "pages_manage_ads and business_management can write"
        );
    }

    #[test]
    fn an_expiry_date_and_write_scopes_moneyball_did_not_ask_for_are_warned_about() {
        // Noon UTC: the same day in every zone from -11 to +11.
        let monday = 1799668800; // 2027-01-11T12:00:00Z
        let debug = json!({"is_valid": true, "expires_at": monday,
            "scopes": ["ads_read", "ads_management", "pages_manage_ads", "pages_manage_posts"]});
        let got = TokenCheck::read("Kailash", 3, &debug).unwrap();
        assert_eq!(got.expiry, "Expires Mon 11 Jan");
        assert!(!got.read_only);
        assert_eq!(got.warnings.len(), 2, "{:?}", got.warnings);
        assert!(got.warnings[0].contains("on Mon 11 Jan"));
        // ads_management and pages_manage_ads are Moneyball's own: full
        // lead data needs them. Only the one it never asked for is warned.
        assert!(
            got.warnings[1].contains("(pages_manage_posts)"),
            "{:?}",
            got.warnings
        );
    }

    #[test]
    fn no_ads_read_is_warned_and_an_invalid_token_refused() {
        let debug = json!({"is_valid": true, "expires_at": 0, "scopes": ["public_profile"]});
        let got = TokenCheck::read("X", 0, &debug).unwrap();
        assert!(!got.read_only);
        assert!(
            got.warnings[0].contains("no ads_read"),
            "{:?}",
            got.warnings
        );
        let e = TokenCheck::read("X", 0, &json!({"is_valid": false})).unwrap_err();
        assert_eq!(e.kind(), "meta_auth");
    }
}
