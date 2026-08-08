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
    /// Start a new session file (writes the header line). `root` is the
    /// workspace to store it under - None falls back to the global dir.
    /// create_new: an id collision must ERROR, never truncate another
    /// session mid-flight - session files are bug-report evidence.
    pub fn create(data_root: PathBuf, root: Option<&Path>) -> Result<Self> {
        use std::io::Write as _;
        for _ in 0..3 {
            let meta = SessionMeta {
                id: make_session_id(),
                started_at: Utc::now(),
                data_root: data_root.clone(),
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
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

fn session_path(id: &str, root: Option<&Path>) -> Result<PathBuf> {
    Ok(sessions_dir(root)?.join(format!("{}.jsonl", id)))
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

/// UTC timestamp + 4-char random suffix so two sessions in the same
/// second don't collide.
pub fn make_session_id() -> String {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    // ONE xorshift state advanced across all four chars. The old code
    // reseeded from the clock per char, so within a microsecond every
    // char was identical (observed live: oooo/3333/rrrr suffixes) -
    // 36 effective ids per second instead of 36^4.
    let mut x = seed();
    let suffix: String = (0..4)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ALPHA[(x as usize) % ALPHA.len()] as char
        })
        .collect();
    format!("mb-{}-{}", stamp, suffix)
}

// Tiny stand-alone PRNG (no rand crate dep). Good enough for suffix.
const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
fn seed() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::SystemTime;
    // Per-process counter: two ids minted in the same nanosecond tick
    // (or a coarse clock) still get distinct seeds.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let x = nanos ^ std::process::id().rotate_left(13) ^ n.rotate_left(24);
    // xorshift needs a non-zero state.
    if x == 0 {
        0x9E37_79B9
    } else {
        x
    }
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

    /// Serializes MONEYBALL_SESSIONS_DIR mutation across tests - env
    /// vars are process-global and cargo runs tests in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Full file round trip through the MONEYBALL_SESSIONS_DIR seam:
    /// create -> append -> open replays the same items, and the file
    /// is append-only JSONL (header first, one item per line).
    #[test]
    fn create_append_open_round_trip_on_disk() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mb-sessions-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MONEYBALL_SESSIONS_DIR", &dir);

        let log = SessionLog::create(PathBuf::from("/w"), None).unwrap();
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
        let log = SessionLog::create(PathBuf::from("/w"), None).unwrap();
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
