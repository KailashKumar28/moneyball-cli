//! Session parsing + ARCHITECTURE.md section 6b invariant checks:
//! every tool_call answered, no dead turns, every line parseable.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::agent::Item;
use crate::session::{self, SessionMeta};
use crate::text;

/// Audit result for one session - the unit `debug --all` aggregates.
pub struct SessionAudit {
    pub meta: SessionMeta,
    pub item_count: usize,
    /// "issue:" / "note:" lines from the invariant checks.
    pub entries: Vec<String>,
}

impl SessionAudit {
    pub fn issue_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|l| l.starts_with("issue:"))
            .count()
    }
}

/// Parse + audit one raw session file without rendering the transcript.
pub fn audit_raw(raw: &str) -> Result<SessionAudit> {
    let (meta, items, skipped) = parse_raw(raw)?;
    Ok(SessionAudit {
        meta,
        item_count: items.len(),
        entries: audit(&items, &skipped),
    })
}

/// Header + items + 1-based file line numbers of unparseable lines.
/// Parses the body itself (not session::parse_session) so lines replay
/// would silently skip are audited instead.
pub(super) fn parse_raw(raw: &str) -> Result<(SessionMeta, Vec<Item>, Vec<usize>)> {
    let mut lines = raw.lines();
    let meta = lines
        .next()
        .and_then(session::parse_header_line)
        .context("first line is not a session header")?;
    let mut items: Vec<Item> = Vec::new();
    let mut skipped: Vec<usize> = Vec::new();
    for (idx, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Item>(line) {
            Ok(it) => items.push(it),
            Err(_) => skipped.push(idx + 2),
        }
    }
    Ok((meta, items, skipped))
}

pub(super) fn is_aborted_marker(text: &str) -> bool {
    text.starts_with("<turn_aborted>")
}

/// Any model-facing marker we author into the transcript - never a real
/// user message, so never part of dead-turn detection.
pub(super) fn is_marker(text: &str) -> bool {
    is_aborted_marker(text) || super::reports::is_bug_report_marker(text)
}

/// Section 6b invariant checks over a replayed transcript. Every entry
/// is "issue: ..." (a broken invariant) or "note: ..." (worth eyes,
/// not necessarily wrong).
pub(super) fn audit(items: &[Item], skipped: &[usize]) -> Vec<String> {
    let mut out = Vec::new();
    if !skipped.is_empty() {
        out.push(format!(
            "issue: {} unparseable line(s) skipped on replay (file lines {})",
            skipped.len(),
            skipped
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Tool-call pairing: every call answered, no orphans, no reuse.
    let mut pending: HashMap<&str, usize> = HashMap::new(); // call_id -> item no.
    let mut seen_calls: HashMap<&str, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let n = i + 1;
        match item {
            Item::ToolCall { call_id, name, .. } => {
                if seen_calls.insert(call_id.as_str(), n).is_some() {
                    out.push(format!(
                        "issue: [{}] duplicate call_id '{}' ({})",
                        n, call_id, name
                    ));
                }
                pending.insert(call_id.as_str(), n);
            }
            Item::ToolOutput { call_id, .. } => {
                let known = pending.remove(call_id.as_str()).is_some();
                if !known {
                    out.push(format!(
                        "issue: [{}] tool_output for unknown call_id '{}'",
                        n, call_id
                    ));
                }
            }
            _ => {}
        }
    }
    for (call_id, n) in pending {
        out.push(format!(
            "issue: [{}] tool_call '{}' never got a tool_output (crash or quit mid-turn)",
            n, call_id
        ));
    }

    // Dead turns: a real user item followed by another real user item
    // means the first got no response at all (failures must become
    // messages, never silence).
    let mut prev_real_user: Option<usize> = None;
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::User { text } if !is_marker(text) => {
                if let Some(p) = prev_real_user {
                    out.push(format!(
                        "issue: [{}] user item got no response before [{}] (dead turn)",
                        p,
                        i + 1
                    ));
                }
                prev_real_user = Some(i + 1);
            }
            _ => prev_real_user = None,
        }
    }
    if let Some(p) = prev_real_user {
        out.push(format!(
            "note: session ends on unanswered user item [{}] (quit or crash before the turn finished)",
            p
        ));
    }

    // Empty assistant items render as blank cells in the TUI.
    for (i, item) in items.iter().enumerate() {
        if let Item::Assistant { text } = item {
            if text.trim().is_empty() {
                out.push(format!("issue: [{}] assistant item is empty", i + 1));
            }
        }
    }

    // Tool failures are by-design messages, but an audit wants them listed.
    let failures: Vec<String> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::ToolOutput {
                output,
                is_error: true,
                ..
            } => Some(format!(
                "[{}] {}",
                i + 1,
                text::truncate_marked(output.lines().next().unwrap_or(""), 120, "...")
            )),
            _ => None,
        })
        .collect();
    if !failures.is_empty() {
        out.push(format!(
            "note: {} tool failure(s) fed back to the model: {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    let aborted = items
        .iter()
        .filter(|it| matches!(it, Item::User { text } if is_aborted_marker(text)))
        .count();
    if aborted > 0 {
        out.push(format!("note: {} interrupted turn(s)", aborted));
    }
    out
}
