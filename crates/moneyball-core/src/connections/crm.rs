//! Connect a CRM: pick it from the catalog and give its keys, or paste
//! any CRM's curl and let the model draft the spec; then a live test pull
//! gates the save and unknown stage names are mapped once. One flow behind
//! the Prompter seam - `moneyball connect crm` in the shell, /connect crm
//! in the TUI. Keys are stored for this workspace (per client).

use crate::config::AppConfig;
use crate::crm::{connect, fetch as crm_fetch, presets, source, CANONICAL_STAGES};
use crate::error::{Error, Result};

use super::Prompter;

/// How a connect ended. Errors (a bad curl, a network failure) are `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    Connected,
    /// The drafted spec was not saved (the user said no).
    NotSaved,
    /// Saved, but the live test pull failed validation.
    TestFailed,
    Cancelled,
}

/// Bail out of the flow when the user cancels a question.
macro_rules! answer {
    ($p:expr, $prompt:expr, $secret:expr) => {
        match $p.ask(&$prompt, $secret) {
            Some(a) => a,
            None => return Ok(Done::Cancelled),
        }
    };
}

pub fn connect(cfg: &AppConfig, p: &mut dyn Prompter) -> Result<Done> {
    let ws = cfg.workspace_id().ok_or_else(|| {
        Error::Config("no workspace here - CRM keys belong to a client's workspace".into())
    })?;
    let catalog = presets::catalog();
    p.show("connect which CRM?");
    for (i, c) in catalog.iter().enumerate() {
        p.show(&format!("  {}) {}", i + 1, c.display));
    }
    p.show(&format!(
        "  {}) other (paste a curl command; the model drafts the spec)",
        catalog.len() + 1
    ));
    let choice: usize = answer!(p, "pick a number: ", false).parse().unwrap_or(0);
    match choice {
        n if (1..=catalog.len()).contains(&n) => preset(cfg, ws, &catalog[n - 1], p),
        n if n == catalog.len() + 1 => custom(cfg, ws, p),
        _ => Err(Error::Config(format!(
            "pick a number between 1 and {}",
            catalog.len() + 1
        ))),
    }
}

/// Catalog path: credentials only, then test, then stage mapping.
fn preset(cfg: &AppConfig, ws: &str, c: &presets::Preset, p: &mut dyn Prompter) -> Result<Done> {
    for s in c.secrets {
        // Re-runs (e.g. after fixing a stage map) keep the stored
        // credential on an empty answer instead of re-demanding it.
        let stored = crate::secrets::own_crm_key(ws, &c.secret_name(s)).is_some();
        p.show(&format!("{}: {}", s.label, s.help));
        let prompt = if stored {
            format!("{} [enter keeps the stored one]: ", s.label)
        } else {
            format!("{}: ", s.label)
        };
        let v = answer!(p, prompt, true);
        if v.is_empty() {
            if stored {
                continue;
            }
            return Err(Error::Config(format!("{} is required", s.label)));
        }
        crate::secrets::store_crm_key(ws, &c.secret_name(s), &v)?;
    }
    let path = crm_fetch::spec_path(cfg);
    // Reconnecting the same CRM keeps its spec, and the stage map mapped
    // last time with it; only a first connect (or a different CRM) writes.
    let same = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| source::parse(&raw).ok())
        .is_some_and(|spec| spec.name == c.id);
    if same {
        p.show(&format!("kept {} (its stage map stays)", path.display()));
    } else {
        std::fs::write(&path, c.render())?;
        p.show(&format!("saved {}", path.display()));
    }
    test_and_map_stages(cfg, &path, p)
}

