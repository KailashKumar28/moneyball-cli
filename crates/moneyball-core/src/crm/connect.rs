//! `crm connect` - agent-drafted onboarding for any REST CRM.
//!
//! The LLM is used exactly ONCE, at setup time: it sees one sample
//! response and drafts `crm.toml`. The draft must parse and dry-run
//! against the sample before it can be saved, and the recurring data
//! path (`crm fetch`) stays fully deterministic afterwards. No direct
//! network here - the probe lives in `crm::fetch`, model calls in `llm`.

use serde_json::Value;

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::provider::ModelProviderInfo;

use super::source::{self, RequestSpec};
use super::CheckReport;

/// What the user supplies interactively before the LLM drafts the rest.
pub struct ConnectInput {
    pub name: String,
    /// Endpoint URL WITHOUT its query string - params live in `query`
    /// so credential values can be secretized before they touch the
    /// LLM or the spec on disk.
    pub url: String,
    /// Header name -> value ref (`secret:<n>` / `env:<VAR>` / literal).
    pub headers: Vec<(String, String)>,
    /// Query param -> value ref, split off the pasted URL.
    pub query: Vec<(String, String)>,
    pub method: String,
    /// JSON body for POST endpoints.
    pub body: Option<String>,
}

impl ConnectInput {
    fn request_spec(&self) -> RequestSpec {
        RequestSpec {
            url: self.url.clone(),
            method: self.method.clone(),
            headers: self.headers.iter().cloned().collect(),
            query: self.query.iter().cloned().collect(),
            body: self.body.clone(),
        }
    }

    /// Build from a pasted curl command - the form every CRM's API docs
    /// hand out, and the only sane way to describe a POST-body endpoint
    /// in a terminal. Recognizes -X/--request, -H/--header, and
    /// -d/--data/--data-raw; everything else is ignored.
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
                query.push((percent_decode(k), percent_decode(v)));
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

/// Undo URL encoding on a query key/value (`%2B`, `+`). The executor
/// re-encodes at request time, so keeping pairs decoded avoids double
/// encoding.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

/// Grab one sample response from the CRM with the supplied auth.
pub fn probe_sample(input: &ConnectInput) -> Result<Value> {
    super::fetch::probe(&input.request_spec())
}

/// Sample bytes shown to the LLM. Enough to see the record shape and a
/// few stage values; small enough to stay cheap and avoid dumping the
/// whole lead book into a prompt.
const SAMPLE_CAP_BYTES: usize = 6_000;

/// Draft a crm.toml from the sample via the workspace's configured LLM.
/// Retries once with parse-error feedback; the returned string is
/// guaranteed to parse as a SourceSpec.
pub fn draft_spec(cfg: &AppConfig, input: &ConnectInput, sample: &Value) -> Result<String> {
    let (pid, pinfo, model) = provider_of(cfg)?;
    let sample_str = truncated_json(sample);
    let mut feedback = String::new();
    for _attempt in 0..2 {
        let user = draft_prompt(input, &sample_str, &feedback);
        let text = crate::llm::stream_blocking(
            &pid,
            &pinfo,
            &model,
            Some(DRAFT_SYSTEM),
            &user,
            &mut |_| {},
        )?;
        let toml_str = extract_toml(&text);
        match source::parse(&toml_str) {
            Ok(_) => return Ok(toml_str),
            Err(e) => feedback = format!("Your previous draft was rejected: {}. Fix it.", e),
        }
    }
    Err(Error::Llm(
        "could not draft a parseable crm.toml in 2 attempts - write it by hand \
         (moneyball crm init) or try a richer sample"
            .into(),
    ))
}

/// Run the drafted spec's map over the sample records and validate the
/// result against the contract (+ latest snapshot join, if any).
/// Returns (record count, report).
pub fn dry_run(cfg: &AppConfig, spec_toml: &str, sample: &Value) -> Result<(usize, CheckReport)> {
    let spec = source::parse(spec_toml)?;
    let records = source::records(sample, &spec.map.root)?;
    let tickets = Value::Array(source::transform(records, &spec.map));
    let stages = cfg
        .workspace
        .as_ref()
        .map(|w| w.crm.stages.clone())
        .unwrap_or_default();
    let snap = cfg
        .snap_for(None)
        .ok()
        .and_then(|p| crate::snapshot::load(&p).ok());
    Ok((
        records.len(),
        super::check(&tickets, &stages, snap.as_ref()),
    ))
}

fn provider_of(cfg: &AppConfig) -> Result<(String, ModelProviderInfo, String)> {
    let w = cfg
        .workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    let pid = w.model_provider.clone().unwrap_or_default();
    let model = w.model.clone().unwrap_or_default();
    if pid.is_empty() || model.is_empty() {
        return Err(Error::Config(
            "no LLM configured - crm connect drafts the spec with your LLM; \
             run /setup (step 4) first, or write crm.toml by hand (moneyball crm init)"
                .into(),
        ));
    }
    let pinfo = w.model_providers.get(&pid).cloned().ok_or_else(|| {
        Error::Config(format!(
            "configured provider '{}' is not in model_providers - re-run /setup",
            pid
        ))
    })?;
    Ok((pid, pinfo, model))
}

fn truncated_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap_or_default();
    if s.len() <= SAMPLE_CAP_BYTES {
        return s;
    }
    // Cut on a char boundary; the LLM only needs the shape + examples.
    let mut end = SAMPLE_CAP_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... (truncated)", &s[..end])
}

