//! oauth tests: the listener against raw requests, as a browser sends them.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::*;

const TOKEN: &str = "EAAMoneyballTestTokenNeverToBeShown0123456789";

/// The browser's request as written, and the answer it gets.
fn send(port: u16, request: &str) -> String {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    conn.read_to_string(&mut answer).unwrap();
    answer
}

/// The broker's page submitting `body` as a form, as the browser does.
pub(crate) fn post(port: u16, body: &str) -> String {
    let request = format!(
        "POST /meta/done HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{}",
        port,
        body.len(),
        body
    );
    send(port, &request)
}

/// A listener waiting on its own thread: its port, its state and the result to come.
fn waiting(
    timeout: Duration,
    cancel: Arc<AtomicBool>,
) -> (u16, String, JoinHandle<Result<String>>) {
    let back = Loopback::bind().unwrap();
    let (port, state) = (back.port(), back.state().to_string());
    let done = thread::spawn(move || back.token("/meta/done", &cancel, timeout));
    (port, state, done)
}

#[test]
fn the_state_is_32_random_bytes_in_base64url() {
    let a = Loopback::bind().unwrap();
    let b = Loopback::bind().unwrap();
    assert_eq!(a.state().len(), 43);
    assert!(a
        .state()
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_ne!(a.state(), b.state());
    assert_ne!(a.port(), b.port());
}

#[test]
fn only_a_post_with_the_state_is_taken_and_the_token_is_never_echoed() {
    let (port, state, done) = waiting(TIMEOUT, Arc::default());
    // Another path, a GET carrying everything, a wrong state, no form:
    // each refused, and the listener keeps waiting.
    assert!(send(port, "GET /favicon.ico HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    let get = format!(
        "GET /meta/done?token={}&state={} HTTP/1.1\r\n\r\n",
        TOKEN, state
    );
    let got = send(port, &get);
    assert!(
        got.starts_with("HTTP/1.1 405") && got.contains("Allow: POST"),
        "{}",
        got
    );
    let wrong = post(port, &format!("token={}&state=not-{}", TOKEN, state));
    assert!(wrong.starts_with("HTTP/1.1 400"), "{}", wrong);
    assert!(!wrong.contains(TOKEN));
    let json =
        "POST /meta/done HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    assert!(send(port, json).starts_with("HTTP/1.1 415"));
    assert!(!done.is_finished());

    let body = format!("state={}&token={}", state, TOKEN);
    let ok = post(port, &body);
    assert!(ok.starts_with("HTTP/1.1 200"), "{}", ok);
    assert!(ok.contains("Connected. You can close this tab.") && !ok.contains(TOKEN));
    assert_eq!(done.join().unwrap().unwrap(), TOKEN);
    // Closed: nothing listens there any more.
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn an_error_is_forwarded_in_words_and_shown_to_the_browser() {
    let (port, state, done) = waiting(TIMEOUT, Arc::default());
    let words = "Facebook sign-in was cancelled, so nothing was connected.";
    let page = post(
        port,
        &format!("error={}&state={}", text::percent_encode(words), state),
    );
    assert!(
        page.starts_with("HTTP/1.1 200") && page.contains(words),
        "{}",
        page
    );
    let e = done.join().unwrap().unwrap_err();
    assert_eq!((e.kind(), e.reason()), ("authorization", words.to_string()));
}

#[test]
fn cancel_and_the_timeout_end_the_wait_without_the_token() {
    let cancel = Arc::new(AtomicBool::new(false));
    let (_, _, done) = waiting(TIMEOUT, cancel.clone());
    let asked = Instant::now();
    cancel.store(true, Ordering::SeqCst);
    let e = done.join().unwrap().unwrap_err();
    assert!(asked.elapsed() < Duration::from_secs(1));
    assert_eq!(e.kind(), "authorization");
    assert!(e.reason().starts_with("Cancelled"), "{}", e);

    let (port, state, done) = waiting(Duration::from_millis(300), Arc::default());
    // A token with the wrong state is not taken, and not remembered.
    post(port, &format!("token={}&state=x{}", TOKEN, state));
    let e = done.join().unwrap().unwrap_err();
    assert!(e.reason().contains("within 1 minute,"), "{}", e);
    assert!(!e.to_string().contains(TOKEN));
}
