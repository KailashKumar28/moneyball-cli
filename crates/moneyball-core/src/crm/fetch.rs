//! CRM fetch executor - runs the `crm.toml` spec against the CRM's HTTP
//! endpoint and writes a validated `crm.json` into today's snapshot dir.
//!
//! The fourth and last network module (with meta.rs, fetch.rs, llm.rs).
//! Deterministic: no LLM anywhere in this path - the spec was authored
//! once (by hand or via /crm connect) and executes the same way forever.
//! The validator gates the write: a non-conformant transform never
//! replaces good data on disk.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Duration, Local};
use serde_json::Value;

use crate::config::AppConfig;
use crate::error::{Error, Result};

use super::source::{self, PagingMode, SourceSpec};
use super::CheckReport;

pub struct CrmFetchReport {
    pub name: String,
    pub tickets: usize,
    pub pages: u32,
    /// Set only when validation passed and crm.json was written.
    pub path: Option<PathBuf>,
    pub check: CheckReport,
}

/// Spec location: `<workspace>/.moneyball/crm.toml`.
pub fn spec_path(cfg: &AppConfig) -> PathBuf {
    cfg.mb_dir().join("crm.toml")
}

/// Execute the workspace crm.toml: pull `days` of leads, transform to
/// contract tickets, validate, and (only on PASS) write
/// `<snap>/<today>/crm.json`.
pub fn fetch_crm(cfg: &AppConfig, days: u32) -> Result<CrmFetchReport> {
    let workspace = cfg
        .workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    let spec_file = spec_path(cfg);
    let raw = std::fs::read_to_string(&spec_file).map_err(|_| {
        Error::Config(format!(
            "no CRM spec at {} - create one with: moneyball crm init",
            spec_file.display()
        ))
    })?;
    let spec = source::parse(&raw)?;
    if spec.paging.mode == PagingMode::Page && spec.paging.param.is_empty() {
        return Err(Error::Config(
            "crm.toml: paging.mode = \"page\" requires paging.param".into(),
        ));
    }

    let today = Local::now().date_naive();
    let from = today - Duration::days(days.max(1) as i64);
    let mut vars: HashMap<&str, String> = HashMap::from([
        ("from_date", from.format("%Y-%m-%d").to_string()),
        ("to_date", today.format("%Y-%m-%d").to_string()),
        ("page_size", spec.paging.size.to_string()),
    ]);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Config(format!("client: {}", e)))?;

    let mut records: Vec<Value> = Vec::new();
    let mut page = spec.paging.start;
    let mut pages = 0u32;
    // Runaway backstop: a CRM that ignores the page param would otherwise
    // return the same full batch forever.
    const MAX_PAGES: u32 = 500;
    loop {
        vars.insert("page", page.to_string());
        let resp = request_page(&client, &spec, &vars)?;
        let batch = source::records(&resp, &spec.map.root)?;
        let n = batch.len();
        records.extend(batch.iter().cloned());
        pages += 1;
        let done = spec.paging.mode == PagingMode::None || n < spec.paging.size as usize;
        if done {
            break;
        }
        if pages >= MAX_PAGES {
            return Err(Error::Config(format!(
                "{} pages and every one full - the endpoint is likely ignoring paging.param \"{}\"; check the spec",
                pages, spec.paging.param
            )));
        }
        page += 1;
    }

    let tickets = source::transform(&records, &spec.map);
    let crm = Value::Array(tickets);

    // Validate before writing; join against the latest snapshot if any.
    let snap = cfg
        .snap_for(None)
        .ok()
        .and_then(|p| crate::snapshot::load(&p).ok());
    let check = super::check(&crm, &workspace.crm.stages, snap.as_ref());

    let path = if check.passed() {
        let date = today.format("%Y-%m-%d").to_string();
        Some(write_crm_json(cfg, &date, &crm)?)
    } else {
        None
    };
    Ok(CrmFetchReport {
        name: spec.name,
        tickets: crm.as_array().map(Vec::len).unwrap_or(0),
        pages,
        path,
        check,
    })
}

/// One HTTP call with refs resolved and templates expanded.
fn request_page(
    client: &reqwest::blocking::Client,
    spec: &SourceSpec,
    vars: &HashMap<&str, String>,
) -> Result<Value> {
    let url = source::expand(&spec.request.url, vars);
    let mut req = match spec.request.method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        m => return Err(Error::Config(format!("unsupported method {}", m))),
    };
    for (k, v) in &spec.request.headers {
        req = req.header(k, source::expand(&source::resolve_ref(v)?, vars));
    }
    let mut query: Vec<(String, String)> = spec
        .request
        .query
        .iter()
        .map(|(k, v)| Ok((k.clone(), source::expand(&source::resolve_ref(v)?, vars))))
        .collect::<Result<_>>()?;
    // Page mode carries its params implicitly - the spec declares the
    // names once in [paging], not again in [request.query].
    if spec.paging.mode == PagingMode::Page {
        query.push((spec.paging.param.clone(), vars["page"].clone()));
        if !spec.paging.size_param.is_empty() {
            query.push((spec.paging.size_param.clone(), spec.paging.size.to_string()));
        }
    }
    req = req.query(&query);
    if let Some(body) = &spec.request.body {
        req = req
            .header("Content-Type", "application/json")
            .body(source::expand(body, vars));
    }
    let resp = req
        .send()
        .map_err(|e| Error::Config(format!("CRM request failed: {}", e)))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .map_err(|e| Error::Config(format!("CRM response is not JSON: {}", e)))?;
    if !status.is_success() {
        return Err(Error::Config(format!(
            "CRM returned {}: {}",
            status,
            preview(&body)
        )));
    }
    Ok(body)
}

/// First ~200 chars of an error body - enough to diagnose, never a dump.
fn preview(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 200 {
        format!("{}...", &s[..200])
    } else {
        s
    }
}

/// Atomic write of crm.json into the date's snapshot dir (tmp + rename,
/// same pattern as fetch::write_snapshot).
fn write_crm_json(cfg: &AppConfig, date: &str, crm: &Value) -> Result<PathBuf> {
    let dir = cfg.history_dir().join("snap").join(date);
    std::fs::create_dir_all(&dir)?;
    let final_path = dir.join("crm.json");
    let tmp = dir.join("crm.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(crm)?)?;
    std::fs::rename(&tmp, &final_path)?;
    Ok(final_path)
}
