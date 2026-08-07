//! Aggregate listings: `debug --all` (every session audited),
//! `--reports` (open bug reports), `--archived` (resolved history).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::session;

use super::{audit_raw, reports};

/// `debug --all`: one summary line per saved session, audit entries
/// under each, then totals and how to report a problem. `root` selects
/// the workspace whose sessions are audited (None = global fallback).
pub fn report_all(root: Option<&Path>) -> Result<String> {
    let metas = session::list(root)?;
    let mut out = String::new();
    if metas.is_empty() {
        let _ = writeln!(out, "(no saved sessions)");
        return Ok(out);
    }
    let total = metas.len();
    let mut flagged = 0usize;
    let mut issues_total = 0usize;
    for m in metas {
        match session::read_raw(&m.id, root).and_then(|(_, raw)| audit_raw(&raw)) {
            Ok(a) => {
                let issues = a.issue_count();
                let _ = writeln!(
                    out,
                    "{}  {}  {:>4} items  {} issue(s)",
                    a.meta.id,
                    a.meta.started_at.format("%Y-%m-%d %H:%M"),
                    a.item_count,
                    issues
                );
                for e in &a.entries {
                    let _ = writeln!(out, "    {}", e);
                }
                if issues > 0 {
                    flagged += 1;
                    issues_total += issues;
                }
            }
            Err(e) => {
                // A session debug cannot even parse is itself a finding.
                let _ = writeln!(out, "{}  UNREADABLE: {}", m.id, e);
                flagged += 1;
                issues_total += 1;
            }
        }
    }
    let _ = writeln!(
        out,
        "\n{} session(s): {} with issues, {} issue(s) total",
        total, flagged, issues_total
    );
    let _ = writeln!(
        out,
        "to report a problem: moneyball debug <id> --full > report.txt, then share it\n\
         with the session file from {} (transcripts contain\n\
         conversation and portfolio data, never API keys - review before sharing).",
        session::sessions_dir(root)?.display()
    );
    Ok(out)
}

/// `debug --reports`: every user-filed bug report, newest first, each
/// with its comment and an audit of the frozen session copy.
pub fn report_reports() -> Result<String> {
    let list = reports::list_reports()?;
    let mut out = String::new();
    if list.is_empty() {
        let _ = writeln!(out, "(no bug reports filed)");
        return Ok(out);
    }
    let _ = writeln!(out, "bug reports ({}), newest first:", list.len());
    for r in &list {
        let _ = writeln!(
            out,
            "\n  {}  reported {} UTC  v{}  at item {}",
            r.session_id,
            r.reported_at.format("%Y-%m-%d %H:%M"),
            r.version,
            r.item_count
        );
        if !r.comment.is_empty() {
            let _ = writeln!(out, "    comment: {}", r.comment);
        }
        match reports::frozen_raw(&r.session_id)? {
            Some(raw) => match audit_raw(&raw) {
                Ok(a) => {
                    let _ = writeln!(
                        out,
                        "    frozen copy: {} items, {} issue(s)",
                        a.item_count,
                        a.issue_count()
                    );
                    for e in &a.entries {
                        let _ = writeln!(out, "      {}", e);
                    }
                }
                Err(e) => {
                    let _ = writeln!(out, "    frozen copy: UNREADABLE ({})", e);
                }
            },
            None => {
                let _ = writeln!(out, "    frozen copy: missing");
            }
        }
    }
    let _ = writeln!(
        out,
        "\nreview a report: moneyball debug <id> --full (live file) or the frozen\ncopy in {}",
        reports::reports_dir()?.display()
    );
    let _ = writeln!(
        out,
        "close one: moneyball debug --resolve <id> --note \"what was fixed\""
    );
    let archived = reports::list_archived()?.len();
    if archived > 0 {
        let _ = writeln!(
            out,
            "{} resolved report(s) in the archive (moneyball debug --archived)",
            archived
        );
    }
    Ok(out)
}

/// `debug --archived`: resolved reports with their resolution stamps -
/// the audit trail of what got fixed, when, and in which version.
pub fn report_archived() -> Result<String> {
    let list = reports::list_archived()?;
    let mut out = String::new();
    if list.is_empty() {
        let _ = writeln!(out, "(no archived reports)");
        return Ok(out);
    }
    let _ = writeln!(out, "archived reports ({}), newest first:", list.len());
    for r in &list {
        let _ = writeln!(
            out,
            "\n  {}  reported {} UTC  v{}  at item {}",
            r.session_id,
            r.reported_at.format("%Y-%m-%d %H:%M"),
            r.version,
            r.item_count
        );
        if !r.comment.is_empty() {
            let _ = writeln!(out, "    comment:  {}", r.comment);
        }
        if let Some(res) = &r.resolution {
            let _ = writeln!(
                out,
                "    resolved: {} UTC in v{} - {}",
                res.resolved_at.format("%Y-%m-%d %H:%M"),
                res.fixed_in,
                res.note
            );
        }
    }
    Ok(out)
}
