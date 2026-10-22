//! The prediction ledger: what the advisor recommended, the number it
//! said would move, and whether it did. mb.py's rec submit / settle /
//! ledger natively, except that "applied" is read from the ad account's
//! change log and the verdict from the snapshot instead of being typed.
//! Append-only `.moneyball/ledger.jsonl` (moneyball.ledger/1).

use std::io::Write;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate};

use crate::config::AppConfig;
use crate::diagnose::RECENT_DAYS;
use crate::funnel::{self, By, FunnelRow};
use crate::schema::{LedgerEvent, Rec, RecState, LEDGER_SCHEMA};
use crate::snapshot::Snapshot;

mod print;
mod submit;
mod sweep;
pub use print::{run, text};
pub use submit::submit;
pub use sweep::sweep;

/// mb.py's guards: new recs a day, the budget step that restarts Meta's
/// learning, and how far evidence may drift from the data.
const MAX_PER_DAY: usize = 5;
const BUDGET_STEP: f64 = 0.20;
const EVIDENCE_TOLERANCE: f64 = 0.05;
/// A pull holds 28 days, so a later check could not see the change.
const MAX_HORIZON_DAYS: i64 = 28;
/// CRM outcomes of the last 3 days are still coming in (funnel's lag).
const CRM_LAG_DAYS: i64 = funnel::LAG_HOURS / 24;

/// Each lever, and the days its entity is left alone once it is applied.
pub const LEVERS: &[(&str, i64)] = &[
    ("budget", 2),
    ("creative", 7),
    ("targeting", 14),
    ("structure", 14),
    ("form", 7),
    ("crm", 7),
];

/// What a prediction may name, whether it needs CRM data, and whether it
/// is summed over the days (settled as a 7-day rate). Evidence counts
/// must match exactly; money and ratios within 5%.
pub const METRICS: &[(&str, bool, bool)] = &[
    ("spend", false, true),
    ("meta_leads", false, true),
    ("cpl", false, false),
    ("crm_leads", true, true),
    ("qualified", true, true),
    ("cost_per_qualified", true, false),
    ("l_to_q", true, false),
];

/// One recommendation and what has happened to it since.
#[derive(serde::Serialize)]
pub struct Entry {
    pub id: String,
    pub rec: Rec,
    pub state: RecState,
    pub proposed: NaiveDate,
    pub proposed_at: Option<DateTime<FixedOffset>>,
    pub applied: Option<NaiveDate>,
    /// The last event's note: how it was applied, or how it came out.
    pub note: Option<String>,
}

impl Entry {
    pub fn open(&self) -> bool {
        matches!(self.state, RecState::Proposed | RecState::Applied)
    }
    pub fn verify_by(&self) -> Option<NaiveDate> {
        day(&self.rec.prediction.verify_by)
    }
}

/// What a person may mark a rec by hand; the rest the data decides.
pub fn by_hand(s: &str) -> Option<RecState> {
    match s {
        "applied" => Some(RecState::Applied),
        "ignored" => Some(RecState::Ignored),
        _ => None,
    }
}

/// Sweep with the newest snapshot, if there is one: every surface that
/// shows the ledger, and each refresh, calls this first.
pub fn catch_up(cfg: &AppConfig, today: NaiveDate) -> std::io::Result<Vec<LedgerEvent>> {
    match cfg.snap_for(None).and_then(|p| crate::snapshot::load(&p)) {
        Ok(snap) => sweep(cfg, &snap, today),
        Err(_) => Ok(vec![]),
    }
}

pub fn path(cfg: &AppConfig) -> std::path::PathBuf {
    cfg.mb_dir().join("ledger.jsonl")
}

/// Every rec, in the order proposed. A line that does not parse is
/// skipped: nothing is ever rewritten, so it stays on disk to be fixed.
pub fn load(cfg: &AppConfig) -> Vec<Entry> {
    let raw = std::fs::read_to_string(path(cfg)).unwrap_or_default();
    let mut out: Vec<Entry> = Vec::new();
    for e in raw
        .lines()
        .filter_map(|l| serde_json::from_str::<LedgerEvent>(l).ok())
    {
        match (e.event, e.rec, day(&e.date)) {
            (RecState::Proposed, Some(rec), Some(date)) => out.push(Entry {
                id: e.id,
                rec,
                state: RecState::Proposed,
                proposed: date,
                proposed_at: e
                    .at
                    .as_deref()
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok()),
                applied: None,
                note: e.note,
            }),
            (state, _, date) => {
                if let Some(entry) = out.iter_mut().find(|x| x.id == e.id) {
                    if state == RecState::Applied {
                        entry.applied = date;
                    }
                    entry.state = state;
                    entry.note = e.note;
                }
            }
        }
    }
    out
}

