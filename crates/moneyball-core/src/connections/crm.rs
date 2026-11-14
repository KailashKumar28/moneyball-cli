//! Connect a CRM: pick it from the catalog and give its keys, or paste
//! any CRM's curl and let the model draft the spec; then a live test pull
//! gates the save and unknown stage names are mapped once. One flow behind
//! the Prompter seam - `moneyball connect crm` in the shell, /connect crm
//! in the TUI - looping over the plain steps the Mac app calls (steps.rs).
//! Keys are stored for this workspace (per client).

use std::collections::HashMap;

use crate::config::AppConfig;
use crate::crm::{connect, fetch as crm_fetch, presets, CANONICAL_STAGES};
use crate::error::{Error, Result};

use super::Prompter;

mod steps;
pub use steps::{
    preset_forms, save_preset, save_stage_map, test_pull, FormField, PresetForm, SpecSaved,
    TestPull, TEST_DAYS,
};
use steps::save_spec;

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
            // Into: Done in the flow, Some(Done) in its steps.
            None => return Ok(Done::Cancelled.into()),
        }
    };
}

// After answer!: the steps use it.
mod request;

pub fn connect(cfg: &AppConfig, p: &mut dyn Prompter) -> Result<Done> {
    let ws = steps::workspace(cfg)?;
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

/// Catalog path: the preset's fields one by one, then save, test and
/// map stages.
fn preset(cfg: &AppConfig, ws: &str, c: &presets::Preset, p: &mut dyn Prompter) -> Result<Done> {
    let form = steps::form(cfg, ws, c);
    let mut values = HashMap::new();
    for f in &form.fields {
        p.show(&format!("{}: {}", f.label, f.help));
        // Re-runs (e.g. after fixing a stage map) keep the stored
        // credential on an empty answer instead of re-demanding it.
        let prompt = match (f.stored, f.refused) {
            (true, true) => format!(
                "{} [the stored one was refused on the last pull - paste a fresh one]: ",
                f.label
            ),
            (true, false) => format!("{} [enter keeps the stored one]: ", f.label),
            (false, _) => format!("{}: ", f.label),
        };
        let v = answer!(p, prompt, f.secret);
        f.require(&v)?;
        values.insert(f.key.to_string(), v);
    }
    let path = crm_fetch::spec_path(cfg);
    match save_preset(cfg, c.id, &values)? {
        SpecSaved::Kept => p.show(&format!("kept {} (its stage map stays)", path.display())),
        SpecSaved::Written => p.show(&format!("saved {}", path.display())),
    }
    test_and_map_stages(cfg, p)
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
    if let Some(done) = request::secretize(ws, &name, &mut input, p)? {
        return Ok(done);
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
    save_spec(&path, draft)?;
    test_and_map_stages(cfg, p)
}

/// The shared tail: live test pull, then a stage map for whatever stage
/// names the real data contained, then a confirming pull.
fn test_and_map_stages(cfg: &AppConfig, p: &mut dyn Prompter) -> Result<Done> {
    p.show(&format!("running a live test pull ({} days)...", TEST_DAYS));
    let r = test_pull(cfg)?;
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
    for l in &r.lines {
        p.show(l);
    }
    if !r.passed {
        p.show(&format!(
            "FAIL - fix the errors above (edit {}) and connect again",
            crm_fetch::spec_path(cfg).display()
        ));
        return Ok(Done::TestFailed);
    }
    if !r.unknown_stages.is_empty() {
        p.show("your CRM uses stage names moneyball doesn't know. map each one:");
        for (i, c) in CANONICAL_STAGES.iter().enumerate() {
            p.show(&format!("  {}) {}", i + 1, c));
        }
        p.show("  0) keep as-is (counts as a lead, never as qualified)");
        let mut choices = Vec::new();
        for stage in &r.unknown_stages {
            let a = answer!(
                p,
                format!("\"{}\" -> [0-{}]: ", stage, CANONICAL_STAGES.len()),
                false
            );
            let to = match a.parse::<usize>() {
                Ok(n) if (1..=CANONICAL_STAGES.len()).contains(&n) => {
                    Some(CANONICAL_STAGES[n - 1].to_string())
                }
                _ => {
                    p.show(&format!("  keeping \"{}\" as-is", stage));
                    None
                }
            };
            choices.push((stage.clone(), to));
        }
        if save_stage_map(cfg, &choices)? {
            p.show("stage map saved - confirming with another pull...");
            for l in test_pull(cfg)?.lines {
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
