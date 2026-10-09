//! Meta Graph API client (read-only). Used by the wizard to discover
//! the user's ad accounts when they paste an access token.
//!
//! Per the project hard contract: moneyball-core NEVER writes to Meta.
//! This module is read-only and only ever calls graph.facebook.com.

use std::time::Duration;

use reqwest::blocking::RequestBuilder;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

pub(crate) const META_GRAPH_BASE: &str = "https://graph.facebook.com";
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
        .get(format!("{}/me", META_GRAPH_BASE))
        .query(&[("access_token", token), ("fields", "id,name")]);
    get_json(req, "the token check").map(drop)
}

/// List ad accounts the token can see, every page of them.
pub fn list_ad_accounts(token: &str) -> Result<Vec<AdAccount>> {
    let client = http_client()?;
    let first = client
        .get(format!("{}/me/adaccounts", META_GRAPH_BASE))
        .query(&[
            ("access_token", token),
            ("fields", "id,name,account_status"),
            ("limit", "200"),
        ]);
    get_all(&client, first, "the ad account list")?
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

/// How long a call Meta turned away as temporary waits before its one
/// retry.
const RETRY_WAIT: Duration = Duration::from_secs(10);

/// One Graph GET: its JSON body, or the error every caller reports the
/// same way. A call Meta turns away as temporary (a rate limit, a 5xx,
/// is_transient) waits and goes once more - six run at once, and a burst
/// can trip a limit that clears in seconds - unless Meta says the block
/// lasts minutes.
pub(crate) fn get_json(req: RequestBuilder, what: &str) -> Result<Value> {
    get_json_waiting(req, what, RETRY_WAIT)
}

fn get_json_waiting(req: RequestBuilder, what: &str, wait: Duration) -> Result<Value> {
    // A GET has no body to stream, so the clone always exists.
    let again = req.try_clone();
    let first = send(req, what)?;
    let (status, body, regain) = match again {
        Some(again) if worth_retry(&first) => {
            std::thread::sleep(wait);
            send(again, what)?
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
    Err(api_error(status, &body, what, regain))
}

/// Every `data` row across pages: the first request, then each
/// paging.next (a complete URL, token included) as it is.
pub(crate) fn get_all(
    client: &reqwest::blocking::Client,
    first: RequestBuilder,
    what: &str,
) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    let mut req = first;
    loop {
        let mut body = get_json(req, what)?;
        if let Some(Value::Array(page)) = body.get_mut("data").map(Value::take) {
            rows.extend(page);
        }
        match body.pointer("/paging/next").and_then(|n| n.as_str()) {
            Some(next) => req = client.get(next),
            None => return Ok(rows),
        }
    }
}

/// Status, body (Null when it is not JSON) and the minutes Meta says a
/// rate limit lasts (0 when it names none).
fn send(req: RequestBuilder, what: &str) -> Result<(StatusCode, Value, u64)> {
    let resp = req.send().map_err(|e| net_err(what, e))?;
    let status = resp.status();
    let regain = regain_minutes(resp.headers());
    let text = resp.text().map_err(|e| net_err(what, e))?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::Null),
        regain,
    ))
}

/// Meta's rate-limit codes: app (4), user (17), page (32), per hour (613)
/// and the business use case limits (80000s).
fn throttled(code: Option<u32>) -> bool {
    matches!(code, Some(4 | 17 | 32 | 613 | 80000..=80014))
}

/// A refusal worth one more try: a 5xx, Meta's is_transient, or a rate
/// limit Meta names no long wait for.
fn worth_retry((status, body, regain): &(StatusCode, Value, u64)) -> bool {
    if status.is_success() || *regain > 0 {
        return false;
    }
    status.is_server_error() || envelope(body).is_some_and(|e| e.is_transient || throttled(e.code))
}

fn envelope(body: &Value) -> Option<ApiError> {
    serde_json::from_value::<ErrorEnvelope>(body.clone())
        .ok()
        .map(|e| e.error)
}