/// Unknown CRM: paste the curl from its API docs; header values are
/// stored as secrets (the spec on disk only carries refs); the model
/// drafts the field map from a live sample, dry-run gated.
fn custom(cfg: &AppConfig, ws: &str, p: &mut dyn Prompter) -> Result<Done> {
    let name = answer!(p, "CRM name (e.g. acme-crm): ", false);
    if name.is_empty() {
        return Err(Error::Config("a name is required".into()));
    }
    p.show("paste the 'list leads' request from your CRM's API docs -");
    p.show("a full curl command (POST bodies fine) or just the URL:");
    // Hidden: a curl from API docs carries the credentials in clear.
    let line = answer!(p, "curl or URL (hidden): ", true);
    let mut input = connect::ConnectInput::from_curl(name.clone(), &line)?;
    p.show(&format!(
        "  {} {} - headers: {} - query: {}",
        input.method,
        input.url,
        names(&input.headers),
        names(&input.query)
    ));
    // A body goes to the model and into crm.toml as written: say so when
    // it looks like it carries a credential, and let the user stop.
    let carried = input
        .body
        .as_deref()
        .map(body_credentials)
        .unwrap_or_default();
    if !carried.is_empty() {
        let go = answer!(
            p,
            format!(
                "the request body carries {} - it is sent to the model and saved in crm.toml as is. continue? [y/N]: ",
                carried.join(", ")
            ),
            false
        );
        if !go.eq_ignore_ascii_case("y") {
            p.show("nothing saved - take the credential out of the body, or pick a listed CRM");
            return Ok(Done::NotSaved);
        }
    }
    let secret_name = |k: &str| format!("{}_{}", name, k.to_lowercase().replace('-', "_"));
    for (k, v) in input.headers.iter_mut() {
        let sname = secret_name(k);
        crate::secrets::store_crm_key(ws, &sname, v)?;
        *v = format!("secret:{}", sname);
        p.show(&format!("  header {} stored as secret:{}", k, sname));
    }
    // Query params carry credentials in many CRMs (LeadSquared puts
    // accessKey/secretKey there). Secretize per param - dates and flags
    // must stay literal so the model can turn them into templates.
    for (k, v) in input.query.iter_mut() {
        let looks_secret = looks_credential(k);
        let hint = if looks_secret { "[Y/n]" } else { "[y/N]" };
        let a = answer!(
            p,
            format!(
                "query param \"{}\" - store its value as a secret? {}: ",
                k, hint
            ),
            false
        );
        let yes = if a.is_empty() {
            looks_secret
        } else {
            a.eq_ignore_ascii_case("y")
        };
        if yes {
            let sname = secret_name(k);
            crate::secrets::store_crm_key(ws, &sname, v)?;
            *v = format!("secret:{}", sname);
            p.show(&format!("  query {} stored as secret:{}", k, sname));
        }
    }

    p.show("probing the endpoint for a sample...");
    let sample = connect::probe_sample(&input, Some(ws), cfg.agent)?;
    let preview = sample.to_string();
    p.show(&format!(
        "sample received ({} bytes): {}...",
        preview.len(),
        crate::text::truncate_chars(&preview, 300)
    ));
    p.show("drafting crm.toml with your configured model (a truncated sample is sent to it)...");
    let draft = connect::draft_spec(cfg, &input, &sample)?;
    p.show("----- drafted crm.toml -----");
    for l in draft.lines() {
        p.show(l);
    }
    p.show("----------------------------");
    let (n, report) = connect::dry_run(cfg, &draft, &sample)?;
    p.show(&format!("dry run over the sample: {} record(s)", n));
    for l in report.lines() {
        p.show(&l);
    }
    let verdict = if report.passed() {
        "PASS"
    } else {
        "has errors"
    };
    let path = crm_fetch::spec_path(cfg);
    let save = answer!(
        p,
        format!("dry run {} - save to {}? [y/N]: ", verdict, path.display()),
        false
    );
    if !save.eq_ignore_ascii_case("y") {
        p.show("not saved. connect again, or hand-edit via: moneyball crm init");
        return Ok(Done::NotSaved);
    }
    std::fs::write(&path, &draft)?;
    test_and_map_stages(cfg, &path, p)
}

