//! `brief` - 7-day portfolio summary per product + feasibility math.
//!
//! Reads a snapshot, aggregates per-product totals across the trailing
//! 7-day window, and emits a fixed-width table followed by feasibility
//! math + setup-debt count. Reimplements mb.py:cmd_scoreboard natively.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::Deserialize;

use crate::config::{AppConfig, CrmConfig};
use crate::error::Result;
use crate::snapshot::{Snapshot, SNAPSHOT_FILES};

const IST_OFFSET_HOURS: i64 = 5;
const IST_OFFSET_MINUTES: i64 = 30;
const LAG_HOURS: i64 = 72;
const QUALIFIED_PLUS: &[&str] = &["Contactable", "Visit", "Revisit", "Booking"];
const VISIT_PLUS: &[&str] = &["Visit", "Revisit", "Booking"];

#[derive(Debug, Clone)]
pub struct ProductRow {
    pub product: String,
    pub spend_per_day: u64,
    pub m7d: u64,
    pub l7d: u64,
    pub q7d: u64,
    pub q_per_day: f64,
    pub rs_per_q: Option<u64>,
    pub l_to_q: Option<f64>,
    pub goal: f64,
    pub gap: f64,
    pub trend: String, // space-separated q/day from history
}

#[derive(Debug, Clone)]
pub struct Feasibility {
    pub tot_q_per_day: f64,
    pub tot_spend_per_day: u64,
    pub tot_goal_per_day: f64,
    pub cur_rpq: u64,
    pub required_at_cur: Option<u64>,
    pub required_at_best: Option<u64>,
    pub best_rpq: Option<u64>,
    pub open_debt: Vec<String>,
}

pub fn compute(snap: &Snapshot, cfg: &AppConfig, history: &[HistoryRow]) -> ProductRowsAndFeasibility {
    let snap_date = NaiveDate::parse_from_str(&snap.date, "%Y-%m-%d")
        .expect("snapshot date is YYYY-MM-DD");
    let d1 = snap_date - Duration::days(1); // yesterday
    let d0 = d1 - Duration::days(6); // 7-day window: d0..=d1

    // IST epoch windows for CRM ticket filtering.
    let d1_ist = ist_midnight_epoch(snap_date); // start of snap day in IST
    let d0_ist = d1_ist - 7 * 86400;
    let lag_cut_ist = d1_ist - LAG_HOURS * 3600;

    let mut rows = Vec::new();
    let mut best_rpq: Option<u64> = None;
    let workspace = cfg.workspace.as_ref().expect("brief called without workspace");
    let target_rpq = workspace.target_rs_per_q;
    for prod in &workspace.products {
        let r = compute_product(
            prod.name.as_str(),
            snap,
            d0,
            d1,
            d0_ist,
            d1_ist,
            lag_cut_ist,
            workspace.goal_for(&prod.name),
        );
        if let Some(rpq) = r.rs_per_q {
            best_rpq = Some(match best_rpq { Some(b) if b <= rpq => b, _ => rpq });
        }
        rows.push(r);
    }

    let feasibility = compute_feasibility(&rows, cfg, best_rpq);
    let trend_rows = trend_rows_for(history, &rows);
    apply_trends(&mut rows, trend_rows);
    let _ = target_rpq;

    ProductRowsAndFeasibility { rows, feasibility }
}

