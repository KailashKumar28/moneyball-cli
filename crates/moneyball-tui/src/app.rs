//! App state: the View enum, App struct, stream events, and
//! session load/save glue.

use crate::*;

use anyhow::Result;

use moneyball_core::brief::{self, ProductRowsAndFeasibility};
use moneyball_core::session::{Session, SessionCell};
use moneyball_core::AppConfig;

// Two chrono types collide on import name; alias the plain Utc.
use chrono::Utc as ChronoUtc;

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Brief is a unit variant; SetupState is fat.
                                     // Boxing the latter would force every match arm to deref.
pub(crate) enum View {
    Setup(SetupState),
    Brief,
}

pub struct App {
    pub(crate) cfg: AppConfig,
    pub(crate) view: View,
    pub input: String,
    pub cursor: usize,
    pub completion_idx: Option<usize>,
    pub completions: Vec<&'static str>,
    pub status: Option<String>,
    pub brief: Option<ProductRowsAndFeasibility>,
    pub snap_date: Option<String>,
    pub quit: bool,
    pub quit_emoji: String,
    /// Chat-style message log. Every interaction (slash-command, free-form
    /// question, tool output) is appended here. The main TUI body renders
    /// from this log; the dashboard view was replaced.
    pub chat: chat::ChatLog,
    /// Session id for auto-save. Set on App::new (fresh) or load_session_into (resume).
    pub session_id: Option<String>,
    /// When this session was started.
    pub session_started: Option<chrono::DateTime<chrono::Utc>>,
    /// Live LLM stream: deltas arrive from a worker thread and are drained
    /// by the event loop each tick (codex/claude-code streaming pattern).
    /// `Some` while a response is in flight; dropping it cancels the stream.
    pub stream: Option<std::sync::mpsc::Receiver<StreamEvent>>,
}

/// Events sent by the LLM streaming worker thread.
pub enum StreamEvent {
    Delta(String),
    Done { ms: u64, provider: String },
    Failed(String),
}

impl App {
    pub fn new_for_test(cfg: AppConfig) -> Self {
        Self::new(cfg)
    }

    pub fn force_setup_for_test(&mut self, state: SetupState) {
        self.view = View::Setup(state);
    }

    /// Test-only: route the app to the brief view with no brief loaded,
    /// so the welcome screen (configured but no data) renders.
    pub fn force_welcome_for_test(&mut self) {
        self.view = View::Brief;
        self.brief = None;
        self.snap_date = None;
    }

    /// Test-only: synthesize a workspace config so the welcome screen
    /// can render the configured-products list. Replaces whatever `cfg`
    /// had with a fresh in-memory WorkspaceConfig (not saved to disk).
    pub fn force_workspace_for_test(&mut self, products: Vec<(String, String)>) {
        use moneyball_core::config::{Product, WorkspaceConfig};
        let wc = WorkspaceConfig {
            products: products
                .into_iter()
                .map(|(n, a)| Product {
                    name: n,
                    ad_account: a,
                })
                .collect(),
            goals: Default::default(),
            target_rs_per_q: None,
            crm: Default::default(),
            model_provider: None,
            model: None,
            model_providers: Default::default(),
        };
        self.cfg.workspace = Some(wc);
    }

