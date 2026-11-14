//! Meta Graph API client (read-only): every Graph GET goes through
//! get_json / get_all, and the token checks and account list of setup
//! live here (their words in `token`).
//!
//! Per the project hard contract: moneyball-core NEVER writes to Meta.
//! This module is read-only and only ever calls graph.facebook.com.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::blocking::RequestBuilder;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

mod token;
pub use token::TokenCheck;

/// Where every Graph GET goes. Only a build with core's `e2e` feature (the
/// Mac app's end-to-end run) reads `MB_E2E_META_BASE`, to talk to a
/// stand-in Meta; a normal build never reads it. Never a config key: the
/// token goes wherever this points.
// NOTE: do NOT pin a version segment (e.g. /v18.0/) - calls should use the
// version configured in the user's Meta developer app, so we hit the base
// URL with no version prefix.
pub(crate) fn graph_base() -> String {
    #[cfg(feature = "e2e")]
    if let Ok(base) = std::env::var("MB_E2E_META_BASE") {
        return base;
    }
    "https://graph.facebook.com".to_string()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdAccount {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub account_status: Option<u32>,
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
    /// Meta's own "try again later" flag.
    #[serde(default)]
    is_transient: bool,
}

/// Validate a Meta Marketing API access token by calling /me.
pub fn validate_token(token: &str) -> Result<()> {
    let req = http_client()?
        .get(format!("{}/me", graph_base()))
        .query(&[("access_token", token), ("fields", "id,name")]);
    // The wizard waits on these: enter again is their retry.
    fetch_json(req, "the token check", None).map(drop)
}

/// Check a token before it is stored (onboarding Step 1): its shape with
/// no network, then Meta's `/me` (whose it is), `debug_token` (expiry and
/// scopes) and the ad accounts it sees. Stores nothing.
pub fn check_token(token: &str) -> Result<TokenCheck> {
    token::check_shape(token)?;
    let client = http_client()?;
    let get = |path: &str, query: &[(&str, &str)]| {
        let req = client.get(format!("{}/{}", graph_base(), path));
        fetch_json(req.query(query), "the token check", None)
    };
    let me = get("me", &[("access_token", token), ("fields", "name")])?;
    let debug = get(
        "debug_token",
        &[("input_token", token), ("access_token", token)],
    )?;
    let accounts = list_ad_accounts(token)?.len();
    let name = me["name"].as_str().unwrap_or_default();
    TokenCheck::read(name, accounts, &debug["data"])
}

/// List ad accounts the token can see, every page of them.
pub fn list_ad_accounts(token: &str) -> Result<Vec<AdAccount>> {
    let client = http_client()?;
    let first = client
        .get(format!("{}/me/adaccounts", graph_base()))
        .query(&[
            ("access_token", token),
            ("fields", "id,name,account_status"),
            ("limit", "200"),
        ]);
    get_all(&client, first, "the ad account list", None)?
        .into_iter()
        .map(|a| serde_json::from_value(a).map_err(|e| Error::Meta(format!("decode: {}", e))))
        .collect()
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Meta(format!("client: {}", e)))
}

/// How long a pull's call that Meta turned away as temporary waits
/// before its one retry: six run at once, and a burst can trip a limit
/// that clears in seconds.
pub(crate) const PULL: Option<Duration> = Some(Duration::from_secs(10));

/// When a retry last came back rate-limited. For a while after, a
/// rate-limited call fails at once: a limit that outlasted one wait
/// outlasts the next, and every ad's leads call would pay it.
// ponytail: process-wide; per token if moneyball ever serves many.
static STILL_LIMITED: Mutex<Option<Instant>> = Mutex::new(None);
const LIMIT_MEMORY: Duration = Duration::from_secs(300);

/// Status, body (Null when it is not JSON) and the minutes Meta says a
/// rate limit lasts (0 when it names none).
type Reply = (StatusCode, Value, u64);

/// One Graph GET in a pull: its JSON body, or the error every caller
/// reports the same way. A call Meta turns away as temporary (a rate
/// limit, a 5xx, is_transient) waits and goes once more, unless Meta
/// says the limit lasts minutes.
pub(crate) fn get_json(req: RequestBuilder, what: &str) -> Result<Value> {
    fetch_json(req, what, PULL)
}