fn compute_product(
    product: &str,
    snap: &Snapshot,
    d0: NaiveDate,
    d1: NaiveDate,
    d0_ist: i64,
    d1_ist: i64,
    lag_cut_ist: i64,
    goal: f64,
) -> ProductRow {
    // Per-campaign buckets.
    let mut spend: HashMap<String, f64> = HashMap::new();
    let mut m: HashMap<String, u64> = HashMap::new();
    // ad_id -> campaign_id (for CRM join).
    let mut ad_to_campaign: HashMap<String, String> = HashMap::new();

    let d0s = d0.format("%Y-%m-%d").to_string();
    let d1s = d1.format("%Y-%m-%d").to_string();

    for r in &snap.ads_daily {
        if r._product != product { continue; }
        if r.date_start < d0s || r.date_start > d1s { continue; }
        let cid = r.campaign_id.clone();
        *spend.entry(cid.clone()).or_default() += r.spend_num();
        *m.entry(cid.clone()).or_default() += count_m_leads(&r.actions);
        ad_to_campaign.entry(r.ad_id.clone()).or_insert(cid);
    }

    // CRM: bucket l/q/v/b per campaign (via ad_id -> campaign_id join).
    let mut l: HashMap<String, u64> = HashMap::new();
    let mut q: HashMap<String, u64> = HashMap::new();
    let mut v: HashMap<String, u64> = HashMap::new();
    let mut b: HashMap<String, u64> = HashMap::new();

    for_each_crm_ticket(snap, |ticket, delivery_ep| {
        if delivery_ep < d0_ist || delivery_ep >= d1_ist { return; }
        let aid = ticket_ad_id(ticket).unwrap_or_default();
        let cid = match ad_to_campaign.get(&aid) {
            Some(c) => c.clone(),
            None => return,
        };
        let stage = ticket_stage(ticket);
        let funnel = ticket_funnel(ticket);
        *l.entry(cid.clone()).or_default() += 1;
        if QUALIFIED_PLUS.contains(&stage.as_str()) {
            *q.entry(cid.clone()).or_default() += 1;
        }
        if VISIT_PLUS.contains(&stage.as_str()) {
            *v.entry(cid.clone()).or_default() += 1;
        }
        if funnel == "WON" || stage == "Booking" {
            *b.entry(cid.clone()).or_default() += 1;
        }
        let _ = lag_cut_ist; // mb.py tracks but doesn't surface at brief level
    });

    let total_spend: f64 = spend.values().sum();
    let total_m: u64 = m.values().sum();
    let total_l: u64 = l.values().sum();
    let total_q: u64 = q.values().sum();
    let spend_per_day = (total_spend / 7.0).round() as u64;
    let rs_per_q = if total_q > 0 { Some((total_spend / total_q as f64).round() as u64) } else { None };
    let l_to_q = if total_l > 0 { Some((total_q as f64 / total_l as f64) * 100.0) } else { None };
    let qpd = total_q as f64 / 7.0;

    ProductRow {
        product: product.into(),
        spend_per_day,
        m7d: total_m,
        l7d: total_l,
        q7d: total_q,
        q_per_day: round2(qpd),
        rs_per_q,
        l_to_q: l_to_q.map(round1),
        goal,
        gap: round1(goal - qpd),
        trend: "-".into(),
    }
}

