//! Run records on disk: `<workspace>/.moneyball/runs/<date>.json`, one per
//! snapshot day (a second run that day replaces it), and the lock that
//! keeps the TUI and the schedule from refreshing the same workspace at
//! once. No network.

use std::path::PathBuf;

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::schema::RunRecord;

/// Atomic write (temp then rename): a reader never sees half a record.
pub fn write(cfg: &AppConfig, run: &RunRecord) -> Result<PathBuf> {
    let dir = cfg.runs_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", run.date));
    let tmp = dir.join(format!("{}.json.tmp.{}", run.date, std::process::id()));
    let body = serde_json::to_string_pretty(run)
        .map_err(|e| Error::Config(format!("serialize run record: {}", e)))?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// The run record for a snapshot day, if one was written.
pub fn read(cfg: &AppConfig, date: &str) -> Option<RunRecord> {
    let raw = std::fs::read_to_string(cfg.runs_dir().join(format!("{}.json", date))).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Held for a whole refresh; dropping it releases the lock.
pub struct RunLock(#[allow(dead_code)] std::fs::File);

/// One refresh per workspace at a time. Never waits: a second refresh
/// fails at once with a sentence saying one is already running.
pub fn lock(cfg: &AppConfig) -> Result<RunLock> {
    let dir = cfg.state_dir();
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join("refresh.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(RunLock(file)),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::Config(
            "a refresh is already running for this workspace - wait for it to finish".into(),
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;

    fn cfg_at(root: &std::path::Path) -> AppConfig {
        AppConfig {
            data_root: root.to_path_buf(),
            date: None,
            workspace: None,
            agent: true,
        }
    }

    #[test]
    fn record_round_trips_and_the_lock_is_exclusive() {
        let tmp = std::env::temp_dir().join(format!("mb-runs-{}", std::process::id()));
        let cfg = cfg_at(&tmp);
        let run = RunRecord {
            schema: RUN_SCHEMA.into(),
            workspace_id: "ws".into(),
            date: "2026-10-09".into(),
            trigger: Trigger::Manual,
            started_at: "2026-10-09T02:00:00Z".into(),
            finished_at: "2026-10-09T02:01:20Z".into(),
            outcome: Outcome::Degraded,
            steps: vec![RunStep {
                name: "crm".into(),
                status: StepStatus::Failed,
                ms: 1100,
                counts: Default::default(),
                error_kind: Some("crm_auth".into()),
                message: Some("LeadZump refused the token (HTTP 401)".into()),
                fix: Some("moneyball crm secret leadzump_token".into()),
            }],
            artifacts: Default::default(),
        };
        write(&cfg, &run).unwrap();
        let back = read(&cfg, "2026-10-09").unwrap();
        assert_eq!(back.outcome, Outcome::Degraded);
        assert_eq!(
            back.steps[0].fix.as_deref(),
            Some("moneyball crm secret leadzump_token")
        );
        assert!(read(&cfg, "2026-10-08").is_none());

        let held = lock(&cfg).unwrap();
        let second = lock(&cfg).err().expect("a second refresh is refused");
        assert!(second.to_string().contains("already running"));
        drop(held);
        assert!(lock(&cfg).is_ok(), "released when the first one ends");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