/// `retry`: how long to wait before the one retry; None reports the
/// refusal at once.
fn fetch_json(req: RequestBuilder, what: &str, retry: Option<Duration>) -> Result<Value> {
    // A GET has no body to stream, so the clone always exists.
    let again = req.try_clone();
    let first = send(req, what)?;
    let (status, body, regain_minutes) = match (again, retry) {
        (Some(again), Some(wait)) if worth_retry(&first) => {
            std::thread::sleep(wait);
            let reply = send(again, what)?;
            if meta_error(&reply.1).is_some_and(|e| throttled(e.code)) {
                *STILL_LIMITED.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
            }
            reply
        }
        _ => first,
    };
    if status.is_success() {
        return match body {
            Value::Null => Err(Error::Meta(format!(
                "Meta answered {} with something other than JSON",
                what
            ))),
            body => Ok(body),
        };
    }
    Err(api_error(status, &body, what, regain_minutes))
}

/// Every `data` row across pages: the first request, then each
/// paging.next (a complete URL, token included) as it is. `retry` as in
/// fetch_json.
pub(crate) fn get_all(
    client: &reqwest::blocking::Client,
    first: RequestBuilder,
    what: &str,
    retry: Option<Duration>,
) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    let mut req = first;
    loop {
        let mut body = fetch_json(req, what, retry)?;
        if let Some(Value::Array(page)) = body.get_mut("data").map(Value::take) {
            rows.extend(page);
        }
        match body.pointer("/paging/next").and_then(|n| n.as_str()) {
            Some(next) => req = client.get(next),
            None => return Ok(rows),
        }
    }
}

fn send(req: RequestBuilder, what: &str) -> Result<Reply> {
    let resp = req.send().map_err(|e| net_err(what, e))?;
    let status = resp.status();
    let regain_minutes = minutes_to_regain(resp.headers());
    let text = resp.text().map_err(|e| net_err(what, e))?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::Null),
        regain_minutes,
    ))
}

/// Meta's rate-limit codes: app (4), user (17), page (32), per hour (613)
/// and the business use case limits (80000s).
fn throttled(code: Option<u32>) -> bool {
    matches!(code, Some(4 | 17 | 32 | 613 | 80000..=80014))
}

/// A refusal worth one more try: a 5xx or Meta's is_transient, or a rate
/// limit Meta names no long wait for and that has not just outlasted a
/// retry.
fn worth_retry((status, body, regain_minutes): &Reply) -> bool {
    if status.is_success() {
        return false;
    }
    match meta_error(body) {
        Some(e) if throttled(e.code) => {
            let last = *STILL_LIMITED.lock().unwrap_or_else(|p| p.into_inner());
            *regain_minutes == 0 && last.is_none_or(|t| t.elapsed() > LIMIT_MEMORY)
        }
        e => status.is_server_error() || e.is_some_and(|e| e.is_transient),
    }
}

fn meta_error(body: &Value) -> Option<ApiError> {
    serde_json::from_value::<ErrorEnvelope>(body.clone())
        .ok()
        .map(|e| e.error)
}

/// The longest wait Meta's business use case header names, in minutes.
fn minutes_to_regain(headers: &reqwest::header::HeaderMap) -> u64 {
    let usage = headers
        .get("x-business-use-case-usage")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let Some(Value::Object(per_business)) = usage else {
        return 0;
    };
    per_business
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|limit| limit.get("estimated_time_to_regain_access")?.as_u64())
        .max()
        .unwrap_or(0)
}

/// Meta's sentence with any access token in it masked ("Malformed access
/// token EAAB..." quotes the token it refused).
fn without_tokens(message: &str) -> String {
    message
        .split(' ')
        .map(|w| match w.starts_with("EAA") && w.len() > 40 {
            true => "<token>",
            false => w,
        })
        .collect::<Vec<_>>()
        .join(" ")
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
fn api_error(status: StatusCode, body: &Value, what: &str, regain_minutes: u64) -> Error {
    let Some(err) = meta_error(body) else {
        return Error::Meta(format!("Meta {} for {}", status, what));
    };
    let (code, message) = (err.code, without_tokens(&err.message));
    if code == Some(190) {
        return Error::MetaAuth(format!(
            "Meta refused the access token (code 190): it expired or was revoked ({})",
            message
        ));
    }
    match code {
        Some(c) if throttled(code) => Error::Meta(format!(
            "Meta is limiting how often this token can call ({}, code {}): try again {}",
            what,
            c,
            match regain_minutes {
                0 => "in a few minutes".to_string(),
                m => format!("in about {}", crate::text::count(m, "minute", "minutes")),
            }
        )),
        // Permission (#200, #10): the token's user lost access to this
        // account. Meta's own sentence is a docs link; say where to fix it.
        Some(c @ (200 | 10)) if what.starts_with("act_") => Error::Meta(format!(
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
#[path = "meta_tests.rs"]
mod tests;
