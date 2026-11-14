//! An authorization's last step (plans/desktop/auth-plan.md, "The Meta
//! protocol" steps 1 and 5): a one-shot listener on 127.0.0.1 that takes
//! the token the broker's page POSTs back, answers the browser, and
//! closes. It makes no request of its own. Blocking std::net, polled, so
//! Cancel and the timeout are seen within `POLL`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::text;

/// How long the person gets to finish in the browser.
pub const TIMEOUT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(50);
/// Enough for a token, a state and a sentence; a bigger request is not the broker's.
const MAX_REQUEST: u64 = 64 * 1024;

/// A listener on 127.0.0.1 at a port the system picks, and the `state`
/// only this attempt knows.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
    state: String,
}

impl Loopback {
    pub fn bind() -> Result<Loopback> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        Ok(Loopback {
            port: listener.local_addr()?.port(),
            listener,
            state: new_state()?,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 32 random bytes, base64url: what the broker must POST back.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Wait for `POST <path>` with this attempt's state, answer the
    /// browser, and return the `token` it brought, or its `error` (the
    /// broker's words). Anything else is refused and the wait goes on,
    /// until `cancel` or `timeout`. The listener closes on return.
    pub fn token(self, path: &str, cancel: &AtomicBool, timeout: Duration) -> Result<String> {
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Authorization(
                    "Cancelled, so nothing was connected.".into(),
                ));
            }
            if started.elapsed() >= timeout {
                return Err(Error::Authorization(format!(
                    "Signing in was not finished in the browser within {}, so nothing was \
                     connected. Try again.",
                    text::count(
                        timeout.as_millis().div_ceil(60_000) as u64,
                        "minute",
                        "minutes"
                    )
                )));
            }
            // WouldBlock, or a connection that reset before it was taken.
            let Ok((conn, _)) = self.listener.accept() else {
                std::thread::sleep(POLL);
                continue;
            };
            if let Some(done) = self.answer(conn, path) {
                return done;
            }
        }
    }

    /// One connection: Some once the request is this attempt's answer.
    // ponytail: one connection at a time, each read capped at 5 s; a
    // client that connects and stays silent delays the broker's POST by that.
    fn answer(&self, mut conn: TcpStream, path: &str) -> Option<Result<String>> {
        let form = match read_form(&conn, path) {
            Ok(form) => form,
            Err(status) => {
                reply(
                    &mut conn,
                    status,
                    "This is not the page Moneyball is waiting for.",
                );
                return None;
            }
        };
        let field = |name: &str| {
            form.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        if field("state") != Some(self.state.as_str()) {
            reply(&mut conn, 400, "This sign-in is not the one Moneyball is waiting for. Start again from the Moneyball app.");
            return None;
        }
        let done = match (field("token"), field("error")) {
            (Some(token), None) if !token.is_empty() => {
                reply(&mut conn, 200, "Connected. You can close this tab.");
                return Some(Ok(token.to_string()));
            }
            (_, Some(error)) => text::truncate_marked(error, 300, "..."),
            _ => {
                "The sign-in came back without a token, so nothing was connected. Try again.".into()
            }
        };
        reply(&mut conn, 200, &format!("{} You can close this tab.", done));
        Some(Err(Error::Authorization(done)))
    }
}

fn new_state() -> Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let b64 = text::base64(&bytes).replace('+', "-").replace('/', "_");
    Ok(b64.trim_end_matches('=').to_string())
}

/// The urlencoded body of `POST <path>`, or the status that refuses the
/// request: 404 another path, 405 another method, 411/413/415 a body that
/// is not a form of a sane size, 400 anything malformed.
fn read_form(conn: &TcpStream, path: &str) -> std::result::Result<Vec<(String, String)>, u16> {
    // The listener polls; this connection waits, but not forever.
    conn.set_nonblocking(false).map_err(|_| 400u16)?;
    conn.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| 400u16)?;
    let mut r = BufReader::new(conn.take(MAX_REQUEST));
    let line = read_line(&mut r)?;
    let mut parts = line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().ok_or(400u16)?);
    if target.split('?').next() != Some(path) {
        return Err(404);
    }
    if method != "POST" {
        return Err(405);
    }
    let (mut length, mut form) = (None, false);
    loop {
        let header = read_line(&mut r)?;
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').ok_or(400u16)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.parse::<u64>().map_err(|_| 400u16)?);
        } else if name.eq_ignore_ascii_case("content-type") {
            form = value.starts_with("application/x-www-form-urlencoded");
        }
    }
    let length = length.ok_or(411u16)?;
    if length > MAX_REQUEST / 2 {
        return Err(413);
    }
    if !form {
        return Err(415);
    }
    let mut body = Vec::new();
    r.take(length).read_to_end(&mut body).map_err(|_| 400u16)?;
    if body.len() as u64 != length {
        return Err(400);
    }
    let body = String::from_utf8(body).map_err(|_| 400u16)?;
    Ok(body
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').unwrap_or((p, "")))
        .map(|(k, v)| (text::percent_decode(k), text::percent_decode(v)))
        .collect())
}

/// One CRLF-ended line, without its ending.
fn read_line(r: &mut impl BufRead) -> std::result::Result<String, u16> {
    let mut line = String::new();
    match r.read_line(&mut line) {
        Ok(n) if n > 0 && line.ends_with('\n') => Ok(line.trim_end_matches(['\r', '\n']).into()),
        _ => Err(400),
    }
}

/// A small page saying `says`, and the connection closed.
fn reply(conn: &mut TcpStream, status: u16, says: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        _ => "Bad Request",
    };
    let allow = if status == 405 { "Allow: POST\r\n" } else { "" };
    let says = says
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Moneyball</title>\
         <style>:root{{color-scheme:light dark}}body{{font:16px/1.5 -apple-system,system-ui,\
         sans-serif;max-width:32rem;margin:20vh auto;padding:0 16px}}</style></head>\
         <body><p>{}</p></body></html>",
        says
    );
    let _ = write!(
        conn,
        "HTTP/1.1 {} {}\r\n{}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        allow,
        body.len(),
        body
    );
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
pub(crate) mod tests;
