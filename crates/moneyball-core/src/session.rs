//! Session persistence - append-only JSONL (ARCHITECTURE.md section 6b,
//! codex rollout pattern).
//!
//! `~/.moneyball/sessions/<id>.jsonl`: line 1 is a header, every further
//! line is one `agent::Item` - the same enum that is the in-memory
//! transcript and the prompt. Resume = read lines, replay. The file is
//! never rewritten.
//!
//! CLI behavior:
//!   moneyball                -> new session
//!   moneyball -c             -> resume most-recent session
//!   moneyball --resume <id>  -> resume a specific session
//!   moneyball --list         -> list saved sessions and exit

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::Item;

/// Header line of every session file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub data_root: PathBuf,
}

/// One line of the file: the header or an item. The serde tags cannot
/// collide: SessionMeta is wrapped, Items use their own type tags.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Line {
    Header { session: SessionMeta },
    Item(Item),
}

/// Append handle for a live session. Opens the file per append - chat
/// cadence makes that cheap, and it means a crash never loses more
/// than the in-flight line.
pub struct SessionLog {
    pub meta: SessionMeta,
    path: PathBuf,
}

impl SessionLog {
    /// Start a new session file (writes the header line).
    pub fn create(data_root: PathBuf) -> Result<Self> {
        let meta = SessionMeta {
            id: make_session_id(),
            started_at: Utc::now(),
            data_root,
        };
        let path = session_path(&meta.id)?;
        let header = serde_json::to_string(&Line::Header {
            session: meta.clone(),
        })?;
        std::fs::write(&path, format!("{}\n", header))
            .with_context(|| format!("write {}", path.display()))?;
        Ok(Self { meta, path })
    }

    /// Open an existing session for resume: returns the handle
    /// (positioned to append) plus the replayed transcript.
    pub fn open(id: &str) -> Result<(Self, Vec<Item>)> {
        let path = session_path(id)?;
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("no session file {}", path.display()))?;
        let (meta, items) = parse_session(&raw)?;
        Ok((Self { meta, path }, items))
    }

    /// Append one transcript item. Errors are surfaced (a session that
    /// silently stops persisting is worse than a visible warning).
    pub fn append(&self, item: &Item) -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open {}", self.path.display()))?;
        writeln!(f, "{}", serde_json::to_string(item)?)?;
        Ok(())
    }
}

/// Parse a session file body: header line first, then items. Unparseable
/// lines are skipped (forward compatibility) - never a hard failure.
fn parse_session(raw: &str) -> Result<(SessionMeta, Vec<Item>)> {
    let mut lines = raw.lines();
    let header = lines.next().context("empty session file")?;
    let meta = match serde_json::from_str::<Line>(header) {
        Ok(Line::Header { session }) => session,
        _ => anyhow::bail!("first line is not a session header"),
    };
    let items = lines
        .filter_map(|l| serde_json::from_str::<Item>(l).ok())
        .collect();
    Ok((meta, items))
}

/// `~/.moneyball/sessions/`, created lazily.
pub fn sessions_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("no HOME / USERPROFILE - cannot resolve sessions directory")?;
    let dir = home.join(".moneyball").join("sessions");
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

fn session_path(id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{}.jsonl", id)))
}

/// Newest-first session metadata (header lines only - cheap).
pub fn list() -> Result<Vec<SessionMeta>> {
    let dir = sessions_dir()?;
    let mut metas: Vec<SessionMeta> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .filter_map(|e| {
            let raw = std::fs::read_to_string(e.path()).ok()?;
            let first = raw.lines().next()?;
            match serde_json::from_str::<Line>(first).ok()? {
                Line::Header { session } => Some(session),
                _ => None,
            }
        })
        .collect();
    metas.sort_by_key(|m| std::cmp::Reverse(m.started_at));
    Ok(metas)
}

/// Id of the most recently started session, if any.
pub fn latest_id() -> Result<Option<String>> {
    Ok(list()?.into_iter().next().map(|m| m.id))
}

/// UTC timestamp + 4-char random suffix so two sessions in the same
/// second don't collide.
pub fn make_session_id() -> String {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let suffix: String = (0..4)
        .map(|_| ALPHA[(rand_u32() as usize) % ALPHA.len()] as char)
        .collect();
    format!("mb-{}-{}", stamp, suffix)
}

// Tiny stand-alone PRNG (no rand crate dep). Good enough for suffix.
const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
fn rand_u32() -> u32 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut x = nanos ^ (pid.rotate_left(13));
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

/// One-line display for the session picker ("5m ago" style age).
pub fn fmt_meta_line(m: &SessionMeta) -> String {
    let secs = Utc::now()
        .signed_duration_since(m.started_at)
        .num_seconds()
        .max(0);
    let human = if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    };
    format!("  {}  {}  {}", m.id, human, m.data_root.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_replays_header_and_items() {
        let meta = SessionMeta {
            id: "mb-test".into(),
            started_at: Utc::now(),
            data_root: PathBuf::from("/w"),
        };
        let mut raw = format!(
            "{}\n",
            serde_json::to_string(&Line::Header {
                session: meta.clone()
            })
            .unwrap()
        );
        let items = vec![
            Item::User { text: "hi".into() },
            Item::ToolCall {
                call_id: "c".into(),
                name: "brief".into(),
                args: serde_json::json!({}),
            },
            Item::ToolOutput {
                call_id: "c".into(),
                output: "t".into(),
                is_error: false,
            },
            Item::Assistant { text: "a".into() },
        ];
        for i in &items {
            raw.push_str(&serde_json::to_string(i).unwrap());
            raw.push('\n');
        }
        raw.push_str("{\"type\":\"future_thing\",\"x\":1}\n"); // skipped, not fatal
        let (m, back) = parse_session(&raw).unwrap();
        assert_eq!(m.id, meta.id);
        assert_eq!(back.len(), items.len());
    }

    #[test]
    fn header_must_be_first_line() {
        assert!(parse_session("{\"type\":\"user\",\"text\":\"x\"}\n").is_err());
        assert!(parse_session("").is_err());
    }

    #[test]
    fn ids_do_not_collide_within_a_second() {
        let a = make_session_id();
        let b = make_session_id();
        assert_ne!(a, b);
    }
}
