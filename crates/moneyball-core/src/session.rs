//! Session persistence.
//!
//! A session is the user's full interaction with moneyball in one invocation:
//! the chat log cells, the workspace they were in, the start time, and the
//! optional snapshot date. Sessions are saved to ~/.moneyball/sessions/ as
//! JSON so a follow-up invocation can resume the conversation.
//!
//! CLI behavior:
//!   moneyball         -> new session (always - no confirmation)
//!   moneyball -c      -> resume most-recent session
//!   moneyball --resume <id>  -> resume a specific session
//!   moneyball --list   -> list all saved sessions and exit

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-cell schema used to persist the chat log as JSON. Parallel to
/// moneyball-tui's chat::Cell enum; we keep them distinct so the wire
/// format can evolve independently of the in-process type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionCell {
    System {
        text: String,
    },
    UserPrompt {
        text: String,
        at: DateTime<Utc>,
    },
    AssistantText {
        text: String,
        streaming: bool,
    },
    ToolCall {
        name: String,
        args: String,
        status: String,
    },
    ToolResult {
        name: String,
        output: Vec<String>,
        success: bool,
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub data_root: PathBuf,
    pub snap_date: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub meta: SessionMeta,
    pub cells: Vec<SessionCell>,
}

impl Session {
    pub fn new(data_root: PathBuf) -> Self {
        let id = make_session_id(Utc::now());
        Self {
            meta: SessionMeta {
                id,
                started_at: Utc::now(),
                ended_at: None,
                data_root,
                snap_date: None,
                label: None,
            },
            cells: Vec::new(),
        }
    }

    pub fn end(&mut self) {
        self.meta.ended_at = Some(Utc::now());
    }
}

/// Returns the directory sessions are persisted to. `~/.moneyball/sessions/`.
/// Creates it lazily.
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
    Ok(sessions_dir()?.join(format!("{}.json", id)))
}

pub fn save(s: &Session) -> Result<()> {
    let p = session_path(&s.meta.id)?;
    let pretty = serde_json::to_string_pretty(s).context("serialize session")?;
    std::fs::write(&p, pretty).with_context(|| format!("write {}", p.display()))?;
    Ok(())
}

/// Returns the most recently started session, or None if there are none.
pub fn latest() -> Result<Option<Session>> {
    let dir = sessions_dir()?;
    let mut sessions: Vec<Session> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| {
            let raw = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str::<Session>(&raw).ok()
        })
        .collect();
    // Most-recently-started first.
    sessions.sort_by_key(|b| std::cmp::Reverse(b.meta.started_at));
    Ok(sessions.into_iter().next())
}

pub fn load(id: &str) -> Result<Session> {
    let p = session_path(id)?;
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let s: Session =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", p.display()))?;
    Ok(s)
}

/// List sessions sorted by recency, newest first. Returns the metadata only
/// (does not load full cells) - cheap for displaying a picker.
pub fn list() -> Result<Vec<SessionMeta>> {
    let dir = sessions_dir()?;
    let mut metas: Vec<SessionMeta> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| {
            let raw = std::fs::read_to_string(e.path()).ok()?;
            // Cheap: parse only the meta field via raw_json. We just parse the whole
            // Session since it's small enough; v2 can switch to a stream parser.
            serde_json::from_str::<Session>(&raw).ok().map(|s| s.meta)
        })
        .collect();
    metas.sort_by_key(|b| std::cmp::Reverse(b.started_at));
    Ok(metas)
}

pub fn make_session_id(_now: chrono::DateTime<chrono::Utc>) -> String {
    // UTC timestamp + 4-char random suffix so two sessions in the same
    // second don't collide.
    let now = Utc::now();
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let suffix: String = (0..4)
        .map(|_| {
            let idx = (rand_u32() as usize) % ALPHA.len();
            ALPHA[idx] as char
        })
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
    let pid = std::process::id() as u32;
    // simple xorshift
    let mut x = nanos ^ (pid.rotate_left(13));
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

/// Format a session for one-line display in the picker. Includes human-readable
/// age (e.g. "5m ago", "2h ago", "yesterday").
pub fn fmt_meta_line(m: &SessionMeta) -> String {
    let dur = Utc::now().signed_duration_since(m.started_at);
    let secs = dur.num_seconds().max(0);
    let human = if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    };
    let label_part = m.label.clone().unwrap_or_else(|| "(no label)".into());
    let end_part = match m.ended_at {
        Some(_) => "ended",
        None => "open",
    };
    format!("  {}  {}  {}  [{}]", m.id, human, end_part, label_part)
}
