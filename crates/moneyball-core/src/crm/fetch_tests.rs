//! CRM fetch tests: auth and status errors. Split from fetch.rs (size cap).

use super::*;

#[test]
fn auth_failure_is_actionable_even_with_empty_body() {
    let e = status_error(
        reqwest::StatusCode::UNAUTHORIZED,
        "",
        &["leadzump_token".into()],
    );
    let msg = e.to_string();
    assert!(
        msg.contains("401") && msg.contains("moneyball crm secret leadzump_token"),
        "{}",
        msg
    );
    assert!(
        !msg.contains("<name>"),
        "the real name, never a placeholder: {}",
        msg
    );
    assert!(!msg.contains("not JSON"), "never the parse error: {}", msg);
}

#[test]
fn other_statuses_carry_a_truncated_snippet() {
    let e = status_error(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        &"boom".repeat(200),
        &[],
    );
    let msg = e.to_string();
    assert!(
        msg.contains("500") && msg.contains("...") && msg.len() < 400,
        "{}",
        msg
    );
}

#[test]
fn a_refused_request_is_fixed_by_reconnecting_and_a_busy_crm_by_retrying() {
    // A retry sends the same request: the fix is redoing the connection.
    let e = status_error(reqwest::StatusCode::BAD_REQUEST, "", &[]);
    assert_eq!(e.kind(), "crm_request");
    assert_eq!(e.fix().as_deref(), Some("/connect crm"));
    assert_eq!(e.reason(), "CRM returned 400 Bad Request");
    for later in [
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        reqwest::StatusCode::REQUEST_TIMEOUT,
        reqwest::StatusCode::BAD_GATEWAY,
    ] {
        assert_eq!(status_error(later, "", &[]).fix(), None, "{}", later);
    }
}