/// The shared tail: live test pull, then a stage map for whatever stage
/// names the real data contained, then a confirming pull.
fn test_and_map_stages(
    cfg: &AppConfig,
    spec: &std::path::Path,
    p: &mut dyn Prompter,
) -> Result<Done> {
    // The refresh's window: this pull's crm.json replaces the day's, so a
    // shorter test window would shrink what /brief and the report read.
    p.show("running a live test pull (28 days)...");
    let r = crm_fetch::fetch_crm(cfg, 28)?;
    p.show(&format!(
        "test pull ({}): {} ad-attributed ticket(s) over {} page(s){}",
        r.name,
        r.tickets,
        r.pages,
        if r.dropped_no_ad_id > 0 {
            format!(
                " ({} organic/direct lead(s) dropped - no ad id)",
                r.dropped_no_ad_id
            )
        } else {
            String::new()
        }
    ));
    for l in r.check.lines() {
        p.show(&l);
    }
    if r.path.is_none() {
        p.show(&format!(
            "FAIL - fix the errors above (edit {}) and connect again",
            spec.display()
        ));
        return Ok(Done::TestFailed);
    }
    if !r.check.unknown_stages.is_empty() {
        p.show("your CRM uses stage names moneyball doesn't know. map each one:");
        for (i, c) in CANONICAL_STAGES.iter().enumerate() {
            p.show(&format!("  {}) {}", i + 1, c));
        }
        p.show("  0) keep as-is (counts as a lead, never as qualified)");
        let mut pairs = Vec::new();
        for stage in &r.check.unknown_stages {
            let a = answer!(
                p,
                format!("\"{}\" -> [0-{}]: ", stage, CANONICAL_STAGES.len()),
                false
            );
            match a.parse::<usize>() {
                Ok(n) if (1..=CANONICAL_STAGES.len()).contains(&n) => {
                    pairs.push((stage.clone(), CANONICAL_STAGES[n - 1].to_string()));
                }
                _ => p.show(&format!("  keeping \"{}\" as-is", stage)),
            }
        }
        if !pairs.is_empty() {
            let text = std::fs::read_to_string(spec)?;
            std::fs::write(spec, source::add_stage_mappings(&text, &pairs)?)?;
            p.show("stage map saved - confirming with another pull...");
            let r2 = crm_fetch::fetch_crm(cfg, 28)?;
            for l in r2.check.lines() {
                p.show(&l);
            }
        }
    }
    p.show("connected. each refresh pulls it with Meta (moneyball refresh, or /refresh).");
    Ok(Done::Connected)
}

fn names(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return "none".into();
    }
    pairs
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The credential-looking field names in a request body (JSON or form).
fn body_credentials(body: &str) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    if looks_credential(k) {
                        out.push(k.clone());
                    }
                    walk(v, out);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) => walk(&v, &mut out),
        Err(_) => out.extend(
            body.split('&')
                .filter_map(|kv| kv.split_once('=').map(|(k, _)| k.to_string()))
                .filter(|k| looks_credential(k)),
        ),
    }
    out
}

/// Does this query-param name look like it carries a credential?
/// Drives only the DEFAULT of the store-as-secret prompt above.
fn looks_credential(key: &str) -> bool {
    let k = key.to_lowercase();
    ["key", "token", "secret", "auth", "sig", "pass", "pwd"]
        .iter()
        .any(|n| k.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers in order; None once they run out (the user cancelled).
    struct Script(Vec<&'static str>, Vec<String>);

    impl Prompter for Script {
        fn show(&mut self, line: &str) {
            self.1.push(line.to_string());
        }
        fn ask(&mut self, _prompt: &str, _secret: bool) -> Option<String> {
            (!self.0.is_empty()).then(|| self.0.remove(0).to_string())
        }
    }

    fn cfg() -> AppConfig {
        AppConfig {
            data_root: std::env::temp_dir().join("mb-connect-crm-none"),
            date: None,
            workspace: Some(
                serde_json::from_value(serde_json::json!({
                    "workspace_id": "ws-test", "products": []
                }))
                .unwrap(),
            ),
            agent: true,
        }
    }

    #[test]
    fn credentials_in_a_body_are_named() {
        let json = r#"{"filter": {"stage": "x"}, "auth": {"apiKey": "k", "page": 1}}"#;
        assert_eq!(body_credentials(json), vec!["auth", "apiKey"]);
        assert_eq!(body_credentials("q=1&access_token=t"), vec!["access_token"]);
        assert!(body_credentials(r#"{"page": 1}"#).is_empty());
    }

    #[test]
    fn the_flow_runs_on_any_prompter_and_stops_where_told() {
        // Cancel at the first question: nothing chosen, nothing saved.
        let mut p = Script(vec![], vec![]);
        assert_eq!(connect(&cfg(), &mut p).unwrap(), Done::Cancelled);
        assert!(p.1.iter().any(|l| l.contains("LeadZump")), "{:?}", p.1);

        // A pick off the list is an error that names the range.
        let e = connect(&cfg(), &mut Script(vec!["99"], vec![])).unwrap_err();
        assert!(e.to_string().contains("between 1 and"), "{}", e);

        // No workspace: CRM keys have nowhere to go.
        let mut none = cfg();
        none.workspace = None;
        assert!(connect(&none, &mut Script(vec!["1"], vec![])).is_err());
    }
}
