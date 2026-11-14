//! A pasted curl command as a ConnectInput: the form every CRM's API docs
//! hand out. Split from connect.rs (size cap).

use crate::error::{Error, Result};
use crate::text;

use super::connect::ConnectInput;

impl ConnectInput {
    /// Build from a pasted curl command - the form every CRM's API docs
    /// hand out, and the only sane way to describe a POST-body endpoint
    /// in a terminal. Recognizes -X/--request, -H/--header, -u/--user (a
    /// Basic Authorization header) and -d/--data/--data-raw; everything
    /// else is ignored.
    pub fn from_curl(name: String, curl: &str) -> Result<Self> {
        let toks = shell_tokens(curl);
        let mut url = String::new();
        let mut method = String::new();
        let mut headers = Vec::new();
        let mut body = None;
        let mut it = toks.iter().peekable();
        while let Some(t) = it.next() {
            match t.as_str() {
                "curl" => {}
                "-X" | "--request" => method = it.next().cloned().unwrap_or_default(),
                "-H" | "--header" => {
                    if let Some(h) = it.next() {
                        if let Some((k, v)) = h.split_once(':') {
                            headers.push((k.trim().to_string(), v.trim().to_string()));
                        }
                    }
                }
                "-u" | "--user" => {
                    if let Some(user) = it.next() {
                        let basic = crate::text::base64(user.as_bytes());
                        headers.push(("Authorization".into(), format!("Basic {}", basic)));
                    }
                }
                "-d" | "--data" | "--data-raw" | "--data-binary" => {
                    body = it.next().cloned();
                }
                other if other.starts_with("http") => url = other.to_string(),
                _ => {}
            }
        }
        if url.is_empty() {
            return Err(Error::Config(
                "no URL found in the curl command - paste it exactly as the API docs show".into(),
            ));
        }
        if method.is_empty() {
            method = if body.is_some() { "POST" } else { "GET" }.into();
        }
        // Split the query string into pairs: API docs put credentials
        // there (LeadSquared: accessKey/secretKey), and the connect
        // flow must be able to secretize each value individually.
        let mut query = Vec::new();
        if let Some(qpos) = url.find('?') {
            let qs = url.split_off(qpos + 1);
            url.pop(); // the '?'
            for pair in qs.split('&').filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                query.push((text::percent_decode(k), text::percent_decode(v)));
            }
        }
        Ok(Self {
            name,
            url,
            headers,
            query,
            method,
            body,
        })
    }
}

/// Minimal shell-style tokenizer: whitespace-separated, single/double
/// quotes group, backslash-newline continuations dropped.
fn shell_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.replace("\\\n", " ").chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, _) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_curl_maps_user_to_a_basic_header() {
        let c = ConnectInput::from_curl("x".into(), "curl -u 'user:pa ss' https://x.ai/l").unwrap();
        assert_eq!(
            c.headers,
            vec![(
                "Authorization".to_string(),
                "Basic dXNlcjpwYSBzcw==".to_string()
            )]
        );
    }

    #[test]
    fn from_curl_parses_post_with_headers_and_body() {
        let c = r#"curl -X POST 'https://x.ai/api/q?a=1' -H 'authorization: tok' \
            -H "content-type: application/json" --data '{"page": 0}'"#;
        let i = ConnectInput::from_curl("x".into(), c).unwrap();
        assert_eq!(i.url, "https://x.ai/api/q");
        assert_eq!(i.query, vec![("a".to_string(), "1".to_string())]);
        assert_eq!(i.method, "POST");
        assert_eq!(i.body.as_deref(), Some(r#"{"page": 0}"#));
        assert_eq!(i.headers[0], ("authorization".into(), "tok".into()));
    }

    #[test]
    fn from_curl_splits_and_decodes_query_credentials() {
        let c = "curl 'https://api.example.com/Leads.Get?accessKey=u%24r&secretKey=abc+def&fromDate=2026-01-01'";
        let i = ConnectInput::from_curl("ls".into(), c).unwrap();
        assert_eq!(i.url, "https://api.example.com/Leads.Get");
        assert_eq!(
            i.query,
            vec![
                ("accessKey".to_string(), "u$r".to_string()),
                ("secretKey".to_string(), "abc def".to_string()),
                ("fromDate".to_string(), "2026-01-01".to_string()),
            ]
        );
    }

    #[test]
    fn from_curl_defaults_get_and_requires_url() {
        let i = ConnectInput::from_curl("x".into(), "curl https://a.b/leads").unwrap();
        assert_eq!(i.method, "GET");
        assert!(ConnectInput::from_curl("x".into(), "curl -H 'a: b'").is_err());
    }
}
