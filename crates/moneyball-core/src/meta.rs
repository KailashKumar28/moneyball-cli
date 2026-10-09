//! Meta Graph API client (read-only). Used by the wizard to discover
//! the user's ad accounts when they paste an access token.
//!
//! Per the project hard contract: moneyball-core NEVER writes to Meta.
//! This module is read-only and only ever calls graph.facebook.com.

use serde::Deserialize;

use crate::error::{Error, Result};

const META_GRAPH_BASE: &str = "https://graph.facebook.com";
// NOTE: do NOT pin a version segment (e.g. /v18.0/) - calls should use the
// version configured in the user's Meta developer app, so we hit the base
// URL with no version prefix.

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdAccount {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub account_status: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: Vec<AdAccount>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ApiError,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
    #[serde(default)]
    code: Option<u32>,
}

/// Validate a Meta Marketing API access token by calling /me.
pub fn validate_token(token: &str) -> Result<()> {
    let url = format!("{}/me", META_GRAPH_BASE);
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[("access_token", token), ("fields", "id,name")])
        .send()
        .map_err(|e| crate::meta::net_err("network", e))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().map_err(|e| crate::meta::net_err("json", e))?;
    if !status.is_success() {
        return Err(api_error(status, &body, "the token check"));
    }
    Ok(())
}

/// List ad accounts the token can see.
pub fn list_ad_accounts(token: &str) -> Result<Vec<AdAccount>> {
    let url = format!("{}/me/adaccounts", META_GRAPH_BASE);
    let client = http_client()?;
    let resp = client
        .get(&url)
        .query(&[
            ("access_token", token),
            ("fields", "id,name,account_status"),
            ("limit", "200"),
        ])
        .send()
        .map_err(|e| crate::meta::net_err("network", e))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().map_err(|e| crate::meta::net_err("json", e))?;
    if !status.is_success() {
        return Err(api_error(status, &body, "the ad account list"));
    }
    let parsed: ApiResponse =
        serde_json::from_value(body).map_err(|e| Error::Meta(format!("decode: {}", e)))?;
    Ok(parsed.data)
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Meta(format!("client: {}", e)))
}

/// Transport/body error with the URL stripped: every Meta GET carries
/// access_token in the query string (paging.next URLs embed it too),
/// and reqwest's Display appends "for url (...)".
pub(crate) fn net_err(what: &str, e: reqwest::Error) -> Error {
    Error::Meta(format!("{}: {}", what, e.without_url()))
}

/// Every non-success Meta response becomes one of two errors: code 190
/// (the token expired or was revoked) is MetaAuth, which the refresh and
/// every screen treat as "connect Meta again"; anything else is Meta with
/// Meta's own sentence. `what` says which call failed.
pub(crate) fn api_error(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
    what: &str,
) -> Error {
    let env = serde_json::from_value::<ErrorEnvelope>(body.clone()).ok();
    let code = env.as_ref().and_then(|e| e.error.code);
    let message = env
        .map(|e| e.error.message)
        .unwrap_or_else(|| body.to_string());
    if code == Some(190) {
        return Error::MetaAuth(format!(
            "Meta refused the access token (code 190): it expired or was revoked ({})",
            message
        ));
    }
    match code {
        // Permission (#200, #10): the token's user lost access to this
        // account. Meta's own sentence is a docs link; say where to fix it.
        Some(c @ (200 | 10)) => Error::Meta(format!(
            "the token can't read {}: its user has no ads_read permission there. \
             In Meta Business Settings, give that user access to the ad account again (code {})",
            what, c
        )),
        Some(c) => Error::Meta(format!(
            "Meta {} for {}: {} (code {})",
            status, what, message, c
        )),
        None => Error::Meta(format!("Meta {} for {}: {}", status, what, message)),
    }
}

pub fn account_id_for_storage(act_id_or_id: &str) -> String {
    // Meta returns "act_<digits>"; strip prefix so the config matches mb.py.
    act_id_or_id
        .strip_prefix("act_")
        .unwrap_or(act_id_or_id)
        .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn net_err_never_carries_the_token() {
        // Port 1 on loopback refuses instantly - a real reqwest error.
        let e = reqwest::blocking::Client::new()
            .get("http://127.0.0.1:1/x?access_token=SECRET123")
            .send()
            .unwrap_err();
        assert!(
            e.to_string().contains("SECRET123"),
            "premise: reqwest leaks the url"
        );
        let msg = super::net_err("leads network", e).to_string();
        assert!(!msg.contains("SECRET123"), "{}", msg);
    }

    #[test]
    fn code_190_is_meta_auth_everything_else_meta() {
        let expired =
            serde_json::json!({"error": {"message": "Error validating access token", "code": 190}});
        let e = super::api_error(reqwest::StatusCode::BAD_REQUEST, &expired, "act_1");
        assert_eq!(e.kind(), "meta_auth", "{}", e);
        let denied = serde_json::json!({"error": {"message": "(#200) no permission", "code": 200}});
        let e = super::api_error(reqwest::StatusCode::FORBIDDEN, &denied, "act_1");
        assert_eq!(e.kind(), "meta");
        assert!(
            e.to_string().contains("(code 200)") && !e.to_string().contains("Some("),
            "{}",
            e
        );
        assert!(
            e.reason().starts_with("the token can't read act_1:")
                && e.reason().contains("Business Settings"),
            "{}",
            e
        );
    }
}
