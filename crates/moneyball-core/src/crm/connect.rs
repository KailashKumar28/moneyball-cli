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
}

/// Grab one sample response from the CRM with the supplied auth (its
/// secret refs resolve against `workspace`'s keys).
pub fn probe_sample(input: &ConnectInput, workspace: Option<&str>, quiet: bool) -> Result<Value> {
    super::fetch::probe(&input.request_spec(), workspace, quiet)
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
    let (mapped, _dropped) = source::transform(records, &spec.map);
    let tickets = Value::Array(mapped);
    Ok((records.len(), super::check_with_workspace(cfg, &tickets)))
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
             run /setup first to pick one, or write crm.toml by hand (moneyball crm init)"
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
    // Capped: the LLM only needs the shape + examples.
    let s = serde_json::to_string_pretty(v).unwrap_or_default();
    crate::text::truncate_marked(&s, SAMPLE_CAP_BYTES, "\n... (truncated)")
}

/// Pull the toml out of a model response that may wrap it in fences or
/// lead with prose.
fn extract_toml(text: &str) -> String {
    let t = text.trim();
    if let Some((_, after)) = t.split_once("```") {
        let after = after.strip_prefix("toml").unwrap_or(after);
        if let Some((body, _)) = after.split_once("```") {
            return body.trim().to_string();
        }
    }
    // No fences: drop any prose before the first toml key/table.
    match t.find("name") {
        Some(i) => t.split_at(i).1.trim().to_string(),
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
        Some(b) => format!(
            "Request body (use verbatim as request.body; {{secret:...}} refs stay as-is): {}\n",
            b
        ),
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
