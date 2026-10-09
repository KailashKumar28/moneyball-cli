//! Data health: one reading of how fresh the workspace's data is, behind
//! the TUI's context bar and landing (and /status next). File reads only:
//! the snapshot on screen, the newest snapshot holding CRM data, and the
//! run records. No network.

use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate};

use crate::config::AppConfig;
use crate::schema::{CreativeReport, Outcome, RunRecord, StepStatus, CREATIVE_REPORT_SCHEMA};
use crate::snapshot::Snapshot;

/// Data this many days old reads as stale (the brief warns at the same age).
pub const STALE_DAYS: i64 = 2;

/// The bar's state; when several hold, the first listed wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NoData,
    /// Today's refresh failed: nothing new was written.
    Failed,
    /// The newest data is STALE_DAYS or more old.
    Stale,
    /// The snapshot on screen has no CRM data.
    CrmMissing,
    /// The pull that wrote this snapshot missed some products.
    Partial {
        have: u64,
        of: u64,
    },
    Fresh,
}

#[derive(Debug, Clone)]
pub struct Health {
    pub today: NaiveDate,
    /// The snapshot on screen, and the newest one holding CRM data.
    pub snapshot: Option<NaiveDate>,
    pub crm_snapshot: Option<NaiveDate>,
    pub crm_connected: bool,
    /// The run that wrote the snapshot, when one was recorded.
    pub run: Option<RunRecord>,
    /// When the snapshot's Meta data was pulled (its run, else the file).
    pub pulled_at: Option<DateTime<Local>>,
    /// Today's run, when it failed.
    pub failed_today: Option<RunRecord>,
    pub state: State,
}

impl Health {
    pub fn of(cfg: &AppConfig, snap: Option<&Snapshot>) -> Health {
        let today = Local::now().date_naive();
        let day = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
        let snapshot = snap.and_then(|s| day(&s.date));
        let crm_snapshot = match snap {
            Some(s) if !crate::crm::is_empty(&s.crm) => snapshot,
            _ => crate::crm::status::newest_crm_snapshot(cfg).and_then(|d| day(&d)),
        };
        let run = snap.and_then(|s| crate::runs::read(cfg, &s.date));
        let pulled_at = run
            .as_ref()
            .and_then(|r| DateTime::parse_from_rfc3339(&r.finished_at).ok())
            .map(|t| t.with_timezone(&Local))
            .or_else(|| {
                let file = cfg.snap_dir().join(&snap?.date).join("ads_daily.json");
                Some(std::fs::metadata(file).ok()?.modified().ok()?.into())
            });
        let failed_today = crate::runs::read(cfg, &today.format("%Y-%m-%d").to_string())
            .filter(|r| r.outcome == Outcome::Failed);
        Health {
            today,
            snapshot,
            crm_snapshot,
            crm_connected: crate::crm::fetch::spec_path(cfg).is_file(),
            state: state(
                today,
                snapshot,
                crm_snapshot,
                failed_today.is_some(),
                run.as_ref(),
            ),
            run,
            pulled_at,
            failed_today,
        }
    }

    /// Days since the snapshot was pulled.
    pub fn age_days(&self) -> Option<i64> {
        self.snapshot.map(|s| (self.today - s).num_days())
    }

    /// The last day the data covers: the day before its snapshot.
    pub fn data_day(&self) -> Option<NaiveDate> {
        self.snapshot.and_then(|d| d.pred_opt())
    }

    pub fn crm_day(&self) -> Option<NaiveDate> {
        self.crm_snapshot.and_then(|d| d.pred_opt())
    }

    pub fn label(&self) -> String {
        match self.state {
            State::NoData => "no data".into(),
            State::Failed => "refresh failed".into(),
            State::Stale => "stale".into(),
            State::CrmMissing => "CRM missing".into(),
            State::Partial { have, of } => format!("{} of {} products", have, of),
            State::Fresh => "fresh".into(),
        }
    }
}

/// The daily report written from a snapshot: its report.json and the
/// HTML a client opens. None until that snapshot has a daily report.
pub fn daily_report(cfg: &AppConfig, snap_date: &str) -> Option<(CreativeReport, PathBuf)> {
    let dir = cfg.mb_dir().join("reports").join(snap_date);
    let raw = std::fs::read_to_string(dir.join("creative-report.json")).ok()?;
    let report: CreativeReport = serde_json::from_str(&raw).ok()?;
    if report.schema != CREATIVE_REPORT_SCHEMA {
        return None;
    }
    let html = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("-Daily-") && n.ends_with(".html"))
        })?;
    Some((report, html))
}

fn state(
    today: NaiveDate,
    snapshot: Option<NaiveDate>,
    crm_snapshot: Option<NaiveDate>,
    failed_today: bool,
    run: Option<&RunRecord>,
) -> State {
    let Some(snap) = snapshot else {
        return State::NoData;
    };
    if failed_today {
        return State::Failed;
    }
    if (today - snap).num_days() >= STALE_DAYS {
        return State::Stale;
    }
    if crm_snapshot != Some(snap) {
        return State::CrmMissing;
    }
    let meta = run.and_then(|r| r.steps.iter().find(|s| s.name == "meta"));
    match meta {
        Some(s) if s.status == StepStatus::Partial => {
            let have = s.counts.get("products").copied().unwrap_or(0);
            let failed = s.counts.get("products_failed").copied().unwrap_or(0);
            State::Partial {
                have,
                of: have + failed,
            }
        }
        _ => State::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RunStep, Trigger, RUN_SCHEMA};

    fn d(s: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    }

    #[test]
    fn the_most_urgent_state_wins() {
        let today = d("2026-10-09").unwrap();
        let fresh = d("2026-10-09");
        assert_eq!(state(today, None, None, false, None), State::NoData);
        assert_eq!(state(today, fresh, fresh, true, None), State::Failed);
        assert_eq!(
            state(today, d("2026-10-07"), d("2026-08-12"), false, None),
            State::Stale,
            "two days old is stale, whatever CRM says"
        );
        assert_eq!(
            state(today, d("2026-10-08"), fresh, false, None),
            State::CrmMissing,
            "a day old is fine for Meta, but CRM must be in that snapshot"
        );
        assert_eq!(state(today, fresh, fresh, false, None), State::Fresh);

        let run = RunRecord {
            schema: RUN_SCHEMA.into(),
            workspace_id: "ws".into(),
            date: "2026-10-09".into(),
            trigger: Trigger::Manual,
            started_at: String::new(),
            finished_at: String::new(),
            outcome: Outcome::Degraded,
            steps: vec![RunStep {
                name: "meta".into(),
                status: StepStatus::Partial,
                ms: 1,
                counts: [("products".into(), 4), ("products_failed".into(), 1)].into(),
                error_kind: Some("meta_product".into()),
                message: None,
                fix: None,
            }],
            artifacts: Default::default(),
        };
        assert_eq!(
            state(today, fresh, fresh, false, Some(&run)),
            State::Partial { have: 4, of: 5 }
        );
    }
}
