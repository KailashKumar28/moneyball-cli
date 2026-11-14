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
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::Item;

mod id;
mod summary;
pub use id::make_session_id;
pub use summary::{summaries, SessionSummary};

/// Header line of every session file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub data_root: PathBuf,
    /// The front-end that wrote the file ("tui", "desktop"). None in a
    /// file from before this was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_end: Option<String>,
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
    /// Start a new session file (writes the header line). `root` is the
    /// workspace to store it under - None falls back to the global dir;
    /// `front_end` names the caller in the header.
    /// create_new: an id collision must ERROR, never truncate another
    /// session mid-flight - session files are bug-report evidence.
    pub fn create(data_root: PathBuf, root: Option<&Path>, front_end: &str) -> Result<Self> {
        use std::io::Write as _;
        for _ in 0..3 {
            let meta = SessionMeta {
                id: make_session_id(),
                started_at: Utc::now(),
                data_root: data_root.clone(),
                front_end: Some(front_end.into()),
            };
            let path = session_path(&meta.id, root)?;
            let mut f = match std::fs::File::create_new(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).with_context(|| format!("create {}", path.display())),
            };
            let header = serde_json::to_string(&Line::Header {
                session: meta.clone(),
            })?;
            writeln!(f, "{}", header).with_context(|| format!("write {}", path.display()))?;
            return Ok(Self { meta, path });
        }
        anyhow::bail!("could not mint a unique session id after 3 tries")
    }

    /// Open an existing session for resume: returns the handle
    /// (positioned to append) plus the replayed transcript.
    pub fn open(id: &str, root: Option<&Path>) -> Result<(Self, Vec<Item>)> {
        let path = session_path(id, root)?;
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

/// Parse one line as a session header, if it is one. Public because
/// `debug` re-parses files line by line to audit what replay would skip.
pub fn parse_header_line(line: &str) -> Option<SessionMeta> {
    match serde_json::from_str::<Line>(line) {
        Ok(Line::Header { session }) => Some(session),
        _ => None,
    }
}

/// Parse a session file body: header line first, then items. Unparseable
/// lines are skipped (forward compatibility) - never a hard failure.
fn parse_session(raw: &str) -> Result<(SessionMeta, Vec<Item>)> {
    let mut lines = raw.lines();
    let header = lines.next().context("empty session file")?;
    let meta = parse_header_line(header).context("first line is not a session header")?;
    let items = lines
        .filter_map(|l| serde_json::from_str::<Item>(l).ok())
        .collect();
    Ok((meta, items))
}

/// Where sessions live, created lazily. Precedence:
/// `MONEYBALL_SESSIONS_DIR` (hermetic-test seam, same pattern as
/// MONEYBALL_AUTH_PATH) > `<workspace>/.moneyball/sessions/` when a
/// workspace root is given (ARCHITECTURE section 2: workspace state
/// lives in the workspace dot-dir) > `~/.moneyball/sessions/` as the
/// no-workspace fallback (first run, before /setup).
pub fn sessions_dir(root: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("MONEYBALL_SESSIONS_DIR") {
        let dir = PathBuf::from(d);
        std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
        return Ok(dir);
    }
    let dir = match root {
        Some(ws) => ws.join(crate::config::DOT_DIR).join("sessions"),
        None => crate::config::home_dir()
            .context("no HOME / USERPROFILE - cannot resolve sessions directory")?
            .join(".moneyball")
            .join("sessions"),
    };
    // ~/.moneyball is owner-only; a workspace's dot-dir keeps its own mode.
    let made = match root {
        Some(_) => std::fs::create_dir_all(&dir),
        None => crate::secrets::create_private_dir(&dir),
    };
    made.with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

/// Workspace file first; an id that lives in the global dir (minted
/// before /setup in this very session, or before sessions moved into
/// workspaces) still resolves - /debug, --resume and audits all route
/// through here.
fn session_path(id: &str, root: Option<&Path>) -> Result<PathBuf> {
    let name = format!("{}.jsonl", id);
    let path = sessions_dir(root)?.join(&name);
    if root.is_some() && !path.exists() && std::env::var_os("MONEYBALL_SESSIONS_DIR").is_none() {
        let global = crate::config::home_dir().map(|h| h.join(".moneyball/sessions").join(&name));
        if let Some(g) = global.filter(|g| g.is_file()) {
            return Ok(g);
        }
    }
    Ok(path)
}

/// Raw file contents + path for a session id (the `debug` surface -
/// audits need the bytes replay would skip, not the parsed items).
pub fn read_raw(id: &str, root: Option<&Path>) -> Result<(PathBuf, String)> {
    let path = session_path(id, root)?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("no session file {}", path.display()))?;
    Ok((path, raw))
}

/// Newest-first session metadata (header lines only - cheap).
pub fn list(root: Option<&Path>) -> Result<Vec<SessionMeta>> {
    let dir = sessions_dir(root)?;
    let mut metas: Vec<SessionMeta> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .filter_map(|e| {
            let raw = std::fs::read_to_string(e.path()).ok()?;
            parse_header_line(raw.lines().next()?)
        })
        .collect();
    metas.sort_by_key(|m| std::cmp::Reverse(m.started_at));
    Ok(metas)
}

/// Id of the most recently started session, if any.
pub fn latest_id(root: Option<&Path>) -> Result<Option<String>> {
    Ok(list(root)?.into_iter().next().map(|m| m.id))
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

/// Serializes MONEYBALL_SESSIONS_DIR mutation across tests - env vars
/// are process-global and cargo runs tests in parallel. Any test that
/// creates a session file holds it.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Full file round trip through the MONEYBALL_SESSIONS_DIR seam:
    /// create -> append -> open replays the same items, and the file
    /// is append-only JSONL (header first, one item per line).
    #[test]
    fn create_append_open_round_trip_on_disk() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mb-sessions-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MONEYBALL_SESSIONS_DIR", &dir);

        let log = SessionLog::create(PathBuf::from("/w"), None, "tui").unwrap();
        let id = log.meta.id.clone();
        let items = vec![
            Item::User { text: "hi".into() },
            Item::ToolCall {
                call_id: "c".into(),
                name: "brief".into(),
                args: serde_json::json!({}),
            },
            Item::ToolOutput {
                call_id: "c".into(),
                output: "out".into(),
                is_error: false,
            },
            Item::Assistant { text: "a".into() },
        ];
        for i in &items {
            log.append(i).unwrap();
        }

        let (reopened, replayed) = SessionLog::open(&id, None).unwrap();
        assert_eq!(reopened.meta.id, id);
        assert_eq!(reopened.meta.front_end.as_deref(), Some("tui"));
        assert_eq!(replayed.len(), items.len());
        assert!(matches!(&replayed[0], Item::User { text } if text == "hi"));
        assert!(matches!(&replayed[3], Item::Assistant { text } if text == "a"));
        // Appending to the reopened log continues the same file.
        reopened
            .append(&Item::User {
                text: "again".into(),
            })
            .unwrap();
        let (_, replayed2) = SessionLog::open(&id, None).unwrap();
        assert_eq!(replayed2.len(), items.len() + 1);
        // latest_id sees this session through the same seam.
        assert_eq!(latest_id(None).unwrap().as_deref(), Some(id.as_str()));

        std::env::remove_var("MONEYBALL_SESSIONS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_session_replays_header_and_items() {
        let meta = SessionMeta {
            id: "mb-test".into(),
            started_at: Utc::now(),
            data_root: PathBuf::from("/w"),
            front_end: None,
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

    /// The front-end is an additive header field: a file from before it
    /// reads as written by no one in particular, and writes back as it was.
    #[test]
    fn a_header_without_a_front_end_still_reads() {
        let old =
            r#"{"session":{"id":"mb-old","started_at":"2026-11-01T10:00:00Z","data_root":"/w"}}"#;
        let meta = parse_header_line(old).unwrap();
        assert_eq!(meta.front_end, None);
        let line = serde_json::to_string(&Line::Header { session: meta }).unwrap();
        assert_eq!(line, old);
        let new = old.replace(r#""/w""#, r#""/w","front_end":"desktop""#);
        let meta = parse_header_line(&new).unwrap();
        assert_eq!(meta.front_end.as_deref(), Some("desktop"));
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

    /// The observed live bug: per-char clock reseeding made suffixes
    /// like "oooo"/"3333". With one advancing state, a run of ids must
    /// show non-repeated suffixes and no duplicates.
    #[test]
    fn suffixes_are_not_single_char_runs() {
        let ids: Vec<String> = (0..50).map(|_| make_session_id()).collect();
        let repeated = ids
            .iter()
            .filter(|id| {
                let sfx: Vec<char> = id.chars().rev().take(4).collect();
                sfx.iter().all(|c| *c == sfx[0])
            })
            .count();
        // One aaaa-style suffix in 50 is possible (36^-3 per id ~ 0.1%);
        // more than two means the state is not advancing.
        assert!(repeated <= 2, "degenerate suffixes: {:?}", ids);
        let mut uniq = ids.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len(), "duplicate ids in-process");
    }

    /// create_new contract: a second create landing on an existing path
    /// must not truncate it - the file keeps its content.
    #[test]
    fn existing_session_file_is_never_truncated() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mb-noclobber-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MONEYBALL_SESSIONS_DIR", &dir);
        let log = SessionLog::create(PathBuf::from("/w"), None, "tui").unwrap();
        log.append(&Item::User {
            text: "precious evidence".into(),
        })
        .unwrap();
        let path = dir.join(format!("{}.jsonl", log.meta.id));
        let before = std::fs::read_to_string(&path).unwrap();
        // Direct create_new on the same path errors instead of truncating.
        assert!(std::fs::File::create_new(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        std::env::remove_var("MONEYBALL_SESSIONS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }
}
