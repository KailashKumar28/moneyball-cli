//! What a session list shows per saved session, read from the files as
//! they are (the format has no title or count of its own).

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::agent::{view_items, ViewItem};

/// The longest preview, in bytes (cut on a char boundary).
const PREVIEW_CAP: usize = 200;

/// One saved session, for a list.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: DateTime<Utc>,
    /// When the file was last appended to.
    pub modified_at: DateTime<Utc>,
    /// Items in the file, markers included.
    pub item_count: usize,
    /// The first thing the user said (never a marker we authored), cut
    /// to one line; None for a session with no user message yet.
    pub preview: Option<String>,
    /// The front-end that wrote it (`SessionMeta::front_end`): for a
    /// front-end that lists only its own, not part of what a list shows.
    #[serde(skip)]
    pub front_end: Option<String>,
}

/// Every saved session under `root` (None = the global fallback dir),
/// most recently used first. Files that do not parse are left out.
pub fn summaries(root: Option<&Path>) -> Result<Vec<SessionSummary>> {
    summaries_in(&super::sessions_dir(root)?)
}

fn summaries_in(dir: &Path) -> Result<Vec<SessionSummary>> {
    let mut out: Vec<SessionSummary> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .filter_map(|e| summary_of(&e.path()))
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.modified_at));
    Ok(out)
}

fn summary_of(path: &Path) -> Option<SessionSummary> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (meta, items) = super::parse_session(&raw).ok()?;
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let preview = view_items(&items).into_iter().find_map(|v| match v {
        ViewItem::User { text } => Some(one_line(&text)),
        _ => None,
    });
    Some(SessionSummary {
        id: meta.id,
        started_at: meta.started_at,
        modified_at: modified.into(),
        item_count: items.len(),
        preview,
        front_end: meta.front_end,
    })
}

fn one_line(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::text::truncate_marked(&line, PREVIEW_CAP, "...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Item, TURN_ABORTED_MARKER};

    fn write(dir: &Path, id: &str, started: &str, items: &[Item]) {
        let mut raw = format!(
            "{{\"session\":{{\"id\":\"{id}\",\"started_at\":\"{started}\",\"data_root\":\"/w\"}}}}\n"
        );
        for i in items {
            raw.push_str(&serde_json::to_string(i).unwrap());
            raw.push('\n');
        }
        std::fs::write(dir.join(format!("{id}.jsonl")), raw).unwrap();
    }

    fn user(text: &str) -> Item {
        Item::User { text: text.into() }
    }

    #[test]
    fn a_summary_counts_items_and_previews_the_first_real_user_text() {
        let dir = std::env::temp_dir().join(format!("mb-summary-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "mb-old",
            "2026-11-01T10:00:00Z",
            &[
                user(TURN_ABORTED_MARKER),
                user(&crate::debug::marker_text("bad")),
                user("why is  Namma Mane\nso expensive?"),
                Item::Assistant { text: "a".into() },
            ],
        );
        write(&dir, "mb-empty", "2026-11-02T10:00:00Z", &[]);
        std::fs::write(dir.join("broken.jsonl"), "not a session\n").unwrap();
        // mb-old was used last: an hour after mb-empty.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let f = std::fs::File::options()
            .append(true)
            .open(dir.join("mb-old.jsonl"))
            .unwrap();
        f.set_modified(later).unwrap();

        let all = summaries_in(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(all.len(), 2, "the unparseable file is left out");
        assert_eq!(all[0].id, "mb-old", "most recently used first");
        assert_eq!(all[0].item_count, 4);
        assert_eq!(
            all[0].preview.as_deref(),
            Some("why is Namma Mane so expensive?")
        );
        assert!(all[0].modified_at > all[1].modified_at);
        assert_eq!(all[1].id, "mb-empty");
        assert_eq!((all[1].item_count, all[1].preview.as_deref()), (0, None));
    }

    #[test]
    fn a_long_preview_is_cut_on_a_char_boundary() {
        let cut = one_line(&"\u{20B9}".repeat(100));
        assert!(cut.ends_with("..."));
        assert!(cut.len() <= PREVIEW_CAP + 3);
    }
}