    pub fn render_to_string(&self, width: u16, height: u16) -> String {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render::render(f, self)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    pub(crate) fn new(cfg: AppConfig) -> Self {
        let view = if cfg.has_workspace() {
            View::Brief
        } else {
            View::Setup(SetupState::new(cfg.data_root.clone()))
        };
        let mut chat = chat::ChatLog::new();
        let now = ChronoUtc::now();
        let session_id = format!("mb-{}", now.format("%Y%m%dT%H%M%SZ"));
        // Push the moneyball ASCII logo as the first cell on every fresh session.
        chat.push(chat::Cell::System(chat::cells::System(
            moneyball_core::LOGO.into(),
        )));
        if cfg.has_workspace() {
            chat.push(chat::Cell::System(chat::cells::System(
                "workspace configured. try /brief, /funnel <product>, /ask or anything you want."
                    .into(),
            )));
        } else {
            chat.push(chat::Cell::System(chat::cells::System(
                "no workspace yet - run /setup to configure.".into(),
            )));
        }
        Self {
            cfg,
            view,
            input: String::new(),
            cursor: 0,
            completion_idx: None,
            completions: vec![],
            status: None,
            brief: None,
            snap_date: None,
            quit: false,
            quit_emoji: String::new(),
            chat,
            session_id: Some(session_id),
            session_started: Some(now),
            stream: None,
        }
    }

    pub fn load_brief(&mut self) {
        // Try to load the brief; failure shouldn't kill the REPL.
        match self.cfg.snap_for(self.cfg.date.as_deref()) {
            Ok(p) => match crate::snapshot_load(&p) {
                Ok(snap) => {
                    let history =
                        brief::load_history(&self.cfg.history_dir().join("scoreboard.csv"));
                    let r = brief::compute(&snap, &self.cfg, &history);
                    self.snap_date = Some(r::date_of(&snap));
                    self.brief = Some(r);
                    self.status = None;
                }
                Err(e) => self.status = Some(format!("snapshot load failed: {}", e)),
            },
            // Missing snapshot is the normal first-run state, not a fault:
            // the context bar already shows "no data", so keep the status
            // hint short + actionable instead of dumping the raw error.
            Err(_) => {
                self.status =
                    Some("no snapshot yet - /brief works once your fetcher writes data".into());
            }
        }
    }
}

mod r {
    use moneyball_core::Snapshot;
    pub fn date_of(s: &Snapshot) -> String {
        s.date.clone()
    }
}

pub(crate) fn snapshot_load(p: &std::path::Path) -> Result<moneyball_core::Snapshot> {
    Ok(moneyball_core::snapshot::load(p)?)
}

pub(crate) fn load_session_into(app: &mut App, s: Session) {
    // Original-codex style: instant prompt, no message-log replay.
    // The session_id and session_started are inherited so any new cells the
    // user creates will be appended to the same on-disk session file
    // (handled by save_current_session -> Session { id, started_at, cells }).
    app.chat = chat::ChatLog::new();
    app.session_id = Some(s.meta.id.clone());
    app.session_started = Some(s.meta.started_at);
}

fn chat_cell_to_session(c: &chat::Cell) -> SessionCell {
    use chat::cells as cc;
    match c {
        chat::Cell::System(cc::System(text)) => SessionCell::System { text: text.clone() },
        chat::Cell::UserPrompt(cc::UserPrompt { text, at }) => SessionCell::UserPrompt {
            text: text.clone(),
            at: at.with_timezone(&ChronoUtc),
        },
        chat::Cell::AssistantText(cc::AssistantText { text, streaming }) => {
            SessionCell::AssistantText {
                text: text.clone(),
                streaming: *streaming,
            }
        }
        chat::Cell::ToolCall(cc::ToolCall { name, args, status }) => SessionCell::ToolCall {
            name: name.clone(),
            args: args.clone(),
            status: match status {
                cc::ToolStatus::Pending => "pending".into(),
                cc::ToolStatus::Running => "running".into(),
                cc::ToolStatus::Done => "done".into(),
                cc::ToolStatus::Failed => "failed".into(),
            },
        },
        chat::Cell::ToolResult(cc::ToolResult {
            name,
            output,
            success,
            duration_ms,
        }) => SessionCell::ToolResult {
            name: name.clone(),
            output: output.clone(),
            success: *success,
            duration_ms: *duration_ms,
        },
        // BriefPlaceholder is a no-op cell (renders as empty). Persist
        // an empty assistant text so the session round-trips without
        // losing cells.
        chat::Cell::BriefPlaceholder => SessionCell::AssistantText {
            text: String::new(),
            streaming: false,
        },
    }
}

/// Snapshot the current chat log + workspace into a Session and persist it.
/// Overwrites if a session with the same id already exists (so the same
/// session id can be updated across multiple invocations).
pub(crate) fn save_current_session(app: &App) -> Result<()> {
    let meta = moneyball_core::session::SessionMeta {
        id: app.session_id.clone().unwrap_or_else(|| {
            // Generate one if we don't have one (shouldn't happen at this point).
            moneyball_core::session::list()
                .ok()
                .and_then(|mut v| v.pop().map(|m| m.id))
                .unwrap_or_else(|| format!("mb-{}", ChronoUtc::now().format("%Y%m%dT%H%M%SZ")))
        }),
        started_at: app.session_started.unwrap_or_else(ChronoUtc::now),
        ended_at: Some(ChronoUtc::now()),
        data_root: app.cfg.data_root.clone(),
        snap_date: app.snap_date.clone(),
        label: None,
    };
    let cells: Vec<SessionCell> = app.chat.cells.iter().map(chat_cell_to_session).collect();
    let s = Session { meta, cells };
    moneyball_core::session::save(&s)?;
    Ok(())
}