fn compute_feasibility(rows: &[ProductRow], cfg: &AppConfig, best_rpq: Option<u64>) -> Feasibility {
    let tot_q: f64 = rows.iter().map(|r| r.q7d as f64).sum();
    let tot_q_per_day = tot_q / 7.0;
    let tot_spend_per_day: u64 = rows.iter().map(|r| r.spend_per_day).sum();
    let tot_goal_per_day: f64 = rows.iter().map(|r| r.goal).sum();
    let cur_rpq = if tot_q > 0.0 { (tot_spend_per_day as f64 / tot_q_per_day).round() as u64 } else { 0 };
    let required_at_cur = if tot_q_per_day > 0.0 { Some(((tot_goal_per_day * cur_rpq as f64).round()) as u64) } else { None };
    let required_at_best = best_rpq.map(|b| ((tot_goal_per_day * b as f64).round()) as u64);
    let open_debt = load_open_debt(&cfg.state_dir().join("setup_debt.json"));
    Feasibility {
        tot_q_per_day,
        tot_spend_per_day,
        tot_goal_per_day,
        cur_rpq,
        required_at_cur,
        required_at_best,
        best_rpq,
        open_debt,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryRow {
    pub product: String,
    pub qualified: f64,
}

fn trend_rows_for(history: &[HistoryRow], rows: &[ProductRow]) -> HashMap<String, Vec<f64>> {
    let mut out: HashMap<String, Vec<f64>> = HashMap::new();
    for r in rows {
        let h: Vec<f64> = history.iter().rev()
            .filter(|h| h.product == r.product)
            .take(7)
            .map(|h| h.qualified)
            .collect();
        // history is most-recent-first; we want oldest->newest like mb.py
        let mut v = h;
        v.reverse();
        out.insert(r.product.clone(), v);
    }
    out
}

fn apply_trends(rows: &mut [ProductRow], trends: HashMap<String, Vec<f64>>) {
    for r in rows.iter_mut() {
        if let Some(v) = trends.get(&r.product) {
            r.trend = if v.is_empty() { "-".into() } else {
                v.iter().map(|q| format!("{:.0}", q)).collect::<Vec<_>>().join(" ")
            };
        }
    }
}

fn load_open_debt(path: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { return vec![] };
    v.get("items")
        .and_then(|x| x.as_object())
        .map(|obj| {
            obj.iter()
                .filter(|(_, v)| v.get("status").and_then(|s| s.as_str()) != Some("cleared"))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

// ---------- CRM iteration ----------

fn for_each_crm_ticket<F: FnMut(&serde_json::Map<String, serde_json::Value>, i64)>(snap: &Snapshot, mut f: F) {
    let v = &snap.crm;
    match v {
        serde_json::Value::Object(map) => {
            // {campaign_id: {tickets: [...]}}
            for (_cid, blob) in map {
                let Some(blob) = blob.as_object() else { continue };
                let Some(tickets) = blob.get("tickets").and_then(|t| t.as_array()) else { continue };
                for t in tickets {
                    let Some(obj) = t.as_object() else { continue };
                    let ep = ticket_delivery_epoch(obj).unwrap_or(i64::MIN);
                    f(obj, ep);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for t in arr {
                let Some(obj) = t.as_object() else { continue };
                let ep = ticket_delivery_epoch(obj).unwrap_or(i64::MIN);
                f(obj, ep);
            }
        }
        _ => {}
    }
}

fn ticket_delivery_epoch(t: &serde_json::Map<String, serde_json::Value>) -> Option<i64> {
    let delivery = t.get("delivery")?;
    parse_epoch(delivery)
}

fn parse_epoch(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => {
            if let Ok(i) = s.parse::<i64>() { return Some(i); }
            // ISO string - parse as UTC then shift to IST for date-bucketing
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.timestamp());
            }
            // Try naive ISO + treat as UTC
            for fmt in &["%Y-%m-%dT%H:%M:%S%z", "%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%dT%H:%M:%S"] {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                    return Some(Utc.from_utc_datetime(&dt).timestamp());
                }
            }
            None
        }
        _ => None,
    }
}

fn ticket_ad_id(t: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if let Some(s) = t.get("ad_id").and_then(|v| v.as_str()) { return Some(s.into()); }
    if let Some(obj) = t.get("adId").and_then(|v| v.as_object()) {
        if let Some(s) = obj.get("adId").and_then(|v| v.as_str()) { return Some(s.into()); }
    }
    None
}

fn ticket_stage(t: &serde_json::Map<String, serde_json::Value>) -> String {
    t.get("stage").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn ticket_funnel(t: &serde_json::Map<String, serde_json::Value>) -> String {
    t.get("funnel").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

// ---------- m-leads from Meta actions ----------

/// Mirrors mb.py `leads_from_actions`: try a fixed list of canonical lead
/// action types (first match wins, avoids double-counting), else fall back
/// to any action_type whose key contains "lead".
const LEAD_TYPES: &[&str] = &[
    "onsite_conversion.lead_grouped",
    "leadgen_grouped",
    "leadgen.other",
    "lead",
    "offsite_conversion.fb_pixel_lead",
];

fn count_m_leads(actions: &[serde_json::Value]) -> u64 {
    if actions.is_empty() { return 0; }
    let map: std::collections::HashMap<&str, &str> = actions.iter().filter_map(|a| {
        let at = a.get("action_type").and_then(|v| v.as_str())?;
        let val = a.get("value").and_then(|v| v.as_str()).unwrap_or("0");
        Some((at, val))
    }).collect();
    for t in LEAD_TYPES {
        if let Some(v) = map.get(t) {
            return v.trim().parse::<f64>().unwrap_or(0.0).round() as u64;
        }
    }
    // Fallback: any action_type containing "lead".
    let s: f64 = map.iter()
        .filter(|(k, _)| k.contains("lead"))
        .filter_map(|(_, v)| v.trim().parse::<f64>().ok())
        .sum();
    s.round() as u64
}

// ---------- IST epoch math ----------

fn ist_midnight_epoch(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).unwrap();
    let utc = naive - Duration::hours(IST_OFFSET_HOURS) - Duration::minutes(IST_OFFSET_MINUTES);
    Utc.from_utc_datetime(&utc).timestamp()
}

// ---------- number helpers ----------

fn round1(x: f64) -> f64 { (x * 10.0).round() / 10.0 }
fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }

// ---------- public runner ----------

pub struct ProductRowsAndFeasibility {
    pub rows: Vec<ProductRow>,
    pub feasibility: Feasibility,
}

pub fn run(cfg: &AppConfig, date: Option<&str>) -> Result<()> {
    let snap_path = cfg.snap_for(date)?;
    let snap = crate::snapshot::load(&snap_path)?;
    let history = load_history(&cfg.history_dir().join("scoreboard.csv"));
    let result = compute(&snap, cfg, &history);
    print_brief(&result, &snap.date);
    Ok(())
}

pub fn load_history(path: &Path) -> Vec<HistoryRow> {
    let Ok(raw) = std::fs::read_to_string(path) else { return vec![] };
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_reader(raw.as_bytes());
    let mut out = Vec::new();
    for row in rdr.records().flatten() {
        let product = row.get(0).unwrap_or("").to_string();
        let qualified: f64 = row.get(2).unwrap_or("0").parse().unwrap_or(0.0);
        if !product.is_empty() {
            out.push(HistoryRow { product, qualified });
        }
    }
    out
}

pub fn print_brief(r: &ProductRowsAndFeasibility, snap_date: &str) {
    println!("BRIEF  snapshot {}  (7d window; config.json goals)", snap_date);
    println!("{}", format_brief_table(&r.rows));
    let f = &r.feasibility;
    println!();
    println!("FEASIBILITY  portfolio {:.1} q/day at \u{20B9}{}/day = \u{20B9}{}/qualified \u{00B7} goal {:.0}/day",
        f.tot_q_per_day, comma(f.tot_spend_per_day as i64), comma(f.cur_rpq as i64), f.tot_goal_per_day);
    if let Some(req) = f.required_at_cur {
        println!("  required spend at CURRENT efficiency : \u{20B9}{}/day ({:.1}x today)",
            comma(req as i64), req as f64 / f.tot_spend_per_day.max(1) as f64);
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        println!("  required spend at BEST-OBSERVED \u{20B9}{}/q : \u{20B9}{}/day ({:.1}x today)",
            comma(b as i64), comma(req as i64), req as f64 / f.tot_spend_per_day.max(1) as f64);
    }
    let n = f.open_debt.len();
    let suffix = if f.open_debt.is_empty() { String::new() } else { format!(" ({})", f.open_debt.join(", ")) };
    println!("  open setup debt: {}{}", n, suffix);
}

fn comma(n: i64) -> String {
    let s = n.abs().to_string();
    let negative = n < 0;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(b','); }
        out.push(b);
    }
    out.reverse();
    let mut s = String::from_utf8(out).unwrap();
    if negative { s.insert(0, '-'); }
    s
}

pub fn format_brief_table(rows: &[ProductRow]) -> String {
    let cols: Vec<(&str, Box<dyn Fn(&ProductRow) -> String>)> = vec![
        ("product", Box::new(|r| r.product.clone())),
        ("spend_per_day", Box::new(|r| r.spend_per_day.to_string())),
        ("m7d", Box::new(|r| r.m7d.to_string())),
        ("l7d", Box::new(|r| r.l7d.to_string())),
        ("q7d", Box::new(|r| r.q7d.to_string())),
        ("q_per_day", Box::new(|r| fmt_f64(r.q_per_day, 2))),
        ("rs_per_q", Box::new(|r| r.rs_per_q.map(|x| x.to_string()).unwrap_or_else(|| "-".into()))),
        ("l_to_q", Box::new(|r| r.l_to_q.map(|x| format!("{:.1}", x)).unwrap_or_else(|| "-".into()))),
        ("goal", Box::new(|r| fmt_f64(r.goal, 0))),
        ("gap", Box::new(|r| fmt_f64(r.gap, 1))),
        ("q_last7_by_day", Box::new(|r| r.trend.clone())),
    ];
    let headers: Vec<&str> = cols.iter().map(|(h, _)| *h).collect();
    let width = |s: &str| s.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    let mut rendered: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for r in rows {
        let row: Vec<String> = cols.iter().map(|(_, f)| f(r)).collect();
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
        rendered.push(row);
    }
    let mut lines = Vec::new();
    lines.push(headers.iter().enumerate().map(|(i, h)| h.pad_to_width(widths[i])).collect::<Vec<_>>().join("  "));
    lines.push(lines[0].chars().map(|c| if c == ' ' { '-' } else { '-' }).collect());
    for row in rendered {
        lines.push(row.iter().enumerate().map(|(i, c)| c.pad_to_width(widths[i])).collect::<Vec<_>>().join("  "));
    }
    lines.join("\n")
}

fn fmt_f64(x: f64, decimals: usize) -> String {
    let scaled = (x * 10f64.powi(decimals as i32)).round() as i64;
    let sign = if scaled < 0 { "-" } else { "" };
    let abs = scaled.unsigned_abs() as f64 / 10f64.powi(decimals as i32);
    format!("{}{}", sign, abs)
}

trait PadToWidth { fn pad_to_width(&self, w: usize) -> String; }
impl PadToWidth for str {
    fn pad_to_width(&self, w: usize) -> String {
        if self.len() >= w { self.to_string() } else { format!("{}{}", self, " ".repeat(w - self.len())) }
    }
}

// Re-export for callers that need config access
#[allow(unused_imports)]
use CrmConfig as _;