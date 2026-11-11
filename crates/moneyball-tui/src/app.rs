//! App state: the View enum, App struct, stream events, and
//! session load/save glue.

use crate::*;

use moneyball_core::agent::{Ev, Item, Session, ToolStatus, ViewItem};
use moneyball_core::brief::{self, ProductRowsAndFeasibility};
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
    /// /connect crm in flight; its question, if any, owns the composer.
    pub(crate) connect: Option<crate::crm_connect::ConnectJob>,
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
    /// The conversation (ARCHITECTURE.md 6b): the transcript in wire
    /// format, its session file (none in tests / headless render), and
    /// the turn in flight - which decides what Esc means.
    pub session: Session,
    /// The tool currently executing on the agent worker: (call_id,
    /// name, started). Set on ToolBegin, taken on ToolEnd so the
    /// result cell carries the real name and elapsed time.
    pub running_tool: Option<(String, String, std::time::Instant)>,
    /// When the turn in flight started (meaningful while one is active).
    pub turn_started: std::time::Instant,
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
            connect: None,
            ctrl_c_armed: None,
            quit: false,
            quit_armed: false,
            quit_emoji: String::new(),
            chat,
            session: Session::new(None, Vec::new()),
            running_tool: None,
            turn_started: std::time::Instant::now(),
            stream: None,
        }
    }

    /// Append one item to history AND the session log. A persistence
    /// failure surfaces once in the status line, never kills the REPL.
    pub(crate) fn record(&mut self, item: Item) {
        if let Err(e) = self.session.record(item) {
            self.status = Some(e.to_string());
        }
    }

    /// Rebuild chat cells from a resumed transcript (codex replay
    /// pattern: history drives both the prompt and the visible cells).
    pub(crate) fn replay(&mut self, items: &[Item]) {
        use crate::chat::cells;
        use crate::chat::Cell;
        for row in moneyball_core::agent::view_items(items) {
            match row {
                // Markers are model-facing, not real user messages -
                // show them as dim system notes.
                ViewItem::TurnInterrupted => self
                    .chat
                    .push(Cell::System(cells::System("(turn interrupted)".into()))),
                ViewItem::BugReportFiled => self
                    .chat
                    .push(Cell::System(cells::System("(bug report filed)".into()))),
                ViewItem::User { text } => self.chat.push(Cell::UserPrompt(cells::UserPrompt {
                    text,
                    at: chrono::Local::now(),
                })),
                ViewItem::Assistant { text } => {
                    self.chat.push(Cell::AssistantText(cells::AssistantText {
                        text,
                        streaming: false,
                    }))
                }
                ViewItem::Tool {
                    name,
                    args,
                    status,
                    output,
                    ..
                } => {
                    self.chat.push(Cell::ToolCall(cells::ToolCall {
                        name: name.clone(),
                        args: compact_args(&args),
                        status: cells::ToolStatus::Done,
                    }));
                    // A call cut off by an interrupt has no result.
                    if status != ToolStatus::Pending {
                        self.chat.push(Cell::ToolResult(cells::ToolResult {
                            name,
                            output: output.lines().map(String::from).collect(),
                            success: status == ToolStatus::Ok,
                            // Elapsed time is not persisted; 0 renders as
                            // no duration, never a fabricated "(0ms)".
                            duration_ms: 0,
                        }));
                    }
                }
            }
        }
    }

    /// Load what the bar, the landing and the brief read (core does the
    /// loading); a failure never kills the REPL.
    pub fn load_brief(&mut self) {
        let loaded = brief::load(&self.cfg);
        self.health = Some(loaded.health);
        if let Some(r) = loaded.brief {
            self.crm_note = loaded.crm_note;
            self.snap_date = Some(r.snapshot_date.clone());
            self.brief = Some(r);
            self.status = None;
        } else if let Some(e) = loaded.fault {
            // Configured, with data that would not load: the bar and
            // landing still speak for the workspace, never "setup".
            self.status = Some(format!("snapshot load failed: {}", e));
        }
        // Otherwise no snapshot yet - the normal first-run state: the bar
        // says "no data" and the landing says what to run.
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

/// Short one-line render of tool args for the cell header.
pub(crate) fn compact_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(m) if m.is_empty() => String::new(),
        other => other.to_string(),
    }
}
