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
        return Err(Error::Meta(parse_error(&body, status)));
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
        return Err(Error::Meta(parse_error(&body, status)));
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

fn parse_error(body: &serde_json::Value, status: reqwest::StatusCode) -> String {
    if let Ok(env) = serde_json::from_value::<ErrorEnvelope>(body.clone()) {
        format!(
            "Meta {}: {} (code {:?})",
            status, env.error.message, env.error.code
        )
    } else {
        format!("Meta {}: {}", status, body)
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
}
