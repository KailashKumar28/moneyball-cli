//! Bug-report registry - `~/.moneyball/reports/` (codex-style dotfile,
//! same convention as auth.json and sessions/).
//!
//! A user who sees wrong analysis runs `/debug [comment]` (TUI) or
//! `moneyball debug --report [comment]` (CLI). That appends a marker
//! item to the session at the exact complaint point, freezes a copy of
//! the session file as evidence, and writes a report record. Before the
//! next release the admin reviews them with `moneyball debug --reports`
//! and audits each with `moneyball debug <id>`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Report record: one JSON file per reported session (re-reporting the
/// same session updates it; the frozen copy grows with the session).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugReport {
    pub session_id: String,
    pub reported_at: DateTime<Utc>,
    pub comment: String,
    /// Items in the session when reported - the complaint points at the
    /// item(s) just before this index.
    pub item_count: usize,
    /// moneyball version that produced the session.
    pub version: String,
    /// Stamped when the admin resolves the report (archived records
    /// always carry this; open ones never do).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}

/// How a report was closed - the audit trail a release note points at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub resolved_at: DateTime<Utc>,
    /// moneyball version the fix ships in (the version running resolve).
    pub fixed_in: String,
    pub note: String,
}

const MARKER_PREFIX: &str = "<bug_report>";

/// Marker item text. It is a user-role item on purpose: on resume the
/// model sees the flag ("this answer was wrong") instead of repeating
/// the mistake. The TUI renders it as a dim system note, never as a
/// user message.
pub fn marker_text(comment: &str) -> String {
    let c = comment.trim();
    format!(
        "{}The user flagged the response above as wrong or hallucinated.{}{}</bug_report>",
        MARKER_PREFIX,
        if c.is_empty() { "" } else { " Comment: " },
        c
    )
}

pub fn is_bug_report_marker(text: &str) -> bool {
    text.starts_with(MARKER_PREFIX)
}

