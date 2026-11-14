//! A Meta token in words (onboarding Step 1): its shape, checked before
//! Meta is asked, and what Meta's `debug_token` says about it. No network
//! here; `meta::check_token` makes the calls.

use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::Value;

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
    /// Shown, never blocking: an expiry date, a write scope, no ads_read.
    pub warnings: Vec<String>,
}

/// Refuse what is plainly not a token: an app id (digits only) or an app
/// secret (32 hex characters) sit beside the token on Meta's pages and
/// get pasted by mistake.
pub(super) fn check_shape(token: &str) -> Result<()> {
    let refuse = |s: &str| Err(Error::MetaTokenShape(s.into()));
    if token.is_empty() {
        return refuse("Paste the system user's token first.");
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return refuse(
            "That looks like an app id (only digits), not a token. Paste the token \
             generated for the system user: it is much longer and starts with EAA.",
        );
    }
    if token.len() == 32 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        return refuse(
            "That looks like an app secret (32 letters and digits), not a token. Paste \
             the token generated for the system user: it is much longer and starts with EAA.",
        );
    }
    Ok(())
}

impl TokenCheck {
    /// From `/me`'s name, the number of ad accounts and `debug_token`'s
    /// `data`. A token `debug_token` calls invalid is refused as a 190 is.
    pub fn read(name: &str, accounts: usize, debug: &Value) -> Result<TokenCheck> {
        if debug["is_valid"] == Value::Bool(false) {
            return Err(Error::MetaAuth(
                "Meta says this token is not valid: it expired or was revoked. \
                 Generate a new one for the system user in Business Settings."
                    .into(),
            ));
        }
        let scopes: Vec<&str> = debug["scopes"]
            .as_array()
            .map(|all| all.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // ads_management, business_management, pages_manage_*, *_publish...
        let writes: Vec<&str> = scopes
            .iter()
            .copied()
            .filter(|s| s.contains("manage") || s.contains("publish"))
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
                    "This token expires on {}, and refreshes stop then. Generate one \
                     with the expiry set to Never.",
                    day
                ));
                format!("Expires {}", day)
            }
        };
        if !writes.is_empty() {
            warnings.push(format!(
                "This token can change ads or settings ({}). Moneyball only reads: \
                 a token with ads_read alone is enough.",
                writes.join(", ")
            ));
        }
        if !reads && !scopes.contains(&"ads_management") {
            warnings.push(
                "This token has no ads_read permission, so refreshes cannot read the ads. \
                 Generate it again with ads_read ticked."
                    .into(),
            );
        }
        Ok(TokenCheck {
            name: name.to_string(),
            accounts,
            expiry,
            read_only: reads && writes.is_empty(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_app_id_or_secret_is_refused_by_its_shape() {
        for (pasted, says) in [
            ("", "Paste"),
            ("1234567890123456", "app id"),
            ("0123456789abcdef0123456789ABCDEF", "app secret"),
        ] {
            let e = check_shape(pasted).unwrap_err();
            assert_eq!(e.kind(), "meta_token_shape");
            assert!(e.reason().contains(says), "{}", e.reason());
        }
        check_shape("EAAB1234abcd").unwrap();
        // 32 characters that are not all hex is not a secret.
        check_shape("EAAZ0123456789abcdef0123456789ab").unwrap();
    }

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
    fn an_expiry_date_and_write_scopes_are_warned_about() {
        // Noon UTC: the same day in every zone from -11 to +11.
        let monday = 1799668800; // 2027-01-11T12:00:00Z
        let debug = json!({"is_valid": true, "expires_at": monday,
            "scopes": ["ads_read", "ads_management", "pages_manage_ads"]});
        let got = TokenCheck::read("Kailash", 3, &debug).unwrap();
        assert_eq!(got.expiry, "Expires Mon 11 Jan");
        assert!(!got.read_only);
        assert_eq!(got.warnings.len(), 2, "{:?}", got.warnings);
        assert!(got.warnings[0].contains("expires on Mon 11 Jan"));
        assert!(got.warnings[1].contains("(ads_management, pages_manage_ads)"));
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
