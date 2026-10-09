//! App state: the View enum, App struct, stream events, and
//! session load/save glue.

use crate::*;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use moneyball_core::agent::{Ev, Item};
use moneyball_core::brief::{self, ProductRowsAndFeasibility};
use moneyball_core::session::SessionLog;
use moneyball_core::AppConfig;

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
    /// The CRM health sentence when this snapshot's CRM data isn't fresh.
    pub crm_note: Option<String>,
    /// How fresh the data on screen is (the context bar and the landing).
    pub health: Option<moneyball_core::health::Health>,
    /// The refresh in flight, on its own channel so chat keeps working.
    pub(crate) refresh: Option<crate::refresh_flow::RefreshJob>,
    /// The newest daily report, for ctrl+o.
    pub report_path: Option<std::path::PathBuf>,
    /// The morning refresh's time, when one is installed (launchd).
    pub schedule: Option<(u32, u32)>,
    /// A /schedule confirm waiting for its key.
    pub(crate) schedule_ask: Option<crate::schedule_view::Ask>,
    /// First Ctrl+C timestamp - a second within 2s quits (codex/Claude
    /// Code pattern: one stray Ctrl+C never kills the session).
    pub ctrl_c_armed: Option<std::time::Instant>,
    pub quit: bool,
    /// /quit was asked while a refresh runs; the next /quit leaves.
    pub(crate) quit_armed: bool,
    pub quit_emoji: String,
    /// Chat-style message log. Every interaction (slash-command, free-form
    /// question, tool output) is appended here. The main TUI body renders
    /// from this log; the dashboard view was replaced.
    pub chat: chat::ChatLog,
    /// Conversation transcript in wire format (ARCHITECTURE.md 6b).
    /// The prompt for every turn; appended items also go to `session`.
    pub history: Vec<Item>,
    /// Append-only JSONL session log. None in tests / headless render.
    pub session: Option<SessionLog>,
    /// Cancel flag shared with the agent worker; Esc sets it and the
    /// worker aborts between SSE events.
    pub cancel: Arc<AtomicBool>,
    /// True while an agent turn (not a fetch) is in flight - decides
    /// what Esc means.
    pub turn_active: bool,
    /// The tool currently executing on the agent worker: (call_id,
    /// name, started). Set on ToolBegin, taken on ToolEnd so the
    /// result cell carries the real name and elapsed time.
    pub running_tool: Option<(String, String, std::time::Instant)>,
    /// Live worker events, drained by the event loop each tick.
    /// `Some` while a turn or /report is in flight.
    pub stream: Option<std::sync::mpsc::Receiver<StreamEvent>>,
}

/// Events sent by background worker threads (agent turns, /report),
/// drained by the event loop each tick so the UI never blocks. A refresh
/// has its own channel (refresh_flow::RefreshJob).
pub enum StreamEvent {
    /// Agent-loop event (deltas, tool begin/end, turn end).
    Agent(Ev),
    /// `/report` worker finished generating the creative report.
    ReportDone {
        out: Box<moneyball_core::report::ReportOutput>,
        ms: u64,
    },
    ReportFailed {
        err: String,
        ms: u64,
    },
}

impl App {
    pub fn new_for_test(cfg: AppConfig) -> Self {
        Self::new(cfg)
    }

    pub fn force_setup_for_test(&mut self, state: SetupState) {
        self.view = View::Setup(state);
    }