/// `~/.moneyball/reports/`, created lazily. `MONEYBALL_REPORTS_DIR`
/// overrides it (hermetic-test seam, same pattern as sessions_dir).
pub fn reports_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("MONEYBALL_REPORTS_DIR") {
        let dir = PathBuf::from(d);
        std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
        return Ok(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("no HOME / USERPROFILE - cannot resolve reports directory")?;
    let dir = home.join(".moneyball").join("reports");
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

/// Register a session as buggy: freeze `session_raw` as evidence and
/// write the report record. The caller has already appended the marker
/// item (so the freeze includes it). Returns the record path.
pub fn register(
    session_id: &str,
    session_raw: &str,
    item_count: usize,
    comment: &str,
) -> Result<PathBuf> {
    let dir = reports_dir()?;
    let frozen = dir.join(format!("{}.jsonl", session_id));
    std::fs::write(&frozen, session_raw).with_context(|| format!("write {}", frozen.display()))?;
    let report = BugReport {
        session_id: session_id.to_string(),
        reported_at: Utc::now(),
        comment: comment.trim().to_string(),
        item_count,
        version: env!("CARGO_PKG_VERSION").to_string(),
        resolution: None,
    };
    let path = dir.join(format!("{}.json", session_id));
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// `reports/archive/` - resolved reports move here with a resolution
/// stamp; the root stays a clean inbox of open reports.
pub fn archive_dir() -> Result<PathBuf> {
    let dir = reports_dir()?.join("archive");
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

fn read_records(dir: &Path) -> Result<Vec<BugReport>> {
    let mut out: Vec<BugReport> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| {
            let raw = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&raw).ok()
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.reported_at));
    Ok(out)
}

/// Open report records, newest first. Unreadable records are skipped
/// (a corrupt report must not block the review of the others).
pub fn list_reports() -> Result<Vec<BugReport>> {
    read_records(&reports_dir()?)
}

/// Archived (resolved) report records, newest first.
pub fn list_archived() -> Result<Vec<BugReport>> {
    read_records(&archive_dir()?)
}

/// Close a report: stamp the resolution, move the record and the frozen
/// session copy into `archive/`. Errors if no open report exists for
/// the id (already archived or never filed).
pub fn resolve(session_id: &str, note: &str) -> Result<PathBuf> {
    let dir = reports_dir()?;
    let record = dir.join(format!("{}.json", session_id));
    let raw = std::fs::read_to_string(&record)
        .with_context(|| format!("no open report for '{}' ({})", session_id, record.display()))?;
    let mut report: BugReport =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", record.display()))?;
    report.resolution = Some(Resolution {
        resolved_at: Utc::now(),
        fixed_in: env!("CARGO_PKG_VERSION").to_string(),
        note: note.trim().to_string(),
    });

    let archive = archive_dir()?;
    let dest = archive.join(format!("{}.json", session_id));
    std::fs::write(&dest, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("write {}", dest.display()))?;
    // Evidence moves with the record; a missing frozen copy is fine.
    let frozen = dir.join(format!("{}.jsonl", session_id));
    if frozen.is_file() {
        std::fs::rename(&frozen, archive.join(format!("{}.jsonl", session_id)))
            .with_context(|| format!("archive {}", frozen.display()))?;
    }
    std::fs::remove_file(&record).with_context(|| format!("remove {}", record.display()))?;
    Ok(dest)
}

/// Frozen session copy for a reported session, if present.
pub fn frozen_raw(session_id: &str) -> Result<Option<String>> {
    let path = reports_dir()?.join(format!("{}.jsonl", session_id));
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes MONEYBALL_REPORTS_DIR mutation - without it a parallel
    /// test's remove_var could redirect a mid-run test at the real
    /// ~/.moneyball/reports.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn register_and_list_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mb-reports-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MONEYBALL_REPORTS_DIR", &dir);

        let raw = "{\"session\":{\"id\":\"mb-x\",\"started_at\":\"2026-08-02T00:00:00Z\",\"data_root\":\"/w\"}}\n";
        let path = register("mb-x", raw, 5, "  numbers look wrong  ").unwrap();
        assert!(path.ends_with("mb-x.json"));

        let reports = list_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].session_id, "mb-x");
        assert_eq!(reports[0].comment, "numbers look wrong");
        assert_eq!(reports[0].item_count, 5);
        assert_eq!(frozen_raw("mb-x").unwrap().unwrap(), raw);
        assert!(frozen_raw("mb-missing").unwrap().is_none());

        // Re-reporting the same session updates in place, never duplicates.
        register("mb-x", raw, 7, "still wrong").unwrap();
        let reports = list_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].item_count, 7);

        std::env::remove_var("MONEYBALL_REPORTS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_moves_report_to_archive_with_stamp() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mb-resolve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MONEYBALL_REPORTS_DIR", &dir);

        register("mb-r", "{}\n", 3, "wrong numbers").unwrap();
        let dest = resolve("mb-r", "fixed staleness warning in v0.1.0").unwrap();
        assert!(dest.to_string_lossy().contains("archive"));

        // Inbox is empty; archive holds the stamped record + evidence.
        assert!(list_reports().unwrap().is_empty());
        let archived = list_archived().unwrap();
        assert_eq!(archived.len(), 1);
        let res = archived[0].resolution.as_ref().expect("stamped");
        assert_eq!(res.note, "fixed staleness warning in v0.1.0");
        assert!(archive_dir().unwrap().join("mb-r.jsonl").is_file());
        // Resolving again fails loudly - it is already archived.
        assert!(resolve("mb-r", "again").is_err());

        std::env::remove_var("MONEYBALL_REPORTS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn marker_text_is_flagged_and_carries_comment() {
        assert!(is_bug_report_marker(&marker_text("bad math")));
        assert!(marker_text("bad math").contains("Comment: bad math"));
        assert!(!marker_text("").contains("Comment:"));
        assert!(!is_bug_report_marker("<turn_aborted>x"));
    }
}
