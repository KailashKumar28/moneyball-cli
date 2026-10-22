//! Offline CRM ingestion: a CSV export, and the tail every pull shares -
//! transform, validate against the contract, write crm.json only on a
//! pass. Split from crm/fetch.rs (size cap); no network here.

use serde_json::Value;

use super::fetch::{spec_path, CrmFetchReport};
use super::source::{self, SourceSpec};
use crate::config::AppConfig;
use crate::error::{Error, Result};

/// Import a CSV export through the same spec map + validation pipeline.
/// Offline counterpart to fetch_crm - map paths are CSV column names.
pub fn import_csv(cfg: &AppConfig, file: &std::path::Path) -> Result<CrmFetchReport> {
    let spec = load_spec(cfg)?;
    let raw = std::fs::read_to_string(file)
        .map_err(|e| Error::Config(format!("cannot read {}: {}", file.display(), e)))?;
    let records = source::csv_records(&raw)?;
    let date = super::status::target_snapshot(cfg)?.ok_or_else(|| {
        Error::Config("no ads snapshot to attach CRM data to - run `moneyball fetch` first".into())
    })?;
    validate_and_write(cfg, &date, spec.name, &records, &spec.map, 0)
}

pub(super) fn load_spec(cfg: &AppConfig) -> Result<SourceSpec> {
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
pub(super) fn validate_and_write(
    cfg: &AppConfig,
    date: &str,
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
        // `date` is status::target_snapshot: never an ads-free dir.
        let written = super::status::write_crm_json(cfg, date, &crm)?;
        // Organic contacts ride along for the re-inquiry check (lead
        // segmentation) - only when the spec maps contact fields.
        let contacts = source::organic_contacts(records, map);
        if !contacts.is_empty() {
            super::status::write_contacts(cfg, date, contacts)?;
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
