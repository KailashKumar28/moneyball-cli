//! `moneyball debug` - readable session transcript plus invariant audit.
//!
//! Renders a saved session file headless: every item numbered, tool
//! failures surfaced, and the section 6b invariants checked (audit.rs).
//! The raw JSONL stays the source of truth; truncation is for the eyes,
//! `full` disables it. `report_all` audits every saved session.

mod audit;
mod listing;
pub mod reports;

pub use audit::{audit_raw, SessionAudit};
pub use listing::{report_all, report_archived, report_reports};
pub use reports::{is_bug_report_marker, marker_text, register};

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::agent::Item;
use crate::text;

use audit::{audit, is_aborted_marker, parse_raw};

const PREVIEW_LINES: usize = 8;
const PREVIEW_COLS: usize = 160;

/// Render the full debug report for one session file's raw contents.
pub fn report(path: &Path, raw: &str, full: bool) -> Result<String> {
    let (meta, items, skipped) = parse_raw(raw)?;

    let mut out = String::new();
    let _ = writeln!(out, "session {}", meta.id);
    let _ = writeln!(
        out,
        "  started:   {} UTC",
        meta.started_at.format("%Y-%m-%d %H:%M:%S")
    );
    let _ = writeln!(out, "  workspace: {}", meta.data_root.display());
    let _ = writeln!(
        out,
        "  file:      {} ({} bytes, {} items)",
        path.display(),
        raw.len(),
        items.len()
    );

    let _ = writeln!(out, "\ntranscript");
    let call_names: HashMap<&str, &str> = items
        .iter()
        .filter_map(|it| match it {
            Item::ToolCall { call_id, name, .. } => Some((call_id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    for (i, item) in items.iter().enumerate() {
        let n = i + 1;
        match item {
            Item::User { text } if is_aborted_marker(text) => {
                let _ = writeln!(out, "  [{}] turn_aborted (user interrupt marker)", n);
            }
            Item::User { text } if reports::is_bug_report_marker(text) => {
                let _ = writeln!(out, "  [{}] BUG REPORT (user flagged the item above)", n);
                write_body(&mut out, text, full);
            }
            Item::User { text } => {
                let _ = writeln!(out, "  [{}] user", n);
                write_body(&mut out, text, full);
            }
            Item::Assistant { text } => {
                let _ = writeln!(out, "  [{}] assistant", n);
                write_body(&mut out, text, full);
            }
            Item::ToolCall {
                call_id,
                name,
                args,
            } => {
                let _ = writeln!(out, "  [{}] tool_call {} ({})", n, name, call_id);
                write_body(&mut out, &args.to_string(), full);
            }
            Item::ToolOutput {
                call_id,
                output,
                is_error,
            } => {
                let name = call_names.get(call_id.as_str()).unwrap_or(&"?");
                let status = if *is_error { "ERROR" } else { "ok" };
                let _ = writeln!(
                    out,
                    "  [{}] tool_output {} ({}) {} - {} bytes",
                    n,
                    name,
                    call_id,
                    status,
                    output.len()
                );
                write_body(&mut out, output, full);
            }
        }
    }

    let _ = writeln!(out, "\naudit");
    let entries = audit(&items, &skipped);
    for line in &entries {
        let _ = writeln!(out, "  {}", line);
    }
    let n_issues = entries.iter().filter(|l| l.starts_with("issue:")).count();
    let _ = writeln!(
        out,
        "  {} issue(s), {} note(s)",
        n_issues,
        entries.len() - n_issues
    );
    Ok(out)
}

/// Indented, truncated body lines under an item heading.
fn write_body(out: &mut String, text: &str, full: bool) {
    let total = text.lines().count();
    for (i, line) in text.lines().enumerate() {
        if !full && i == PREVIEW_LINES {
            let _ = writeln!(out, "      ... (+{} more lines; --full to show)", total - i);
            return;
        }
        let shown = if full {
            line.to_string()
        } else {
            text::truncate_marked(line, PREVIEW_COLS, "...")
        };
        let _ = writeln!(out, "      {}", shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionMeta;
    use chrono::Utc;

    fn raw_with(items: &[Item], extra_lines: &[&str]) -> String {
        let meta = SessionMeta {
            id: "mb-test".into(),
            started_at: Utc::now(),
            data_root: "/w".into(),
        };
        let mut raw = format!(
            "{{\"session\":{}}}\n",
            serde_json::to_string(&meta).unwrap()
        );
        for it in items {
            raw.push_str(&serde_json::to_string(it).unwrap());
            raw.push('\n');
        }
        for l in extra_lines {
            raw.push_str(l);
            raw.push('\n');
        }
        raw
    }

    #[test]
    fn clean_session_reports_zero_issues() {
        let raw = raw_with(
            &[
                Item::User { text: "hi".into() },
                Item::ToolCall {
                    call_id: "c1".into(),
                    name: "brief".into(),
                    args: serde_json::json!({}),
                },
                Item::ToolOutput {
                    call_id: "c1".into(),
                    output: "table".into(),
                    is_error: false,
                },
                Item::Assistant {
                    text: "done".into(),
                },
            ],
            &[],
        );
        let rep = report(Path::new("/tmp/x.jsonl"), &raw, false).unwrap();
        assert!(rep.contains("0 issue(s)"), "{}", rep);
        assert!(rep.contains("tool_call brief (c1)"));
    }

    #[test]
    fn audit_flags_dangling_dead_turn_skipped_and_errors() {
        let raw = raw_with(
            &[
                Item::User { text: "one".into() },
                Item::User { text: "two".into() }, // dead turn for "one"
                Item::ToolCall {
                    call_id: "c9".into(),
                    name: "funnel".into(),
                    args: serde_json::json!({"product":"X"}),
                }, // never answered
                Item::ToolOutput {
                    call_id: "zz".into(),
                    output: "boom".into(),
                    is_error: true,
                }, // orphan + failure
            ],
            &["{\"type\":\"future\",\"x\":1}"],
        );
        let rep = report(Path::new("/tmp/x.jsonl"), &raw, false).unwrap();
        assert!(rep.contains("dead turn"), "{}", rep);
        assert!(rep.contains("never got a tool_output"), "{}", rep);
        assert!(rep.contains("unknown call_id 'zz'"), "{}", rep);
        assert!(rep.contains("unparseable line(s)"), "{}", rep);
        assert!(rep.contains("tool failure(s)"), "{}", rep);
        // The same raw through the aggregate unit gives the same counts.
        let a = audit_raw(&raw).unwrap();
        assert_eq!(a.issue_count(), 4, "{:?}", a.entries);
        assert_eq!(a.item_count, 4);
    }

    #[test]
    fn aborted_marker_is_not_a_dead_turn() {
        let raw = raw_with(
            &[
                Item::User { text: "q".into() },
                Item::User {
                    text: crate::agent::TURN_ABORTED_MARKER.into(),
                },
                Item::User {
                    text: "next".into(),
                },
                Item::Assistant { text: "a".into() },
            ],
            &[],
        );
        let rep = report(Path::new("/tmp/x.jsonl"), &raw, false).unwrap();
        assert!(!rep.contains("dead turn"), "{}", rep);
        assert!(rep.contains("1 interrupted turn(s)"), "{}", rep);
    }

    #[test]
    fn truncation_respects_full_flag() {
        let long = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let raw = raw_with(&[Item::Assistant { text: long }], &[]);
        let short = report(Path::new("/t"), &raw, false).unwrap();
        assert!(short.contains("more lines"), "{}", short);
        let full = report(Path::new("/t"), &raw, true).unwrap();
        assert!(full.contains("line19") && !full.contains("more lines"));
    }
}