    /// Test-only: route the app to the chat view with no brief loaded
    /// (configured, but no data on screen).
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
            workspace_id: None,
            brand: None,
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
        // Push the moneyball ASCII logo as the first cell on every fresh session.
        chat.push(chat::Cell::System(chat::cells::System(
            moneyball_core::LOGO.into(),
        )));
        // With a workspace, the landing (push_landing) follows once the
        // snapshot is loaded.
        if !cfg.has_workspace() {
            chat.push(chat::Cell::System(chat::cells::System(
                "no workspace yet - run /setup to configure.".into(),
            )));
        }
        let schedule = moneyball_core::schedule::installed(&cfg);
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
            crm_note: None,
            health: None,
            refresh: None,
            report_path: None,
            schedule,
            schedule_ask: None,
            ctrl_c_armed: None,
            quit: false,
            quit_armed: false,
            quit_emoji: String::new(),
            chat,
            history: Vec::new(),
            session: None,
            cancel: Arc::new(AtomicBool::new(false)),
            turn_active: false,
            running_tool: None,
            stream: None,
        }
    }

    /// Append one item to history AND the session log. A persistence
    /// failure surfaces once in the status line, never kills the REPL.
    pub(crate) fn record(&mut self, item: Item) {
        if let Some(log) = &self.session {
            if let Err(e) = log.append(&item) {
                self.status = Some(format!("session save failed: {}", e));
            }
        }
        self.history.push(item);
    }

    /// Rebuild chat cells from a resumed transcript (codex replay
    /// pattern: history drives both the prompt and the visible cells).
    pub(crate) fn replay(&mut self, items: Vec<Item>) {
        use crate::chat::cells;
        use crate::chat::Cell;
        let mut last_tool = String::from("tool");
        for item in &items {
            match item {
                Item::User { text } => {
                    // Markers are model-facing, not real user messages -
                    // show them as dim system notes.
                    if text.starts_with("<turn_aborted>") {
                        self.chat
                            .push(Cell::System(cells::System("(turn interrupted)".into())));
                    } else if moneyball_core::debug::is_bug_report_marker(text) {
                        self.chat
                            .push(Cell::System(cells::System("(bug report filed)".into())));
                    } else {
                        self.chat.push(Cell::UserPrompt(cells::UserPrompt {
                            text: text.clone(),
                            at: chrono::Local::now(),
                        }));
                    }
                }
                Item::Assistant { text } => {
                    self.chat.push(Cell::AssistantText(cells::AssistantText {
                        text: text.clone(),
                        streaming: false,
                    }));
                }
                Item::ToolCall { name, args, .. } => {
                    last_tool = name.clone();
                    self.chat.push(Cell::ToolCall(cells::ToolCall {
                        name: name.clone(),
                        args: compact_args(args),
                        status: cells::ToolStatus::Done,
                    }));
                }
                Item::ToolOutput {
                    output, is_error, ..
                } => {
                    self.chat.push(Cell::ToolResult(cells::ToolResult {
                        name: last_tool.clone(),
                        output: output.lines().map(String::from).collect(),
                        success: !is_error,
                        // Elapsed time is not persisted; 0 renders as
                        // no duration, never a fabricated "(0ms)".
                        duration_ms: 0,
                    }));
                }
            }
        }
        self.history = items;
    }

    pub fn load_brief(&mut self) {
        // Try to load the brief; failure shouldn't kill the REPL.
        match self.cfg.snap_for(self.cfg.date.as_deref()) {
            Ok(p) => match crate::snapshot_load(&p) {
                Ok(snap) => {
                    let history =
                        brief::load_history(&self.cfg.history_dir().join("scoreboard.csv"));
                    let r = brief::compute(&snap, &self.cfg, &history);
                    self.crm_note =
                        moneyball_core::crm::status::CrmHealth::of(&self.cfg, &snap).note();
                    self.health = Some(moneyball_core::health::Health::of(&self.cfg, Some(&snap)));
                    self.snap_date = Some(r::date_of(&snap));
                    self.brief = Some(r);
                    self.status = None;
                }
                Err(e) => {
                    // Configured, with data that would not load: the bar and
                    // landing still speak for the workspace, never "setup".
                    self.health = Some(moneyball_core::health::Health::of(&self.cfg, None));
                    self.status = Some(format!("snapshot load failed: {}", e));
                }
            },
            // Missing snapshot is the normal first-run state, not a fault:
            // the context bar already shows "no data", so keep the status
            // hint short + actionable instead of dumping the raw error.
            Err(_) => {
                // The bar says "no data" and the landing says what to run.
                self.health = Some(moneyball_core::health::Health::of(&self.cfg, None));
            }
        }
    }

    /// The first thing under the logo: how fresh the data is, and
    /// yesterday's numbers when it is (crate::landing).
    pub(crate) fn push_landing(&mut self) {
        let Some(health) = &self.health else {
            return;
        };
        let (paras, report) = crate::landing::landing(&self.cfg, health, self.schedule.is_some());
        self.report_path = report;
        self.chat.push(chat::Cell::Rich(chat::cells::Rich(paras)));
    }
}

mod r {
    use moneyball_core::Snapshot;
    pub fn date_of(s: &Snapshot) -> String {
        s.date.clone()
    }
}

pub(crate) fn snapshot_load(p: &std::path::Path) -> anyhow::Result<moneyball_core::Snapshot> {
    Ok(moneyball_core::snapshot::load(p)?)
}

/// Short one-line render of tool args for the cell header.
pub(crate) fn compact_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(m) if m.is_empty() => String::new(),
        other => other.to_string(),
    }
}
