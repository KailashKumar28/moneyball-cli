//! Snapshot reader - loads `<snap>/<date>/*.json` into typed structures.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const SNAPSHOT_FILES: &[&str] = &[
    "ads_daily",
    "adsets",
    "creatives",
    "crm",
    "regions",
    "changes",
    "campaigns",
];

/// A single ad's daily metrics. Mirrors the Fincity fetch shape; other
/// users adapt their fetcher to emit this schema (see schema.md).
///
/// Numeric fields are kept as `String` to preserve Meta's native string
/// representation; coerce to numbers via the `*_num()` helpers. All fields
/// default to empty string so partial rows (e.g. video creatives) still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdsDailyRow {
    #[serde(default)]
    pub campaign_id: String,
    #[serde(default)]
    pub campaign_name: String,
    #[serde(default)]
    pub adset_id: String,
    #[serde(default)]
    pub adset_name: String,
    #[serde(default)]
    pub ad_id: String,
    #[serde(default)]
    pub ad_name: String,
    #[serde(default)]
    pub spend: String,
    #[serde(default)]
    pub impressions: String,
    #[serde(default)]
    pub reach: String,
    #[serde(default)]
    pub frequency: String,
    #[serde(default)]
    pub clicks: String,
    #[serde(default)]
    pub inline_link_clicks: String,
    #[serde(default)]
    pub inline_link_click_ctr: String,
    #[serde(default)]
    pub cost_per_inline_link_click: String,
    #[serde(default)]
    pub cpc: String,
    #[serde(default)]
    pub ctr: String,
    #[serde(default)]
    pub cpm: String,
    #[serde(default)]
    pub actions: Vec<serde_json::Value>,
    #[serde(default)]
    pub date_start: String,
    #[serde(default)]
    pub date_stop: String,
    /// Workspace tag - set by the fetcher during snapshot generation.
    /// Required for product filtering.
    #[serde(default)]
    pub _product: String,
}

impl AdsDailyRow {
    pub fn spend_num(&self) -> f64 {
        parse_f64(&self.spend)
    }
    pub fn impressions_num(&self) -> u64 {
        parse_u64(&self.impressions)
    }
    pub fn clicks_num(&self) -> u64 {
        parse_u64(&self.inline_link_clicks)
    }
}

fn parse_f64(s: &str) -> f64 {
    s.trim().parse::<f64>().unwrap_or(0.0)
}
fn parse_u64(s: &str) -> u64 {
    s.trim().parse::<u64>().unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    pub date: String,
    pub ads_daily: Vec<AdsDailyRow>,
    pub adsets: serde_json::Value,
    /// Typed creative identity rows (schema.rs), when the snapshot has
    /// a readable creatives.json. None = predates capture or unknown
    /// major - the report falls back to one-ad-per-card grouping.
    pub creatives: Option<crate::schema::CreativesFile>,
    pub crm: serde_json::Value,
    pub regions: serde_json::Value,
    pub changes: serde_json::Value,
    pub campaigns: serde_json::Value,
}

/// The creatives.json loader rule (ARCHITECTURE section 2): object =>
/// versioned envelope, accept major 1 only; bare array => grandfathered
/// v0 rows (external pipelines), wrapped with defaulted metadata. Rows
/// that fail to parse are skipped, never fatal (same forward-compat
/// stance as session replay).
fn parse_creatives(v: serde_json::Value) -> Option<crate::schema::CreativesFile> {
    use crate::schema::{CreativeRow, CreativesFile, CREATIVES_SCHEMA};
    match v {
        serde_json::Value::Object(_) => {
            let schema_ok = v
                .get("schema")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == CREATIVES_SCHEMA || s.starts_with("moneyball.creatives/1"));
            if !schema_ok {
                return None; // unknown major: absent, not garbage
            }
            serde_json::from_value(v).ok()
        }
        serde_json::Value::Array(rows) => Some(CreativesFile {
            schema: "moneyball.creatives/0".into(),
            fetched_at: String::new(),
            rows: rows
                .into_iter()
                .filter_map(|r| serde_json::from_value::<CreativeRow>(r).ok())
                .collect(),
        }),
        _ => None,
    }
}

pub fn load(snap_path: &Path) -> Result<Snapshot> {
    let date = snap_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Config(format!("bad snapshot path: {}", snap_path.display())))?
        .to_string();
    // The dir name IS the snapshot date. External pipelines write these
    // dirs, so stray names (backups, scratch) are expected input - fail
    // here so every constructed Snapshot carries a parseable date
    // (brief/funnel rely on that).
    if chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(Error::Config(format!(
            "snapshot dir name is not a YYYY-MM-DD date: {}",
            snap_path.display()
        )));
    }
    let mut ads_daily = Vec::new();
    let mut adsets = serde_json::Value::Array(vec![]);
    let mut creatives = None;
    let mut crm = serde_json::Value::Object(Default::default());
    let mut regions = serde_json::Value::Array(vec![]);
    let mut changes = serde_json::Value::Array(vec![]);
    let mut campaigns = serde_json::Value::Array(vec![]);
    for name in SNAPSHOT_FILES {
        let f = snap_path.join(format!("{}.json", name));
        if !f.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&f)?;
        let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| Error::InvalidJson {
            date: date.clone(),
            file: (*name).into(),
            source: e,
        })?;
        match *name {
            "ads_daily" => ads_daily = serde_json::from_value(v)?,
            "adsets" => adsets = v,
            "creatives" => creatives = parse_creatives(v),
            "crm" => crm = v,
            "regions" => regions = v,
            "changes" => changes = v,
            "campaigns" => campaigns = v,
            _ => unreachable!(),
        }
    }
    Ok(Snapshot {
        path: snap_path.to_path_buf(),
        date,
        ads_daily,
        adsets,
        creatives,
        crm,
        regions,
        changes,
        campaigns,
    })
}

pub fn list_dates(snap_root: &Path) -> Result<Vec<String>> {
    if !snap_root.is_dir() {
        return Ok(vec![]);
    }
    let mut out: Vec<String> = std::fs::read_dir(snap_root)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.is_dir() {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| s.len() == 10 && s.chars().nth(4) == Some('-'))
                    .map(String::from)
            } else {
                None
            }
        })
        .collect();
    out.sort();
    Ok(out)
}
