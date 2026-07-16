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
    let spec = load_spec(cfg)?;
    let request = spec.request.as_ref().ok_or_else(|| {
        Error::Config(
            "crm.toml has no [request] - a CSV-only spec; use: moneyball crm import <file.csv>"
                .into(),
        )
    })?;
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
        let resp = request_page(&client, request, &spec.paging, &vars)?;
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

    validate_and_write(cfg, spec.name, &records, &spec.map, pages)
}

/// Import a CSV export through the same spec map + validation pipeline.
/// Offline counterpart to fetch_crm - map paths are CSV column names.
pub fn import_csv(cfg: &AppConfig, file: &std::path::Path) -> Result<CrmFetchReport> {
    let spec = load_spec(cfg)?;
    let raw = std::fs::read_to_string(file)
        .map_err(|e| Error::Config(format!("cannot read {}: {}", file.display(), e)))?;
    let records = source::csv_records(&raw)?;
    validate_and_write(cfg, spec.name, &records, &spec.map, 0)
}

fn load_spec(cfg: &AppConfig) -> Result<SourceSpec> {
    cfg.workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    let spec_file = spec_path(cfg);
    let raw = std::fs::read_to_string(&spec_file).map_err(|_| {
        Error::Config(format!(
            "no CRM spec at {} - create one with: moneyball crm init",
            spec_file.display()
        ))
    })?;
    source::parse(&raw)
}

/// Shared tail of every ingestion path: transform, validate against the
/// contract (+ latest snapshot join), write crm.json only on PASS.
fn validate_and_write(
    cfg: &AppConfig,
    name: String,
    records: &[Value],
    map: &source::MapSpec,
    pages: u32,
) -> Result<CrmFetchReport> {
    let stages = cfg
        .workspace
        .as_ref()
        .map(|w| w.crm.stages.clone())
        .unwrap_or_default();
    let crm = Value::Array(source::transform(records, map));
    let snap = cfg
        .snap_for(None)
        .ok()
        .and_then(|p| crate::snapshot::load(&p).ok());
    let check = super::check(&crm, &stages, snap.as_ref());
    let path = if check.passed() {
        let date = Local::now().date_naive().format("%Y-%m-%d").to_string();
        Some(write_crm_json(cfg, &date, &crm)?)
    } else {
        None
    };
    Ok(CrmFetchReport {
        name,
        tickets: crm.as_array().map(Vec::len).unwrap_or(0),
        pages,
        path,
        check,
    })
}

/// One HTTP call with refs resolved and templates expanded.
fn request_page(
    client: &reqwest::blocking::Client,
    request: &source::RequestSpec,
    paging: &source::PagingSpec,
    vars: &HashMap<&str, String>,
) -> Result<Value> {
    let url = source::expand(&request.url, vars);
    let mut req = match request.method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        m => return Err(Error::Config(format!("unsupported method {}", m))),
    };
    for (k, v) in &request.headers {
        req = req.header(k, source::expand(&source::resolve_ref(v)?, vars));
    }
    let mut query: Vec<(String, String)> = request
        .query
        .iter()
        .map(|(k, v)| Ok((k.clone(), source::expand(&source::resolve_ref(v)?, vars))))
        .collect::<Result<_>>()?;
    // Page mode carries its params implicitly - the spec declares the
    // names once in [paging], not again in [request.query].
    if paging.mode == PagingMode::Page {
        query.push((paging.param.clone(), vars["page"].clone()));
        if !paging.size_param.is_empty() {
            query.push((paging.size_param.clone(), paging.size.to_string()));
        }
    }
    req = req.query(&query);
    if let Some(body) = &request.body {
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