fn append(cfg: &AppConfig, events: &[LedgerEvent]) -> std::io::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(cfg.mb_dir())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(cfg))?;
    for e in events {
        writeln!(f, "{}", serde_json::to_string(e)?)?;
    }
    Ok(())
}

fn event(
    id: &str,
    state: RecState,
    date: NaiveDate,
    by: &str,
    note: Option<String>,
) -> LedgerEvent {
    LedgerEvent {
        schema: LEDGER_SCHEMA.into(),
        id: id.into(),
        event: state,
        date: date.format("%Y-%m-%d").to_string(),
        by: by.into(),
        note,
        at: None,
        rec: None,
    }
}

fn day(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn metric(name: &str) -> Option<(bool, bool)> {
    METRICS.iter().find(|m| m.0 == name).map(|m| (m.1, m.2))
}

/// `name` for `rec`'s entity over [d0, d1]; None when it did not deliver
/// then, or the metric needs CRM data the snapshot lacks. Counts come as
/// a 7-day rate, so a prediction made from a 7-day table settles over any
/// number of days.
pub fn measure(
    snap: &Snapshot,
    cfg: &AppConfig,
    rec: &Rec,
    name: &str,
    (d0, d1): (NaiveDate, NaiveDate),
) -> Option<f64> {
    let (crm, summed) = metric(name)?;
    if crm && crate::crm::is_empty(&snap.crm) {
        return None;
    }
    let by = match rec.level.as_str() {
        "campaign" => By::Campaign,
        "adset" => By::Adset,
        "ad" => By::Ad,
        _ => By::Campaign,
    };
    let rows = funnel::compute_between(snap, cfg, &rec.product, (d0, d1), by);
    let row = match rec.level.as_str() {
        "product" => (!rows.is_empty()).then(|| total(&rows))?,
        _ => rows.into_iter().find(|r| r.id == rec.entity_id)?,
    };
    let per = |n: u64| (n > 0).then(|| row.spend as f64 / n as f64);
    let v = match name {
        "spend" => Some(row.spend as f64),
        "meta_leads" => Some(row.m as f64),
        "cpl" => per(row.m),
        "crm_leads" => Some(row.l as f64),
        "qualified" => Some(row.q as f64),
        "cost_per_qualified" => per(row.q),
        _ => (row.l > 0).then(|| row.q as f64 / row.l as f64 * 100.0),
    }?;
    let days = (d1 - d0).num_days() + 1;
    Some(match summed {
        true => v * RECENT_DAYS as f64 / days as f64,
        false => v,
    })
}

fn total(rows: &[FunnelRow]) -> FunnelRow {
    rows.iter().fold(FunnelRow::default(), |mut t, r| {
        t.spend += r.spend;
        t.m += r.m;
        t.l += r.l;
        t.q += r.q;
        t
    })
}

/// The last 7 complete days, as the funnel and diagnose tables show them.
fn week(snap: &Snapshot) -> (NaiveDate, NaiveDate) {
    let d1 = day(&snap.date).expect("snapshot::load validated the date") - Duration::days(1);
    (d1 - Duration::days(RECENT_DAYS - 1), d1)
}

/// A hand-entered state change, if the rec's state allows it.
pub fn mark(
    cfg: &AppConfig,
    id: &str,
    state: RecState,
    note: Option<String>,
    today: NaiveDate,
) -> Result<(), String> {
    let ledger = load(cfg);
    let e = ledger
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("no rec {} in the ledger", id))?;
    let allowed = match e.state {
        RecState::Proposed => &[RecState::Applied, RecState::Ignored, RecState::Expired][..],
        RecState::Applied => &[
            RecState::Confirmed,
            RecState::Refuted,
            RecState::Inconclusive,
        ],
        _ => &[],
    };
    if !allowed.contains(&state) {
        return Err(format!(
            "{} is {}; it cannot become {}",
            id,
            name(e.state),
            name(state)
        ));
    }
    append(cfg, &[event(id, state, today, "you", note)]).map_err(|e| e.to_string())
}

pub fn name(s: RecState) -> &'static str {
    match s {
        RecState::Proposed => "proposed",
        RecState::Applied => "applied",
        RecState::Ignored => "ignored",
        RecState::Expired => "expired",
        RecState::Confirmed => "confirmed",
        RecState::Refuted => "refuted",
        RecState::Inconclusive => "inconclusive",
    }
}

#[cfg(test)]
mod tests;
