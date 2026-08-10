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
use std::time::Instant;

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
    /// In-window records dropped for having no ad id (organic/direct
    /// leads - normal in every CRM; they never join to Meta spend).
    pub dropped_no_ad_id: usize,
    /// Set only when validation passed and crm.json was written.
    pub path: Option<PathBuf>,
    pub check: CheckReport,
}

/// Per-call progress reporting for `request_page`. Suppressed under
/// `MB_AGENT=1` (machine-readable output stays clean).
struct Progress<'a> {
    /// True when the binary is running in machine-readable mode; the
    /// caller passes `cfg.agent` here.
    agent: bool,
    /// Short label like "page 3" or "probe"; `None` means a single-shot
    /// request without pagination context.
    label: Option<&'a str>,
}

/// Human-friendly elapsed time: `1.2s`, `340ms`, `2m03s`. ASCII only.
pub(super) fn fmt_elapsed(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1_000 {
        return format!("{}ms", ms);
    }
    let s = d.as_secs();
    if s < 60 {
        let tenths = (ms % 1_000) / 100;
        return format!("{}.{}s", s, tenths);
    }
    let m = s / 60;
    let r = s % 60;
    format!("{}m{:02}s", m, r)
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
    let today = Local::now().date_naive();
    let from = today - Duration::days(days.max(1) as i64);
    // from_epoch is the start of the window in epoch seconds; the
    // client-side cutover drops any record whose delivery is older. The
    // server-side cutover (via the spec's body/query) is preferred when
    // the connector's DSL supports it - this is the fallback for ones
    // that don't (LeadZump's DSL only supports EQUALS, for example).
    let from_epoch = from
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(Local).single())
        .map(|dt| dt.timestamp())
        .unwrap_or(i64::MIN);
    let mut vars: HashMap<&str, String> = HashMap::from([
        ("from_date", from.format("%Y-%m-%d").to_string()),
        ("to_date", today.format("%Y-%m-%d").to_string()),
        ("page_size", spec.paging.size.to_string()),
    ]);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Config(format!("client: {}", e)))?;

    if !cfg.agent {
        eprintln!(
            "fetching {} days of leads from {} ({} {})",
            days,
            spec.name,
            request.method.to_uppercase(),
            request.url
        );
    }
    let start = Instant::now();
    let mut records: Vec<Value> = Vec::new();
    let mut in_window: usize = 0;
    let mut total_pulled: usize = 0;
    let mut total_elements: Option<usize> = None;
    let mut page = spec.paging.start;
    let mut pages = 0u32;
    // Safety backstop: a connector that ignores BOTH paging.param and
    // doesn't expose totalElements would loop forever otherwise. With
    // totalElements honored, this is unreachable on well-behaved
    // connectors (Spring Data, etc.); raise it so a single bad pull
    // can't dominate runtime on dumb ones.
    const MAX_PAGES: u32 = 10_000;
    loop {
        vars.insert("page", page.to_string());
        let label = if spec.paging.mode == PagingMode::Page {
            Some(format!("page {}", page))
        } else {
            None
        };
        let resp = request_page(
            &client,
            request,
            &spec.paging,
            &vars,
            &Progress {
                agent: cfg.agent,
                label: label.as_deref(),
            },
        )?;
        let batch = source::records(&resp, &spec.map.root)?;
        let n = batch.len();
        // Server-side total: first page only. LeadZump / Spring Data
        // connectors expose it; we trust it as the authoritative upper
        // bound on the server's result set. A connector that omits it
        // falls back to the empty-batch heuristic.
        if pages == 0 {
            total_elements = resp
                .get("totalElements")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
        }
        // Client-side date cutover: count records whose delivery (read
        // via the spec's map.delivery path, then parsed by the same
        // rules the validator uses) is in the requested window. Records
        // outside the window are dropped - they would have failed
        // bucketing anyway.
        let page_in_window = super::paging::keep_records(batch, &spec.map, from_epoch);
        let page_kept = page_in_window.len();
        in_window += page_kept;
        total_pulled += n;
        records.extend(page_in_window);
        if !cfg.agent {
            eprintln!(
                "  {} -> {} record(s), {} in window, total {} in window",
                label.as_deref().unwrap_or("request"),
                n,
                page_kept,
                in_window
            );
        }
        pages += 1;
        // Termination: defer to `should_stop` so the policy is testable
        // without a network. See the helper for the priority order.
        if super::paging::should_stop(
            total_pulled,
            total_elements,
            in_window,
            page_kept,
            n,
            spec.paging.size as usize,
            spec.paging.mode,
        ) {
            break;
        }
        if pages >= MAX_PAGES {
            return Err(Error::Config(format!(
                "{} pages and still going - the endpoint is likely ignoring paging.param \"{}\" AND omitting totalElements; check the spec",
                pages, spec.paging.param
            )));
        }
        page += 1;
    }

    let report = validate_and_write(cfg, spec.name, &records, &spec.map, pages)?;
    if !cfg.agent {
        let dropped = if report.dropped_no_ad_id > 0 {
            format!(
                ", {} non-ad lead(s) dropped (no ad id)",
                report.dropped_no_ad_id
            )
        } else {
            String::new()
        };
        eprintln!(
            "  done: {} ticket(s) over {} page(s) in {}{}{}",
            report.tickets,
            report.pages,
            fmt_elapsed(start.elapsed()),
            total_elements
                .map(|t| format!(" (of {} total)", t))
                .unwrap_or_default(),
            dropped
        );
    }
    Ok(report)
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
    let (tickets, dropped_no_ad_id) = source::transform(records, map);
    let crm = Value::Array(tickets);
    // Zero tickets never reach disk: an empty export means a wrong date
    // window, a bad map.root, or a broken pull - writing [] would
    // replace a good crm.json and turn /brief's CRM columns into
    // silent zeros (the exact misinformation mod.rs warns about).
    if crm.as_array().map(Vec::is_empty).unwrap_or(true) {
        return Err(Error::Config(format!(
            "{}: 0 tickets after transform ({} record(s) dropped for having \
             no ad id) - check the spec's date window, map.root, and map.ad_id",
            name, dropped_no_ad_id
        )));
    }
    let check = super::check_with_workspace(cfg, &crm);
    let path = if check.passed() {
        // Never create an ads-free "latest" snapshot: writing crm.json
        // into an empty today-dir would make /brief read zero spend for
        // everything. Join today's dir only if ads are already there,
        // else the newest snapshot that has them.
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        let snap_root = cfg.history_dir().join("snap");
        let date = if snap_root.join(&today).join("ads_daily.json").is_file() {
            today
        } else {
            crate::snapshot::list_dates(&snap_root)?
                .into_iter()
                .rev()
                .find(|d| snap_root.join(d).join("ads_daily.json").is_file())
                .ok_or_else(|| {
                    Error::Config(
                        "no ads snapshot to attach CRM data to - run `moneyball fetch` (or /fetch) first"
                            .into(),
                    )
                })?
        };
        let written = write_crm_json(cfg, &date, &crm)?;
        // Organic contacts ride along for the re-inquiry check (lead
        // segmentation) - only when the spec maps contact fields.
        let contacts = source::organic_contacts(records, map);
        if !contacts.is_empty() {
            write_contacts(cfg, &date, contacts)?;
        }
        Some(written)
    } else {
        None
    };
    Ok(CrmFetchReport {
        name,
        tickets: crm.as_array().map(Vec::len).unwrap_or(0),
        pages,
        dropped_no_ad_id,
        path,
        check,
    })
}

