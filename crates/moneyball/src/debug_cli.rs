//! `moneyball debug` dispatch - transcript dump, session audits, bug
//! report filing, and the admin review/archive pass. Split from main.rs
//! (connect_flow.rs pattern: main stays a thin dispatcher).

use std::path::Path;

use anyhow::Result;
use clap::Args;

use moneyball_core::{debug, session};

#[derive(Args, Debug)]
pub struct DebugArgs {
    /// Session ID (see --list). Default: the most recent session.
    id: Option<String>,
    /// Print full item bodies instead of truncated previews.
    #[arg(long)]
    full: bool,
    /// Audit every saved session: one summary line each, issues listed.
    #[arg(long, conflicts_with_all = ["id", "full"])]
    all: bool,
    /// File a bug report for the session (default: most recent).
    #[arg(long, value_name = "COMMENT", num_args = 0..=1, default_missing_value = "",
          conflicts_with_all = ["all", "full", "reports"])]
    report: Option<String>,
    /// List user-filed bug reports (admin review pass).
    #[arg(long, conflicts_with_all = ["id", "full", "all", "report"])]
    reports: bool,
    /// Close a report: stamp the resolution and move it to the archive.
    #[arg(long, value_name = "ID", conflicts_with_all = ["id", "full", "all", "report", "reports"])]
    resolve: Option<String>,
    /// Resolution note for --resolve (what was fixed, or why closed).
    #[arg(long, value_name = "NOTE", requires = "resolve")]
    note: Option<String>,
    /// List archived (resolved) reports with their resolution stamps.
    #[arg(long, conflicts_with_all = ["id", "full", "all", "report", "reports", "resolve"])]
    archived: bool,
}

/// `root` is the workspace whose sessions are addressed (None = the
/// global pre-setup fallback).
pub fn run(a: DebugArgs, root: Option<&Path>) -> Result<()> {
    if a.all {
        print!("{}", debug::report_all(root)?);
        return Ok(());
    }
    if a.reports {
        print!("{}", debug::report_reports()?);
        return Ok(());
    }
    if a.archived {
        print!("{}", debug::report_archived()?);
        return Ok(());
    }
    if let Some(rid) = a.resolve {
        let dest = debug::reports::resolve(&rid, a.note.as_deref().unwrap_or(""))?;
        println!("report archived: {}", dest.display());
        return Ok(());
    }
    let id = match a.id.or(session::latest_id(root)?) {
        Some(id) => id,
        None => {
            println!("(no saved sessions)");
            return Ok(());
        }
    };
    if let Some(comment) = a.report {
        // Same flow as the TUI's /debug: marker into the session at the
        // complaint point, then freeze + record.
        let (log, items) = session::SessionLog::open(&id, root)?;
        log.append(&moneyball_core::agent::Item::User {
            text: debug::marker_text(&comment),
        })?;
        let (_, raw) = session::read_raw(&id, root)?;
        let path = debug::register(&id, &raw, items.len() + 1, &comment)?;
        println!("bug report filed: {}", path.display());
        println!("admin review: moneyball debug --reports");
        return Ok(());
    }
    let (path, raw) = session::read_raw(&id, root)?;
    print!("{}", debug::report(&path, &raw, a.full)?);
    Ok(())
}
