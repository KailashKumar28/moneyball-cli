//! meta tests. Split from meta.rs (size cap).

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
    super::fetch_json(req, "leads", Some(std::time::Duration::ZERO))
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
    // A limit that outlasted a retry: the next limited call fails at
    // once (one reply only - a retry would find no server).
    let base = serve(vec![("400 Bad Request", "", THROTTLED.into())]);
    assert!(get(&base).unwrap_err().reason().contains("code 17"));
}

#[test]
fn a_token_meta_quotes_back_is_masked() {
    let token = format!("EAAB{}", "x".repeat(60));
    let body = serde_json::json!({"error": {
        "message": format!("Malformed access token {}", token), "code": 190}});
    let e = super::api_error(reqwest::StatusCode::BAD_REQUEST, &body, "act_1", 0);
    assert!(!e.to_string().contains(&token), "{}", e);
    assert!(
        e.to_string().contains("Malformed access token <token>"),
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
fn a_refusal_is_retried_only_when_temporary() {
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
    // A gateway page is not JSON: retried, and when it persists the
    // status says it.
    let base = serve(vec![
        ("502 Bad Gateway", "", "<html>".into()),
        ("200 OK", "", r#"{"data":[]}"#.into()),
    ]);
    assert!(get(&base).is_ok());
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
    let first = client.get(format!("{}/p1", page1));
    let rows = super::get_all(&client, first, "leads", None).unwrap();
    assert_eq!(rows, vec![1, 2, 3]);
}