/// Pull the toml out of a model response that may wrap it in fences or
/// lead with prose.
fn extract_toml(text: &str) -> String {
    let t = text.trim();
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("toml").unwrap_or(after);
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    // No fences: drop any prose before the first toml key/table.
    match t.find("name") {
        Some(i) => t[i..].trim().to_string(),
        None => t.to_string(),
    }
}

const DRAFT_SYSTEM: &str = "You write moneyball crm.toml source specs. Output ONLY the toml \
document - no prose, no markdown fences, no explanation. The toml must follow this schema \
exactly (keys and sections as shown; omit what you cannot infer):\n\n\
name = \"<crm name>\"\n\n\
[request]\nurl = \"<given verbatim>\"\nmethod = \"<given verbatim>\"\n\
body = '<given verbatim, if a request body was provided; {page} is a template>'\n\
[request.headers]\n# given verbatim (values are secret:/env: refs - keep them)\n\
[request.query]\n# given params verbatim (secret:/env: refs stay); date-like literal values may \
become the {from_date} {to_date} templates; {page} {page_size} are also templates\n\n\
[paging]\n# only if the sample shows paging fields; mode = \"page\" needs param (+ optionally size_param, size)\n\n\
[map]\nroot = \"<dot path to the record array; \\\"\\\" if the response IS the array>\"\n\
ad_id = \"<dot path to the Meta ad id>\"\nstage = \"<dot path to the pipeline stage>\"\n\
delivery = \"<dot path to lead delivery time (epoch or ISO)>\"\nfunnel = \"<dot path or \\\"\\\">\"\n\
[map.stage_map]\n# EVERY stage value visible in the sample -> one of: \
Lost, NonContactable, Contactable, Visit, Revisit, Booking\n\n\
Rules: ad_id must point at the Meta ad id (a long numeric id), not form/campaign/internal ids. \
delivery must be the lead delivery timestamp, not record creation, when both exist. \
Map stages by funnel meaning: unreachable -> NonContactable, in-contact/qualified -> Contactable, \
site/store visit -> Visit, repeat visit -> Revisit, won/booked/purchased -> Booking, dead -> Lost.";

fn draft_prompt(input: &ConnectInput, sample: &str, feedback: &str) -> String {
    let kv = |pairs: &[(String, String)]| {
        pairs
            .iter()
            .map(|(k, v)| format!("\"{}\" = \"{}\"", k, v))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let body = match &input.body {
        Some(b) => format!("Request body (use verbatim as request.body): {}\n", b),
        None => String::new(),
    };
    format!(
        "CRM name: {}\nEndpoint URL (use verbatim): {}\nHTTP method: {}\n{}\
         Headers (use verbatim under [request.headers]):\n{}\n\
         Query params (use verbatim under [request.query]; secret:/env: refs stay as-is):\n{}\n\n\
         One sample response from this endpoint:\n{}\n\n{}",
        input.name,
        input.url,
        input.method,
        body,
        kv(&input.headers),
        kv(&input.query),
        sample,
        feedback
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn extract_toml_handles_fences_and_prose() {
        let fenced = "Here you go:\n```toml\nname = \"x\"\n```\nnotes";
        assert_eq!(extract_toml(fenced), "name = \"x\"");
        let bare = "name = \"x\"\n[map]\nad_id = \"a\"";
        assert_eq!(extract_toml(bare), bare);
        let prose = "Sure! The spec:\nname = \"x\"";
        assert_eq!(extract_toml(prose), "name = \"x\"");
    }

    #[test]
    fn truncated_json_cuts_large_samples() {
        let big = Value::String("x".repeat(SAMPLE_CAP_BYTES * 2));
        let s = truncated_json(&big);
        assert!(s.len() < SAMPLE_CAP_BYTES + 32);
        assert!(s.ends_with("(truncated)"));
    }

    #[test]
    fn dry_run_transforms_and_checks_sample() {
        let cfg = AppConfig::resolve_optional(Some("/nonexistent-mb-test"), None);
        let spec = r#"
name = "t"
[map]
root = "Leads"
ad_id = "aid"
stage = "st"
delivery = "at"
[map.stage_map]
"Hot" = "Contactable"
"#;
        let sample = serde_json::json!({ "Leads": [
            { "aid": "111", "st": "Hot", "at": 1752537600 }
        ]});
        let (n, report) = dry_run(&cfg, spec, &sample).unwrap();
        assert_eq!(n, 1);
        assert!(report.passed(), "errors: {:?}", report.errors);
    }
}
