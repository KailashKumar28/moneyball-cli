//! Everything a front-end shows about the workspace's data, loaded in
//! one go: the brief, the CRM sentence and the data health. File reads
//! only. The TUI calls it at start, on /brief and after a refresh.

use serde::Serialize;

use super::ProductRowsAndFeasibility;
use crate::config::AppConfig;
use crate::crm::status::CrmHealth;
use crate::error::Error;
use crate::health::Health;

/// One loading of the snapshot `cfg` points at (`cfg.date`, else the
/// latest).
#[derive(Serialize)]
pub struct Loaded {
    /// Always there: with no snapshot it says so (State::NoData).
    pub health: Health,
    /// None when there is no snapshot, or it would not load.
    pub brief: Option<ProductRowsAndFeasibility>,
    /// The CRM sentence when the snapshot's CRM data is not fresh.
    pub crm_note: Option<String>,
    /// Why a snapshot that exists would not load. A missing snapshot is
    /// the normal first-run state, not a fault: `health` speaks for it.
    pub fault: Option<Error>,
}

/// Load the snapshot and compute what the front-end shows from it.
/// Never fails: every outcome is a `Loaded`.
pub fn load(cfg: &AppConfig) -> Loaded {
    let without = |fault| Loaded {
        health: Health::of(cfg, None),
        brief: None,
        crm_note: None,
        fault,
    };
    let Ok(path) = cfg.snap_for(cfg.date.as_deref()) else {
        return without(None);
    };
    let snap = match crate::snapshot::load(&path) {
        Ok(snap) => snap,
        Err(e) => return without(Some(e)),
    };
    let history = super::load_history(&cfg.history_dir().join("scoreboard.csv"));
    Loaded {
        health: Health::of(cfg, Some(&snap)),
        brief: Some(super::compute(&snap, cfg, &history)),
        crm_note: CrmHealth::of(cfg, &snap).note(),
        fault: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::State;

    fn workspace(name: &str) -> AppConfig {
        let root = std::env::temp_dir().join(format!("mb-load-{}-{}", name, std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join(".moneyball")).unwrap();
        std::fs::write(
            root.join(".moneyball").join("config.json"),
            r#"{"products": [{"name": "P", "ad_account": "act_1"}]}"#,
        )
        .unwrap();
        crate::workspace::open(&root, None).unwrap()
    }

    #[test]
    fn no_snapshot_is_not_a_fault() {
        let cfg = workspace("none");
        let l = load(&cfg);
        assert!(l.brief.is_none() && l.fault.is_none() && l.crm_note.is_none());
        assert_eq!(l.health.state, State::NoData);
        std::fs::remove_dir_all(&cfg.data_root).ok();
    }

    #[test]
    fn a_snapshot_that_will_not_load_is_a_fault_with_health_still_there() {
        let cfg = workspace("broken");
        let snap = cfg.snap_dir().join("2026-11-10");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("ads_daily.json"), "{ not json").unwrap();
        let l = load(&cfg);
        assert!(l.brief.is_none());
        let fault = l.fault.expect("the load error is kept");
        assert_eq!(fault.kind(), "snapshot");
        assert!(fault.to_string().contains("ads_daily"), "{}", fault);
        assert_eq!(l.health.state, State::NoData);
        std::fs::remove_dir_all(&cfg.data_root).ok();
    }

    #[test]
    fn a_snapshot_loads_into_brief_health_and_the_crm_sentence() {
        let cfg = workspace("ok");
        let snap = cfg.snap_dir().join("2026-11-10");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("ads_daily.json"), "[]").unwrap();
        let l = load(&cfg);
        let brief = l.brief.as_ref().expect("the brief is computed");
        assert_eq!(brief.snapshot_date, "2026-11-10");
        assert!(!brief.has_crm);
        assert!(l.fault.is_none());
        assert_eq!(l.health.snap_date, "2026-11-10".parse().ok());
        assert!(l
            .crm_note
            .as_ref()
            .is_some_and(|n| n.contains("no CRM connected")));
        // No CRM data crosses a process boundary as null, never as zero.
        let json = serde_json::to_value(&l).unwrap();
        assert!(json["brief"]["rows"][0]["q7d"].is_null(), "{}", json);
        assert!(json["fault"].is_null());
        std::fs::remove_dir_all(&cfg.data_root).ok();
    }
}