/// One-page probe used by `crm connect` to grab a sample response before
/// any spec exists. No paging params; date templates resolve to the
/// trailing 28 days.
pub fn probe(request: &source::RequestSpec) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Config(format!("client: {}", e)))?;
    let today = Local::now().date_naive();
    let vars: HashMap<&str, String> = HashMap::from([
        (
            "from_date",
            (today - Duration::days(28)).format("%Y-%m-%d").to_string(),
        ),
        ("to_date", today.format("%Y-%m-%d").to_string()),
        ("page", "1".into()),
        ("page_size", "50".into()),
    ]);
    request_page(
        &client,
        request,
        &source::PagingSpec::default(),
        &vars,
        &Progress {
            agent: false,
            label: Some("probe"),
        },
    )
}

/// One HTTP call with refs resolved and templates expanded.
fn request_page(
    client: &reqwest::blocking::Client,
    request: &source::RequestSpec,
    paging: &source::PagingSpec,
    vars: &HashMap<&str, String>,
    progress: &Progress<'_>,
) -> Result<Value> {
    let url = source::expand(&request.url, vars);
    let method = request.method.to_uppercase();
    let mut req = match method.as_str() {
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
    if !progress.agent {
        if let Some(label) = progress.label {
            eprintln!("  {}: {} {}", label, method, url);
        } else {
            eprintln!("  request: {} {}", method, url);
        }
    }
    let req_start = Instant::now();
    // without_url(): the reqwest error's URL carries the resolved query
    // string - which may embed secrets - and must never reach the screen.
    let resp = req
        .send()
        .map_err(|e| Error::Config(format!("CRM request failed: {}", e.without_url())))?;
    let status = resp.status();
    // Read raw bytes so we can report the body size; the JSON parse that
    // follows still surfaces structural errors.
    let bytes = resp
        .bytes()
        .map_err(|e| Error::Config(format!("CRM response read failed: {}", e)))?;
    if !progress.agent {
        eprintln!(
            "    -> {} {} ({} bytes) in {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            bytes.len(),
            fmt_elapsed(req_start.elapsed())
        );
    }
    // Status BEFORE parse: auth failures often carry an empty body, and
    // "response is not JSON: EOF" buries the actual problem (found live
    // 2026-08-08 when the LeadZump token expired).
    if !status.is_success() {
        return Err(status_error(status, &bytes));
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| Error::Config(format!("CRM response is not JSON: {}", e)))?;
    Ok(body)
}

/// Non-2xx -> an actionable error. 401/403 name the fix (expired token
/// is the common case); everything else carries a truncated body
/// snippet. Bodies may echo request details - never include headers.
fn status_error(status: reqwest::StatusCode, bytes: &[u8]) -> Error {
    let snippet = crate::text::truncate_marked(String::from_utf8_lossy(bytes).trim(), 200, "...");
    let tail = if snippet.is_empty() {
        String::new()
    } else {
        format!(": {}", snippet)
    };
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        Error::Secrets(format!(
            "CRM rejected the credentials (HTTP {}){} - the stored token has likely \
             expired. Store a fresh one with: moneyball crm secret <name> \
             (the <name> is what crm.toml references as secret:<name>)",
            status.as_u16(),
            tail
        ))
    } else {
        Error::Config(format!("CRM returned {}{}", status, tail))
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

/// `crm_contacts.json` (0600 - raw PII) next to crm.json.
fn write_contacts(cfg: &AppConfig, date: &str, rows: Vec<crate::schema::ContactRow>) -> Result<()> {
    let file = crate::schema::CrmContactsFile {
        schema: crate::schema::CRM_CONTACTS_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        rows,
    };
    let dir = cfg.history_dir().join("snap").join(date);
    let path = dir.join("crm_contacts.json");
    let tmp = dir.join("crm_contacts.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&file)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn auth_failure_is_actionable_even_with_empty_body() {
        let e = status_error(reqwest::StatusCode::UNAUTHORIZED, b"");
        let msg = e.to_string();
        assert!(
            msg.contains("401") && msg.contains("moneyball crm secret"),
            "{}",
            msg
        );
        assert!(!msg.contains("not JSON"), "never the parse error: {}", msg);
    }

    #[test]
    fn other_statuses_carry_a_truncated_snippet() {
        let e = status_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "boom".repeat(200).as_bytes(),
        );
        let msg = e.to_string();
        assert!(
            msg.contains("500") && msg.contains("...") && msg.len() < 400,
            "{}",
            msg
        );
    }
}