/// The longest wait Meta's business use case header names, in minutes.
fn regain_minutes(headers: &reqwest::header::HeaderMap) -> u64 {
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

/// Transport/body error with the URL stripped: every Meta GET carries
/// access_token in the query string (paging.next URLs embed it too),
/// and reqwest's Display appends "for url (...)".
pub(crate) fn net_err(what: &str, e: reqwest::Error) -> Error {
    Error::Meta(format!("{}: {}", what, e.without_url()))
}

/// Every non-success Meta response becomes one of two errors: code 190
/// (the token expired or was revoked) is MetaAuth, which the refresh and
/// every screen treat as "connect Meta again"; anything else is Meta with
/// Meta's own sentence. `what` says which call failed; `regain` is how
/// many minutes Meta says a rate limit lasts.
fn api_error(status: StatusCode, body: &Value, what: &str, regain: u64) -> Error {
    let Some(err) = envelope(body) else {
        return Error::Meta(format!("Meta {} for {}", status, what));
    };
    let (code, message) = (err.code, err.message);
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
            match regain {
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
        let e = super::api_error(reqwest::StatusCode::BAD_REQUEST, &expired, "act_1", 0);
        assert_eq!(e.kind(), "meta_auth", "{}", e);
        let denied = serde_json::json!({"error": {"message": "(#200) no permission", "code": 200}});
        let e = super::api_error(reqwest::StatusCode::FORBIDDEN, &denied, "act_1", 0);
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

    /// A loopback server answering one connection per canned reply
    /// (status line, extra header lines, body); its base URL.
    fn serve(replies: Vec<(&'static str, &'static str, String)>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for (status, headers, body) in replies {
                let (mut conn, _) = listener.accept().unwrap();
                let _ = conn.read(&mut [0u8; 4096]);
                let _ = write!(
                    conn,
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    headers,
                    body.len(),
                    body
                );
            }
        });
        base
    }

    const THROTTLED: &str = r#"{"error":{"message":"User request limit reached","code":17}}"#;

    fn get(base: &str) -> crate::error::Result<serde_json::Value> {
        let req = reqwest::blocking::Client::new().get(format!("{}/x", base));
        super::get_json_waiting(req, "leads", std::time::Duration::ZERO)
    }

    #[test]
    fn a_throttled_call_goes_once_more() {
        let base = serve(vec![
            ("400 Bad Request", "", THROTTLED.into()),
            ("200 OK", "", r#"{"data":[1]}"#.into()),
        ]);
        assert_eq!(get(&base).unwrap()["data"][0], 1);
        // Twice throttled: the second refusal is the error, in words.
        let base = serve(vec![("400 Bad Request", "", THROTTLED.into()); 2]);
        let e = get(&base).unwrap_err().reason();
        assert!(
            e.contains("limiting how often this token can call (leads, code 17)")
                && e.ends_with("try again in a few minutes"),
            "{}",
            e
        );
    }

    #[test]
    fn a_rate_limit_meta_says_lasts_minutes_is_not_retried() {
        // One reply only: a retry would find no server and fail on the network.
        let base = serve(vec![(
            "400 Bad Request",
            "X-Business-Use-Case-Usage: {\"123\":[{\"type\":\"ads_insights\",\"estimated_time_to_regain_access\":19}]}\r\n",
            r#"{"error":{"message":"too many calls","code":80000}}"#.into(),
        )]);
        let e = get(&base).unwrap_err().reason();
        assert!(e.ends_with("try again in about 19 minutes"), "{}", e);
    }

    #[test]
    fn a_refusal_that_is_not_temporary_is_not_retried() {
        let base = serve(vec![(
            "400 Bad Request",
            "",
            r#"{"error":{"message":"(#100) Invalid parameter","code":100}}"#.into(),
        )]);
        let e = get(&base).unwrap_err().reason();
        assert_eq!(
            e,
            "Meta 400 Bad Request for leads: (#100) Invalid parameter (code 100)"
        );
        // A gateway page is not JSON: the status says it, once retried.
        let base = serve(vec![("502 Bad Gateway", "", "<html>".into()); 2]);
        assert_eq!(
            get(&base).unwrap_err().reason(),
            "Meta 502 Bad Gateway for leads"
        );
    }

    #[test]
    fn get_all_follows_every_page() {
        // The server's own address is only known once it listens, so the
        // second page is served by a second server.
        let page2 = serve(vec![("200 OK", "", r#"{"data":[3]}"#.into())]);
        let body = format!(r#"{{"data":[1,2],"paging":{{"next":"{}/p2"}}}}"#, page2);
        let page1 = serve(vec![("200 OK", "", body)]);
        let client = reqwest::blocking::Client::new();
        let rows = super::get_all(&client, client.get(format!("{}/p1", page1)), "leads").unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }
}
