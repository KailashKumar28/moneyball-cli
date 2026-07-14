//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

pub mod chat;
pub mod widgets;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Terminal;

use moneyball_core::brief::{self, ProductRowsAndFeasibility};
use moneyball_core::session::{Session, SessionCell};
use moneyball_core::provider::{built_in_presets, models_for, ModelProviderInfo, WireApi};
use moneyball_core::{list_ad_accounts, validate_token, AdAccount, AppConfig, WorkspaceConfig};

// Two chrono types collide on import name; alias the plain Utc.
use chrono::Utc as ChronoUtc;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

// ---------- slash-command surface ----------

const COMMANDS: &[(&str, &str)] = &[
    ("/brief", "7-day portfolio brief"),
    ("/funnel", "per-entity funnel for a product"),
    ("/diagnose", "run all 5 diagnostic commands for a product"),
    ("/ask", "free-form question (LLM picks commands)"),
    ("/snapshot", "list or validate snapshots"),
    ("/ledger", "prediction ledger view"),
    ("/setup", "re-run the setup wizard"),
    ("/quit", "exit moneyball"),
];

fn completions(prefix: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| c.starts_with(prefix))
        .collect()
}

// ---------- app state ----------

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Brief is a unit variant; SetupState is fat.
                                     // Boxing the latter would force every match arm to deref.
enum View {
    Setup(SetupState),
    Brief,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetupState {
    pub step: usize,
    pub workspace_path: String,
    /// Step 1 (Meta connect): which substep we're on.
    pub meta_substep: u8,
    /// Step 1 buffer: token paste / "skip".
    pub meta_input: String,
    /// Step 1: discovered ad accounts after token validation.
    pub meta_discovered: Vec<AdAccount>,
    /// Step 1 substep 1: per-account checkbox state.
    pub meta_selections: Vec<bool>,
    /// Step 1 substep 1: cursor row (0-based into meta_discovered).
    pub meta_highlight: usize,
    /// Step 1 substep 1: first visible row (scroll).
    pub meta_scroll: usize,
    /// Step 1 substep 2: rename overrides, format `1=Name 2=OtherName`.
    pub meta_rename_input: String,
    /// Step 1: final selection (Vec of indices into meta_discovered).
    pub meta_selected: Vec<usize>,
    /// Step 1: whether the user pasted a valid token. Token is mandatory now -
    /// we always validate, never skip.
    pub meta_connected: bool,
    /// Length of the validated token in characters. Captured before
    /// `meta_input` is cleared so the collapsed summary can show "••••• (N chars)".
    pub meta_token_len: usize,
    /// Step 2 entry buffer: "Name AdAccount" (space- or comma-separated).
    pub product_input: String,
    pub products: Vec<(String, String)>, // (name, ad_account)
    /// Step 3 entry buffer: "Prod1=10 Prod2=12".
    pub goals_input: String,
    /// Step 4 (LLM provider) substep: 0=provider pick, 1=API key paste,
    /// 2=model pick, 3=custom URL entry.
    pub llm_substep: u8,
    /// Step 4: selected provider id ("openai", "minimax", "custom", ...).
    pub llm_provider_id: String,
    /// Step 4: API key paste buffer (raw, before keychain write).
    pub llm_input: String,
    /// Step 4: length of captured API key for collapsed-summary bullets.
    pub llm_key_len: usize,
    /// Step 4: selected model slug (e.g. "gpt-5", "MiniMax-M3").
    pub llm_model: String,
    /// Step 4: custom base URL (only when provider id is "custom").
    pub llm_url: String,
    /// Step 4 picker: cursor row in the provider/model list.
    pub llm_highlight: usize,
    /// Step 4 picker: first visible row (scroll).
    pub llm_scroll: usize,
    /// Resolved provider entry for the selected id. Built as the user
    /// advances; persisted on save.
    pub llm_provider: Option<ModelProviderInfo>,
    pub error: Option<String>,
}

impl SetupState {
    pub fn new(default: PathBuf) -> Self {
        Self {
            step: 0,
            workspace_path: default.display().to_string(),
            meta_substep: 0,
            meta_input: String::new(),
            meta_discovered: Vec::new(),
            meta_selections: Vec::new(),
            meta_highlight: 0,
            meta_scroll: 0,
            meta_rename_input: String::new(),
            meta_selected: Vec::new(),
            meta_connected: false,
            meta_token_len: 0,
            product_input: String::new(),
            products: Vec::new(),
            goals_input: String::new(),
            llm_substep: 0,
            llm_provider_id: String::new(),
            llm_input: String::new(),
            llm_key_len: 0,
            llm_model: String::new(),
            llm_url: String::new(),
            llm_highlight: 0,
            llm_scroll: 0,
            llm_provider: None,
            error: None,
        }
    }
}

/// Built-in Fincity example products. Loaded when the user types `demo` in step 2.
const DEMO_PRODUCTS: &[(&str, &str)] = &[
    ("Namma Mane", "2087011578504572"),
    ("Valmark CityVille", "852565919728055"),
    ("Purva Sparkling Springs", "1043714050577651"),
    ("Primus by Fincity", "405885579167395"),
];

pub struct App {
    cfg: AppConfig,
    view: View,
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
        terminal.draw(|f| render(f, self)).unwrap();
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

    fn new(cfg: AppConfig) -> Self {
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
            Err(e) => self.status = Some(format!("no snapshot: {}", e)),
        }
    }
}

mod r {
    use moneyball_core::Snapshot;
    pub fn date_of(s: &Snapshot) -> String {
        s.date.clone()
    }
}

fn snapshot_load(p: &std::path::Path) -> Result<moneyball_core::Snapshot> {
    Ok(moneyball_core::snapshot::load(p)?)
}

// ---------- main entry ----------

pub fn run() -> Result<()> {
    run_with(None)
}

/// Entry point that optionally pre-loads a saved session into the chat log.
pub fn run_with(resume_session: Option<Session>) -> Result<()> {
    run_with_cfg(resume_session, None)
}

pub fn run_with_cfg(
    resume_session: Option<Session>,
    cfg_override: Option<AppConfig>,
) -> Result<()> {
    let cfg = match cfg_override {
        Some(c) => c,
        None => AppConfig::resolve_optional(None, None),
    };
    let mut app = App::new(cfg);
    if let Some(s) = resume_session {
        load_session_into(&mut app, s);
    }
    if matches!(app.view, View::Brief) {
        app.load_brief();
    }
    let mut terminal = init()?;
    let res = event_loop(&mut terminal, &mut app);
    restore()?;
    // Auto-save the chat log so /quit or Ctrl-C still persists.
    if let Err(e) = save_current_session(&app) {
        eprintln!("warning: session save failed: {}", e);
    }
    res
}

fn load_session_into(app: &mut App, s: Session) {
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
fn save_current_session(app: &App) -> Result<()> {
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

fn event_loop(t: &mut Tui, app: &mut App) -> Result<()> {
    let tick = Duration::from_millis(100);
    loop {
        t.draw(|f| render(f, app))?;
        if app.quit {
            break;
        }
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(k) => handle_key(app, k),
                // crossterm 0.28 emits Event::Paste for clipboard pastes; route
                // to whichever input field is currently focused.
                Event::Paste(text) => handle_paste(app, text),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Route a pasted string into the currently focused input field. Without this,
/// clipboard pastes (e.g. the Meta access token in the setup wizard) are
/// silently dropped because crossterm emits Event::Paste, not a stream of
/// KeyEvents.
fn handle_paste(app: &mut App, text: String) {
    if text.is_empty() {
        return;
    }
    match &mut app.view {
        View::Setup(state) => {
            // Strip whitespace and newlines so paste of a token works even if
            // it was wrapped or had trailing whitespace in the clipboard.
            let clean: String = text.chars().filter(|c| !c.is_control()).collect();
            match (state.step, state.meta_substep) {
                (1, 0) => state.meta_input.push_str(&clean),
                (1, 2) => state.meta_rename_input.push_str(&clean),
                (2, _) => state.product_input.push_str(&clean),
                (3, _) => state.goals_input.push_str(&clean),
                (0, _) => state.workspace_path.push_str(&clean),
                _ => {}
            }
        }
        View::Brief => app.input.push_str(&text),
    }
}

fn handle_key(app: &mut App, k: KeyEvent) {
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    match &app.view.clone() {
        View::Setup(state) => handle_setup_key(app, state.clone(), k),
        View::Brief => handle_brief_key(app, k),
    }
}

// ---------- brief-view keys ----------

fn handle_brief_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => handle_esc(app),
        KeyCode::Tab => {
            arm_cancel(app);
            if app.input.starts_with('/') && app.completions.is_empty() {
                app.completions = completions(&app.input);
                app.completion_idx = if app.completions.is_empty() {
                    None
                } else {
                    Some(0)
                };
            } else if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            }
            apply_completion(app);
        }
        KeyCode::Backspace => {
            backspace(app);
            refresh_completions(app);
            arm_cancel(app);
        }
        KeyCode::Char(c) => {
            insert(app, c);
            refresh_completions(app);
            arm_cancel(app);
        }
        KeyCode::Enter => {
            arm_cancel(app);
            submit(app);
        }
        KeyCode::Left => {
            arm_cancel(app);
            if app.cursor > 0 {
                app.cursor -= 1;
            }
        }
        KeyCode::Right => {
            arm_cancel(app);
            if app.cursor < app.input.len() {
                app.cursor += 1;
            }
        }
        _ => {
            arm_cancel(app);
        }
    }
}

/// Esc on the chat view is the universal "let me rethink" gesture:
///   - Input non-empty: clear the input, drop completions. Status hint.
///   - Input empty: show a hint pointing to /exit. Never quits.
///   - Use /exit (or /quit, /q) to leave moneyball.
fn handle_esc(app: &mut App) {
    if !app.input.is_empty() {
        app.input.clear();
        app.cursor = 0;
        app.completions.clear();
        app.completion_idx = None;
        app.status = Some("input cleared".into());
        return;
    }
    app.status = Some("esc clears the input. use /exit to leave moneyball.".into());
}

fn arm_cancel(_app: &mut App) {
    // Kept as a no-op for now so callers don't break. Esc no longer arms a
    // quit shortcut in this build (user request: /exit is the only way out).
}

fn insert(app: &mut App, c: char) {
    app.input.insert(app.cursor, c);
    app.cursor += c.len_utf8();
}

fn backspace(app: &mut App) {
    if app.cursor == 0 {
        return;
    }
    let prev = app.input[..app.cursor].chars().next_back().unwrap();
    app.cursor -= prev.len_utf8();
    app.input.remove(app.cursor);
}

fn refresh_completions(app: &mut App) {
    if app.input.starts_with('/') {
        app.completions = completions(&app.input);
        app.completion_idx = if app.completions.is_empty() {
            None
        } else {
            Some(0)
        };
    } else {
        app.completions.clear();
        app.completion_idx = None;
    }
}

fn apply_completion(app: &mut App) {
    if let Some(i) = app.completion_idx {
        if let Some(&c) = app.completions.get(i) {
            // Replace the current token (up to cursor) with completion.
            let before = app.input[..app.cursor]
                .rfind(' ')
                .map(|n| n + 1)
                .unwrap_or(0);
            let after = app.input[app.cursor..].to_string();
            let mut new = String::with_capacity(c.len() + after.len() + (app.cursor - before));
            new.push_str(&app.input[..before]);
            new.push_str(c);
            new.push_str(&after);
            app.input = new;
            app.cursor = before + c.len();
        }
    }
}

fn submit(app: &mut App) {
    use crate::chat::cells;
    use crate::chat::Cell;
    use chrono::Local;
    let line = app.input.trim().to_string();
    app.input.clear();
    app.cursor = 0;
    app.completions.clear();
    app.completion_idx = None;
    // Clear any stale status hint from a prior command/error so the
    // bottom status line doesn't keep showing the previous result.
    app.status = None;
    if line.is_empty() {
        return;
    }

    // User prompt cell (every input becomes part of the scrollback).
    app.chat.push(Cell::UserPrompt(cells::UserPrompt {
        text: line.clone(),
        at: Local::now(),
    }));

    let mut parts = line.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "/quit" | "/exit" | "/q" => {
            app.chat
                .push(Cell::System(cells::System("exiting moneyball.".into())));
            app.quit = true;
        }
        "/brief" => {
            let started = std::time::Instant::now();
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "loading portfolio snapshot...".into(),
                streaming: false,
            }));
            app.load_brief();
            // If brief loaded, push the deterministic numbers as a tool
            // call (so the user can see the source data), then call the
            // configured LLM for an interpretation.
            if let Some(b) = &app.brief {
                let out = format_brief_as_lines(b);
                app.chat
                    .push_tool("brief", "", out, true, started.elapsed().as_millis() as u64);
                call_agent(app, BRIEF_SYSTEM_PROMPT, &build_brief_prompt(b));
            } else {
                let err = app
                    .status
                    .clone()
                    .unwrap_or_else(|| "snapshot failed".into());
                app.chat.push_tool(
                    "brief",
                    "",
                    vec![err],
                    false,
                    started.elapsed().as_millis() as u64,
                );
            }
        }
        "/setup" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "opening setup wizard.".into(),
                streaming: false,
            }));
            app.view = View::Setup(SetupState::new(app.cfg.data_root.clone()));
        }
        "/funnel" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("/funnel {} - wired in the next iteration. for now use /ask + this product name.", arg),
                streaming: false,
            }));
        }
        "/diagnose" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("/diagnose {} - wired in the next iteration.", arg),
                streaming: false,
            }));
        }
        "/ask" => {
            // /ask is now equivalent to free-form chat (the LLM is the
            // default). Strip the prefix and fall through to the free-
            // form path below.
            let question = if arg.is_empty() { line.clone() } else { arg.to_string() };
            run_freeform(app, &question);
            return;
        }
        "/snapshot" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "/snapshot --check validates that <workspace>/moneyball/history/snap/<date>/*.json match schema. wiring next.".into(),
                streaming: false,
            }));
        }
        "/ledger" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "/ledger shows prediction history. wiring next.".into(),
                streaming: false,
            }));
        }
        "/help" | "/?" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "slash commands: /brief /funnel <product> /diagnose <product> /ask <question> /snapshot /ledger /setup /quit".into(),
                streaming: false,
            }));
        }
        _ => {
            // Free-form chat. The LLM is the default; slash commands are
            // optional shortcuts. We try to load the brief so the agent
            // has portfolio context, but failure is silent (we still
            // answer the question).
            run_freeform(app, &line);
        }
    }
}

/// System prompt for the brief assistant. Persona + output contract.
const BRIEF_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You will be given 7-day per-product numbers (spend, leads, qualified leads, qualified-per-day, Rs/qualified, L->Q %, goal, gap) plus portfolio feasibility math.\n\
Produce a concise commentary that:\n\
  1. Calls out which products are at/over goal and which are under (largest gap first).\n\
  2. Identifies the best and worst Rs/qualified and what that implies for budget reallocation.\n\
  3. Notes the BEST-OBSERVED Rs/qualified vs current - is the gap from inefficient spend or just volume?\n\
  4. Flags setup-debt items if any (they cost leads).\n\
Keep it tight - 5-8 sentences, no preamble, no bullets unless the user asks.";

/// System prompt for the free-form chat agent. Same persona, but the
/// user is asking a free-form question rather than triggering /brief.
const AGENT_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You have access to a 7-day snapshot of the portfolio (per-product spend, leads, qualified leads, L->Q, goal, gap, plus feasibility math).\n\
Answer the user's question using that context. If the question can't be answered from the snapshot, say so plainly and suggest a slash command that would help (/brief, /funnel <product>, /diagnose <product>).\n\
Keep the answer focused and concrete. Cite the numbers you use. 3-6 sentences unless the user explicitly asks for more.";

/// Look up the active provider + model from `app.cfg.workspace` and run
/// a single non-streaming completion. Pushes the response (or a red
/// error cell) directly into `app.chat`. Returns the elapsed ms in
/// the success case, or `None` if the call could not be made (no
/// provider configured, runtime init failed, etc.). Used by /brief
/// AND by the free-form chat fallback.
fn call_agent(app: &mut App, system: &str, user_prompt: &str) {
    use crate::chat::cells;
    use crate::chat::Cell;
    let (provider_id, model_providers, model) = match app.cfg.workspace.as_ref() {
        Some(w) => (
            w.model_provider.clone().unwrap_or_default(),
            w.model_providers.clone(),
            w.model.clone().unwrap_or_default(),
        ),
        None => (String::new(), Default::default(), String::new()),
    };
    if provider_id.is_empty() || model.is_empty() {
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "no LLM configured - run /setup (step 4) to pick a provider + paste a key.".into(),
            streaming: false,
        }));
        return;
    }
    let provider_info = match model_providers.get(&provider_id) {
        Some(p) => p,
        None => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!(
                    "configured provider '{}' is not in the model_providers map - re-run /setup.",
                    provider_id
                ),
                streaming: false,
            }));
            return;
        }
    };
    let client = moneyball_core::LlmClient::new();
    let llm_started = std::time::Instant::now();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("llm runtime init failed: {}", e),
                streaming: false,
            }));
            return;
        }
    };
    let result = rt.block_on(client.complete(
        &provider_id,
        provider_info,
        &model,
        Some(system),
        user_prompt,
    ));
    let llm_ms = llm_started.elapsed().as_millis() as u64;
    match result {
        Ok(text) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("{} ({}ms via {})", text.trim(), llm_ms, provider_id),
                streaming: false,
            }));
        }
        Err(e) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("llm call failed: {}", e),
                streaming: false,
            }));
        }
    }
}

/// Free-form chat path. Tries to load the brief silently so the LLM has
/// portfolio context; if no snapshot exists, we still answer with just
/// the user's question. The slash commands (`/brief`, `/funnel`, etc.)
/// are convenience shortcuts that bypass the LLM dispatch and emit a
/// tool result directly. Plain text in the input bar falls into this
/// function - the LLM is the default, mirroring how codex / claude
/// code work.
fn run_freeform(app: &mut App, question: &str) {
    // Try to attach portfolio context. If load_brief fails (no snapshot,
    // no workspace) we still call the agent - the prompt just won't
    // include numbers and the agent should say so.
    let context = match &app.brief {
        Some(b) => Some(build_brief_prompt(b)),
        None => match app.cfg.snap_for(app.cfg.date.as_deref()) {
            Ok(path) => match crate::snapshot_load(&path) {
                Ok(snap) => {
                    let history =
                        brief::load_history(&app.cfg.history_dir().join("scoreboard.csv"));
                    let res = brief::compute(&snap, &app.cfg, &history);
                    app.brief = Some(res.clone());
                    Some(build_brief_prompt(&res))
                }
                Err(_) => None,
            },
            Err(_) => None,
        },
    };

    let prompt = match context {
        Some(ctx) => format!(
            "{}\n\nQuestion: {}\n\nAnswer using the snapshot context above. \
If the question can't be answered from the snapshot, say so and suggest \
a slash command that would help (/brief, /funnel <product>, /diagnose <product>).",
            ctx,
            question
        ),
        None => format!(
            "(no portfolio snapshot available - answer the question with general knowledge; suggest running /brief or /setup if a snapshot is needed)\n\nQuestion: {}",
            question
        ),
    };

    call_agent(app, AGENT_SYSTEM_PROMPT, &prompt);
}

/// Build the user-prompt context from the deterministic brief numbers.
/// The LLM sees exactly the numbers that appear in the chat tool cell.
fn build_brief_prompt(b: &brief::ProductRowsAndFeasibility) -> String {
    let mut s = String::new();
    s.push_str("Snapshot portfolio (7d window). Goal = qualified leads per day.\n\n");
    for r in &b.rows {
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}", x))
            .unwrap_or_else(|| "-".into());
        s.push_str(&format!(
            "- {}: spend Rs.{}/day, m={}, l={}, q={} ({:.2}/day), {}, L->Q {}, goal={}, gap={:.1}\n",
            r.product,
            r.spend_per_day,
            r.m7d,
            r.l7d,
            r.q7d,
            r.q_per_day,
            rs_per_q,
            l_to_q,
            r.goal,
            r.gap,
        ));
    }
    let f = &b.feasibility;
    s.push_str(&format!(
        "\nPortfolio: {:.1} q/day at Rs.{}/day = Rs.{}/q. Goal: {:.0}/day.\n",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        s.push_str(&format!(
            "To hit goal at current efficiency: Rs.{}/day ({:.1}x today).\n",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        s.push_str(&format!(
            "To hit goal at best-observed Rs.{}/q: Rs.{}/day ({:.1}x today).\n",
            b,
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if !f.open_debt.is_empty() {
        s.push_str(&format!(
            "Open setup debt: {}\n",
            f.open_debt.join(", ")
        ));
    }
    s.push_str("\nWrite the commentary.");
    s
}

fn format_brief_as_lines(b: &brief::ProductRowsAndFeasibility) -> Vec<String> {
    let mut out = Vec::new();
    // Header
    out.push("BRIEF  (7d window)".into());
    out.push(String::new());
    // Per-product block - multi-line so it fits any chat width.
    for r in &b.rows {
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}", x))
            .unwrap_or_else(|| "-".into());
        out.push(format!("  > {}", r.product));
        out.push(format!(
            "    {:>7}/d  m{:>4}  l{:>4}  q{:>3}  {:.2}/d  {}",
            r.spend_per_day, r.m7d, r.l7d, r.q7d, r.q_per_day, rs_per_q
        ));
        out.push(format!(
            "    L\u{2192}Q {:>5}   gap {:>5}",
            l_to_q,
            format!("{:.1}", r.gap)
        ));
    }
    out.push(String::new());
    let f = &b.feasibility;
    out.push(format!(
        "FEASIBILITY  {:.1} q/day @ Rs.{}/day = Rs.{}/q  \u{00B7}  goal {:.0}/day",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        out.push(format!(
            "  required @ current:  Rs.{}/day ({:.1}x)",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        out.push(format!(
            "  required @ best Rs.{}/q: Rs.{}/day ({:.1}x)",
            b,
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    let suffix = if f.open_debt.is_empty() {
        String::new()
    } else {
        format!(" ({})", f.open_debt.join(", "))
    };
    out.push(format!("  setup debt: {}{}", f.open_debt.len(), suffix));
    out
}

// ---------- setup-wizard keys ----------

fn handle_setup_key(app: &mut App, mut state: SetupState, k: KeyEvent) {
    // Substep 1 of step 1 is a list-selection mode with its own keymap.
    if state.step == 1 && state.meta_substep == 1 {
        handle_select_keys(&mut state, k);
        app.view = View::Setup(state);
        return;
    }
    // Step 4 picker substeps (provider pick, model pick) have their own
    // arrow-key navigation.
    if state.step == 4 && (state.llm_substep == 0 || state.llm_substep == 2) {
        handle_llm_picker_keys(&mut state, k);
        app.view = View::Setup(state);
        return;
    }
    // Substep 2 of step 1 is the rename buffer. Esc goes back to substep 1.
    if state.step == 1 && state.meta_substep == 2 && k.code == KeyCode::Esc {
        state.meta_substep = 1;
        state.meta_rename_input.clear();
        state.error = None;
        app.view = View::Setup(state);
        return;
    }
    // Char / Enter / Backspace fall through to the default handler below.
    match k.code {
        KeyCode::Esc => {
            // Esc clears the active input buffer (if any). It never quits the
            // wizard or moneyball. /exit is the only way out.
            let cleared = match state.step {
                0 if !state.workspace_path.is_empty() => {
                    state.workspace_path.clear();
                    true
                }
                1 => match state.meta_substep {
                    0 if !state.meta_input.is_empty() => {
                        state.meta_input.clear();
                        true
                    }
                    _ => false,
                },
                2 if !state.product_input.is_empty() => {
                    state.product_input.clear();
                    true
                }
                3 if !state.goals_input.is_empty() => {
                    state.goals_input.clear();
                    true
                }
                _ => false,
            };
            if !cleared {
                state.error = Some("esc clears input. use /exit to leave moneyball.".into());
            } else {
                state.error = None;
            }
        }
        KeyCode::Enter => {
            advance_setup(app, &mut state);
        }
        KeyCode::Backspace => {
            backspace_setup(&mut state);
        }
        KeyCode::Char(c) => {
            insert_setup(&mut state, c);
        }
        _ => {}
    }
    // advance_save may have transitioned us out to View::Brief. Don't clobber that.
    if app.view != View::Brief {
        app.view = View::Setup(state);
    }
}

/// Keyboard handler for the multi-account selection list (step 1 substep 1).
fn handle_select_keys(s: &mut SetupState, k: KeyEvent) {
    let n = s.meta_discovered.len();
    if n == 0 {
        return;
    }
    // Visible rows must match the renderer's visible_rows constant below.
    const VISIBLE_ROWS: usize = 12;
    match k.code {
        KeyCode::Up => {
            if s.meta_highlight > 0 {
                s.meta_highlight -= 1;
                if s.meta_highlight < s.meta_scroll {
                    s.meta_scroll = s.meta_highlight;
                }
            }
        }
        KeyCode::Down => {
            if s.meta_highlight + 1 < n {
                s.meta_highlight += 1;
                if s.meta_highlight >= s.meta_scroll + VISIBLE_ROWS {
                    s.meta_scroll = s.meta_highlight + 1 - VISIBLE_ROWS;
                }
            }
        }
        KeyCode::PageUp => {
            s.meta_highlight = s.meta_highlight.saturating_sub(VISIBLE_ROWS);
            s.meta_scroll = s.meta_scroll.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::PageDown => {
            s.meta_highlight = (s.meta_highlight + VISIBLE_ROWS).min(n - 1);
            s.meta_scroll = (s.meta_highlight + 1).saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Home => {
            s.meta_highlight = 0;
            s.meta_scroll = 0;
        }
        KeyCode::End => {
            s.meta_highlight = n - 1;
            s.meta_scroll = n.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Char(' ') => {
            s.meta_selections[s.meta_highlight] = !s.meta_selections[s.meta_highlight];
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let any = s.meta_selections.iter().any(|&b| b);
            for sel in s.meta_selections.iter_mut() {
                *sel = !any;
            }
        }
        KeyCode::Enter => {
            let chosen: Vec<usize> = (0..n).filter(|&i| s.meta_selections[i]).collect();
            if chosen.is_empty() {
                s.error = Some("select at least one account (Space to toggle, 'a' for all)".into());
                return;
            }
            s.meta_selected = chosen;
            s.meta_substep = 2;
            s.error = None;
        }
        KeyCode::Esc => {
            // "back" - return to substep 0 (token paste) WITHOUT nuking the
            // user's progress. The discovered account list and the per-row
            // checkbox state survive so the user doesn't have to re-select
            // after re-validating. The token itself was already cleared
            // from `meta_input` after validation, so we restore a masked
            // placeholder (N bullets of `meta_token_len`) so the input
            // box doesn't look empty.
            s.meta_input = "\u{2022}".repeat(s.meta_token_len);
            s.meta_rename_input.clear();
            s.meta_substep = 0;
            s.error = None;
        }
        _ => {}
    }
}

/// Arrow-key navigation for step 4 picker substeps (provider pick +
/// model pick). Step 4 substep 1 (paste) and substep 3 (custom URL/model)
/// accept free text via `insert_setup`, so this only handles the picker
/// substeps.
fn handle_llm_picker_keys(s: &mut SetupState, k: KeyEvent) {
    // Visible rows in step 4 picker lists. Keep in sync with the renderer.
    const VISIBLE_ROWS: usize = 6;
    let total = match s.llm_substep {
        0 => built_in_presets().len() + 1, // presets + custom
        2 => {
            let preset = if s.llm_provider_id == "custom" {
                ModelProviderInfo {
                    name: "custom".into(),
                    base_url: s.llm_url.clone(),
                    ..Default::default()
                }
            } else {
                s.llm_provider.clone().unwrap_or_else(ModelProviderInfo::openai)
            };
            models_for(&preset).len()
        }
        _ => 0,
    };
    if total == 0 {
        return;
    }
    match k.code {
        KeyCode::Up => {
            if s.llm_highlight > 0 {
                s.llm_highlight -= 1;
                if s.llm_highlight < s.llm_scroll {
                    s.llm_scroll = s.llm_highlight;
                }
            }
        }
        KeyCode::Down => {
            if s.llm_highlight + 1 < total {
                s.llm_highlight += 1;
                if s.llm_highlight >= s.llm_scroll + VISIBLE_ROWS {
                    s.llm_scroll = s.llm_highlight + 1 - VISIBLE_ROWS;
                }
            }
        }
        KeyCode::Home => {
            s.llm_highlight = 0;
            s.llm_scroll = 0;
        }
        KeyCode::End => {
            s.llm_highlight = total.saturating_sub(1);
            s.llm_scroll = total.saturating_sub(VISIBLE_ROWS);
        }
        _ => {}
    }
}

fn insert_setup(s: &mut SetupState, c: char) {
    match s.step {
        0 => {
            s.workspace_path.push(c);
        }
        1 => {
            meta_insert(s, c);
        }
        2 => {
            s.product_input.push(c);
        }
        3 => {
            s.goals_input.push(c);
        }
        4 => {
            // Step 4 substeps that accept free-text: paste-key (1) and
            // custom URL/model (3). Picker substeps (0, 2) ignore chars.
            match s.llm_substep {
                1 => s.llm_input.push(c),
                3 => {
                    // First fill URL, then model. The first Enter advances
                    // to the model phase; we toggle which buffer is active
                    // by checking whether URL is set.
                    if s.llm_url.is_empty() {
                        s.llm_url.push(c);
                    } else {
                        s.llm_model.push(c);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn backspace_setup(s: &mut SetupState) {
    match s.step {
        0 => {
            s.workspace_path.pop();
        }
        1 => {
            meta_backspace(s);
        }
        2 => {
            s.product_input.pop();
        }
        3 => {
            s.goals_input.pop();
        }
        4 => match s.llm_substep {
            1 => {
                s.llm_input.pop();
            }
            3 => {
                if s.llm_model.is_empty() {
                    s.llm_url.pop();
                } else {
                    s.llm_model.pop();
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn meta_insert(s: &mut SetupState, c: char) {
    match s.meta_substep {
        0 => {
            s.meta_input.push(c);
        }
        // substep 1 is keyboard-driven (Up/Down/Space/'a'/Enter); ignore chars.
        1 => {}
        2 => {
            s.meta_rename_input.push(c);
        }
        _ => {}
    }
}

fn meta_backspace(s: &mut SetupState) {
    match s.meta_substep {
        0 => {
            s.meta_input.pop();
        }
        // substep 1 ignored.
        1 => {}
        2 => {
            s.meta_rename_input.pop();
        }
        _ => {}
    }
}

fn advance_setup(app: &mut App, s: &mut SetupState) {
    s.error = None;
    match s.step {
        0 => advance_workspace(app, s),
        1 => advance_meta(app, s),
        2 => advance_products(s),
        3 => {
            // Goals is now step 3; advance goes to LLM (step 4), not save.
            s.step = 4;
            s.llm_substep = 0;
            s.llm_highlight = 0;
            s.llm_scroll = 0;
        }
        4 => advance_llm(app, s),
        _ => {}
    }
}

fn advance_workspace(app: &mut App, s: &mut SetupState) {
    let p = PathBuf::from(s.workspace_path.trim());
    if !p.is_dir() {
        match std::fs::create_dir_all(&p) {
            Ok(()) => {}
            Err(e) => {
                s.error = Some(format!("can't create {}: {}", p.display(), e));
                return;
            }
        }
    }
    std::fs::create_dir_all(p.join("moneyball")).ok();
    app.cfg.data_root = p;
    s.step = 1;
}

fn advance_meta(_app: &mut App, s: &mut SetupState) {
    match s.meta_substep {
        // Substep 0: paste token. Mandatory.
        0 => {
            let raw = s.meta_input.trim();
            if raw.is_empty() {
                s.error = Some("a Meta Marketing API access token is required (paste it above)".into());
                return;
            }
            // Validate token + list ad accounts.
            if let Err(e) = validate_token(raw) {
                s.error = Some(format!("token rejected: {}", e));
                return;
            }
            match list_ad_accounts(raw) {
                Ok(accounts) => {
                    if accounts.is_empty() {
                        s.error = Some("token is valid but no ad accounts found (need ads_read + an ad account assigned to you)".into());
                        return;
                    }
                    // Persist token to keychain immediately; we'll move it out of memory after.
                    if let Err(e) = moneyball_core::secrets::store_meta_token(raw) {
                        s.error = Some(format!("token accepted but keychain write failed: {}", e));
                        return;
                    }
                    // Capture token length for the collapsed summary ("••••• (N chars)")
                    // BEFORE clearing the buffer.
                    s.meta_token_len = s.meta_input.chars().count();
                    s.meta_discovered = accounts;
                    s.meta_selections = vec![false; s.meta_discovered.len()];
                    s.meta_highlight = 0;
                    s.meta_scroll = 0;
                    s.meta_input.clear();
                    s.meta_substep = 1;
                }
                Err(e) => {
                    s.error = Some(format!("couldn't list ad accounts: {}", e));
                }
            }
        }
        // Substep 1: multi-select list. Enter handler lives in handle_select_keys.
        // (advance_setup is called for Enter; substep 1's Enter is handled there.)
        1 => {
            // Shouldn't usually hit this path (Enter is routed via handle_select_keys).
            // Fall through: confirm whatever is currently selected.
            let chosen: Vec<usize> = (0..s.meta_discovered.len())
                .filter(|&i| s.meta_selections[i])
                .collect();
            if chosen.is_empty() {
                s.error = Some("select at least one account (Space to toggle, 'a' for all)".into());
                return;
            }
            s.meta_selected = chosen;
            s.meta_substep = 2;
        }
        // Substep 2: rename overrides (or blank = use account names).
        2 => {
            let raw = s.meta_rename_input.trim();
            // Build overrides from input.
            let overrides = parse_renames(raw);
            if let Err(e) = overrides {
                s.error = Some(e);
                return;
            }
            let overrides = overrides.unwrap_or_default();
            // Build final products list.
            let mut new_products: Vec<(String, String)> = Vec::new();
            for (i, &idx) in s.meta_selected.iter().enumerate() {
                let acct = &s.meta_discovered[idx];
                // Default to the Meta account's display name; let the user
                // override via "1=Name 2=OtherName" syntax in the rename input.
                let name = overrides
                    .get(&(idx + 1))
                    .cloned()
                    .unwrap_or_else(|| acct.name.clone());
                let id = moneyball_core::meta::account_id_for_storage(&acct.id);
                if new_products.iter().any(|(n, _)| n == &name) {
                    s.error = Some(format!("duplicate product name '{}'", name));
                    return;
                }
                new_products.push((name, id));
                let _ = i;
            }
            // If user explicitly typed 'all' and used demo, skip auto-fills.
            s.products = new_products;
            s.meta_connected = true;
            s.error = None;
            s.meta_rename_input.clear();
            s.step = 3; // skip the manual "add products" step; go to goals.
        }
        _ => {}
    }
}

fn parse_renames(
    raw: &str,
) -> std::result::Result<std::collections::HashMap<usize, String>, String> {
    let mut out = std::collections::HashMap::new();
    for part in raw.split_whitespace() {
        let (idx_s, name) = part
            .split_once('=')
            .ok_or_else(|| format!("bad rename '{}': expected N=Name", part))?;
        let idx: usize = idx_s
            .parse()
            .map_err(|_| format!("bad index '{}'", idx_s))?;
        if idx < 1 {
            return Err(format!("index must be >= 1 (got {})", idx));
        }
        if name.is_empty() {
            return Err(format!("empty name at index {}", idx));
        }
        out.insert(idx, name.to_string());
    }
    Ok(out)
}

fn advance_products(s: &mut SetupState) {
    let raw = s.product_input.trim();
    if raw.is_empty() {
        if s.products.is_empty() {
            s.error = Some("add at least one product (try 'demo' to load Fincity example)".into());
            return;
        }
        s.step = 3;
        return;
    }
    if raw.eq_ignore_ascii_case("demo") {
        s.products = DEMO_PRODUCTS
            .iter()
            .map(|(n, a)| (n.to_string(), a.to_string()))
            .collect();
        s.product_input.clear();
        return;
    }
    let parts: Vec<&str> = raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        [name, acct] => {
            if !acct.chars().all(|c| c.is_ascii_digit()) || acct.len() < 6 {
                s.error = Some(format!(
                    "ad account '{}' should be digits only (15-20 chars)",
                    acct
                ));
                return;
            }
            if s.products.iter().any(|(n, _)| n == name) {
                s.error = Some(format!("product '{}' already added", name));
                return;
            }
            s.products.push((name.to_string(), acct.to_string()));
            s.product_input.clear();
        }
        _ => {
            s.error = Some(format!("expected 'Name AdAccount' - got '{}'", raw));
        }
    }
}

/// Parse the goals step. Returns false if validation fails; sets s.error.
fn advance_goals(s: &mut SetupState) -> bool {
    let raw = s.goals_input.trim();
    if raw.is_empty() {
        // Blank input -> defaults of 10 for every product.
        s.goals_input = s
            .products
            .iter()
            .map(|(n, _)| format!("{}=10", n))
            .collect::<Vec<_>>()
            .join(" ");
    }
    match parse_goals(&s.products, &s.goals_input) {
        Ok(_) => true,
        Err(e) => {
            s.error = Some(e);
            false
        }
    }
}

fn advance_save(app: &mut App, s: &mut SetupState) {
    if !advance_goals(s) {
        return;
    } // validation failed - keep user on goals step
    let products: Vec<_> = s
        .products
        .iter()
        .map(|(n, a)| moneyball_core::config::Product {
            name: n.clone(),
            ad_account: a.clone(),
        })
        .collect();
    let goals_map = parse_goals(&s.products, &s.goals_input).unwrap_or_default();
    // target_rs_per_q is intentionally NOT asked during setup - it's a
    // derived/observed metric per product, not a hardcoded universal value.
    // Stored as None; the advisor derives it from observed performance.
    let cfg = WorkspaceConfig {
        products,
        goals: goals_map,
        target_rs_per_q: None,
        crm: Default::default(),
        model_provider: s.llm_provider_id.clone().into(),
        model: s.llm_model.clone().into(),
        model_providers: llm_providers_map(s),
    };
    if let Err(e) = cfg.save(&app.cfg.data_root) {
        s.error = Some(format!("save failed: {}", e));
        return;
    }
    // If user skipped Meta, scrub any stale token from keychain.
    if !s.meta_connected {
        let _ = moneyball_core::secrets::clear_meta_token();
    }
    app.cfg.workspace = Some(cfg);
    app.view = View::Brief;
    app.load_brief();
}

/// Build the `model_providers` HashMap to persist. Includes the active
/// provider entry plus any built-in presets that the user might want
/// later when switching providers from /settings.
fn llm_providers_map(s: &SetupState) -> std::collections::HashMap<String, ModelProviderInfo> {
    let mut map = std::collections::HashMap::new();
    for (id, p) in built_in_presets() {
        map.insert(id.to_string(), p);
    }
    if let Some(active) = &s.llm_provider {
        map.insert(s.llm_provider_id.clone(), active.clone());
    }
    map
}

/// Step 4 (LLM) advance logic. Dispatches on `llm_substep`:
///   0 -> provider pick (Enter on highlighted row)
///   1 -> API key paste -> writes to keychain, advances to model pick
///   2 -> model pick -> persists + transitions to advance_save
///   3 -> custom URL/model entry -> save
fn advance_llm(app: &mut App, s: &mut SetupState) {
    match s.llm_substep {
        0 => {
            // Pick the highlighted preset or "custom".
            let presets = built_in_presets();
            let total = presets.len() + 1;
            if s.llm_highlight >= total {
                s.error = Some("invalid provider selection".into());
                return;
            }
            if s.llm_highlight < presets.len() {
                let (id, provider) = &presets[s.llm_highlight];
                s.llm_provider_id = (*id).to_string();
                s.llm_provider = Some(provider.clone());
            } else {
                s.llm_provider_id = "custom".into();
                s.llm_provider = Some(ModelProviderInfo {
                    name: "custom".into(),
                    base_url: String::new(),
                    ..Default::default()
                });
            }
            s.llm_substep = 1;
            s.error = None;
        }
        1 => {
            // Validate that a key was pasted; write to keychain.
            let key = s.llm_input.trim();
            if key.is_empty() {
                s.error = Some("API key is required to continue".into());
                return;
            }
            if let Err(e) = moneyball_core::secrets::store_llm_key(&s.llm_provider_id, key) {
                s.error = Some(format!("keychain write failed: {}", e));
                return;
            }
            s.llm_key_len = key.chars().count();
            s.llm_input.clear();
            // For "custom", ask for the base URL next (reuse substep 1 input).
            if s.llm_provider_id == "custom" {
                s.llm_substep = 3;
            } else {
                s.llm_substep = 2;
                s.llm_highlight = 0;
            }
            s.error = None;
        }
        2 => {
            // Pick the highlighted model.
            let preset = if s.llm_provider_id == "custom" {
                ModelProviderInfo {
                    name: "custom".into(),
                    base_url: s.llm_url.clone(),
                    ..Default::default()
                }
            } else {
                s.llm_provider.clone().unwrap_or_else(ModelProviderInfo::openai)
            };
            let models = models_for(&preset);
            if s.llm_highlight >= models.len() {
                s.error = Some("invalid model selection".into());
                return;
            }
            s.llm_model = models[s.llm_highlight].to_string();
            // If user picked the "custom" sentinel, fall through to a free-text
            // URL substep instead of saving here.
            if s.llm_model == "custom" && s.llm_provider_id == "custom" {
                s.llm_model.clear();
                s.llm_substep = 3;
                return;
            }
            advance_save(app, s);
        }
        3 => {
            // Custom substep: collects base URL + (optionally) model slug.
            if s.llm_provider_id == "custom" && s.llm_url.trim().is_empty() {
                s.error = Some("enter the provider's base URL".into());
                return;
            }
            if s.llm_model.is_empty() {
                s.error = Some("enter the model slug".into());
                return;
            }
            // Refresh the provider entry with the URL the user typed.
            if let Some(p) = s.llm_provider.as_mut() {
                p.base_url = s.llm_url.trim().to_string();
            }
            advance_save(app, s);
        }
        _ => {}
    }
}

fn parse_goals(
    products: &[(String, String)],
    s: &str,
) -> std::result::Result<std::collections::HashMap<String, f64>, String> {
    let mut out = std::collections::HashMap::new();
    let known: std::collections::HashSet<&str> = products.iter().map(|(n, _)| n.as_str()).collect();

    // Smart parser: scan for the next '=' which separates a product name
    // from its number. Multi-word product names work because the name is
    // everything up to that '=' (trimmed). Separators between pairs can be
    // any combination of spaces and/or commas.
    let mut rest = s;
    while !rest.trim().is_empty() {
        // Skip leading whitespace/commas between pairs
        let trimmed = rest.trim_start();
        if trimmed.len() != rest.len() {
            rest = trimmed;
        }
        if rest.is_empty() {
            break;
        }

        // Find the '=' that ends this product's name.
        let eq = rest.find('=').ok_or_else(|| {
            let snippet: String = rest.chars().take(40).collect();
            format!(
                "expected 'ProdName=Number', no '=' found in: '{}...'",
                snippet
            )
        })?;

        // Name = chars from start to '=' (trim trailing whitespace).
        let name = rest[..eq].trim();
        if name.is_empty() {
            return Err("empty product name before '='".into());
        }
        if !known.contains(name) {
            return Err(format!(
                "unknown product '{}' (known: {:?})",
                name,
                products.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
            ));
        }

        // Number = chars after '=' until next whitespace/comma or end-of-input.
        let after_eq = &rest[eq + 1..];
        let val_end = after_eq
            .find(|c: char| c.is_whitespace() || c == ',')
            .unwrap_or(after_eq.len());
        let val = after_eq[..val_end].trim();

        let v: f64 = val.parse().map_err(|_| {
            format!(
                "not a number: '{}' in '{}={}{}'",
                val,
                name,
                val,
                after_eq[val_end..].chars().take(20).collect::<String>()
            )
        })?;
        if v <= 0.0 || v > 1000.0 {
            return Err(format!("goal {} out of range (1-1000) for '{}'", v, name));
        }
        out.insert(name.to_string(), v);

        // Advance past the number (and any whitespace/comma immediately after).
        rest = after_eq[val_end..].trim_start();
    }

    // Fill in defaults for any missing products so partial input still saves.
    for (n, _) in products {
        out.entry(n.clone()).or_insert(10.0);
    }
    Ok(out)
}

// ---------- render ----------

fn render(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    // Layout (top to bottom):
    //   logo (2) | context (1) | body (Min) | commands (0 - hidden) | input (3)
    //
    // For the setup wizard, body is the wizard panel (no commands shown).
    // For the chat view (default), body is the chat log + commands + input.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // logo
            Constraint::Length(1), // context bar
            Constraint::Min(6),    // body
        ])
        .split(area);

    // Logo + context bar.
    f.render_widget(Paragraph::new(crate::widgets::logo()), outer[0]);
    let status = if app.brief.is_some() {
        crate::widgets::Status::Ready
    } else if app.cfg.has_workspace() {
        crate::widgets::Status::NoData
    } else {
        crate::widgets::Status::Idle
    };
    let ctx_line = crate::widgets::context_line(
        &app.cfg.data_root.display().to_string(),
        app.snap_date.as_deref(),
        status,
    );
    f.render_widget(Paragraph::new(ctx_line), outer[1]);

    match &app.view {
        View::Setup(s) => render_setup(f, outer[2], s),
        View::Brief => render_chat_view(f, outer[2], app),
    }
}

/// Chat-style body: scrollable log + compact command hint + input bar.
fn render_chat_view(f: &mut ratatui::Frame, area: Rect, app: &App) {
    // Chat-style bottom stack (matches codex / claude code / pi):
    //   1. inline completion ghost (only when input is `/`-prefixed)
    //   2. thin horizontal separator
    //   3. the input line itself (single bar, no boxed title)
    //   4. one-line keybinding caption
    //   5. status hint when something notable happened
    let has_completion = !app.completions.is_empty();
    let completion_h: u16 = if has_completion { 1 } else { 0 };
    let status_h: u16 = if app.status.is_some() { 1 } else { 0 };

    // Body height: count actual content lines, clamp to a reasonable
    // window. Avoids huge blank bodies when chat has few cells
    // (previously Min(5) expanded to fill the screen, leaving 19+
    // empty rows). Capped at 20 so a 200-line backscroll doesn't push
    // the input off-screen.
    let content_lines = count_chat_lines(app, area.width) as u16;
    let body_h = content_lines.clamp(5, 20);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(body_h),       // chat scrollback (natural height)
            Constraint::Length(completion_h), // slash-command completions
            Constraint::Length(1),            // separator
            Constraint::Length(1),            // input line
            Constraint::Length(1),            // keybinding caption
            Constraint::Length(status_h),     // status hint
        ])
        .split(area);

    // 1. Chat log
    let log_lines: Vec<Line<'static>> = {
        let width = chunks[0].width.max(20);
        let height = chunks[0].height.max(5);
        app.chat.render(width, height)
    };
    f.render_widget(Paragraph::new(log_lines), chunks[0]);

    // 2. Inline completion ghost
    if has_completion {
        let mut spans: Vec<Span<'static>> = vec![Span::styled("  ", Style::default())];
        for (i, c) in app.completions.iter().enumerate() {
            let style = if Some(i) == app.completion_idx {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("{} ", c), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), chunks[1]);
    }

    // 3. Thin horizontal separator (above the input line)
    let width = area.width as usize;
    let sep: String = std::iter::repeat_n('\u{2500}', width.max(10)).collect();
    f.render_widget(
        Paragraph::new(Line::from(sep)).style(Style::default().fg(Color::DarkGray)),
        chunks[2 + (if has_completion { 1 } else { 0 })],
    );
    // Actually recompute indices cleanly:
    // chunks[0] = body
    // chunks[1] = completion (if has_completion)
    // chunks[2] = separator
    // chunks[3] = input
    // chunks[4] = caption
    // chunks[5] = status
    // (above logic with `chunks[2 - 0 + ...` was wrong; recompute via explicit indices below.)

    // 4. The input line (single line, no border, no title)
    let sep_idx = 2;
    let input_idx = 3;
    let caption_idx = 4;
    let status_idx = 5;
    let _ = sep; // already drawn; re-draw correctly:
    f.render_widget(
        Paragraph::new(Line::from(sep_str(width))).style(Style::default().fg(Color::DarkGray)),
        chunks[sep_idx],
    );

    let placeholder = "ask moneyball about your portfolio or type / for commands";
    let prompt_line = if app.input.is_empty() {
        Line::from(vec![
            Span::styled(
                "\u{276F} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                placeholder,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled("\u{2588}", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let before = app.input[..app.cursor].to_string();
        let after = app.input[app.cursor..].to_string();
        Line::from(vec![
            Span::styled(
                "\u{276F} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
            Span::styled(after, Style::default().fg(Color::White)),
        ])
    };
    f.render_widget(Paragraph::new(prompt_line), chunks[input_idx]);

    // 5. Keybinding caption (no borders)
    let caption = Line::from(Span::styled(
        "  \u{21B5} send  \u{00B7}  esc clear input  \u{00B7}  \u{21E5} complete  \u{00B7}  /exit to quit",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(caption), chunks[caption_idx]);

    // 6. Status hint (when something notable happened)
    if status_h > 0 {
        if let Some(msg) = &app.status {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {}", msg),
                    Style::default().fg(Color::Yellow),
                ))),
                chunks[status_idx],
            );
        }
    }
}

fn sep_str(width: usize) -> String {
    std::iter::repeat_n('\u{2500}', width.max(10)).collect()
}

/// Count the visual lines the chat log will render in `area.width` columns.
/// Used to size the body chunk so it doesn't expand to fill the whole
/// screen with empty rows when the chat has only a few cells.
fn count_chat_lines(app: &App, width: u16) -> usize {
    use crate::chat::ChatCell;
    let mut n = 0;
    for cell in &app.chat.cells {
        n += 1; // the blank separator line between cells
        n += cell.desired_height(width) as usize;
    }
    n
}

fn render_setup(f: &mut ratatui::Frame, area: Rect, s: &SetupState) {
    // Codex-style vertical stack (no boxed modal):
    //   step indicator strip  (1 row)
    //   completed-step lines  (1 row each)
    //   active-step panel     (variable, plain text + rounded input border)
    //   error/footer hint     (1-2 rows, single dim line)
    //
    // The previous boxed-modal layout clipped the input prompt on step 2
    // (products) and step 3 (goals) once the user had >= 3 products.
    // Composing manually lets each section size to its actual content.
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.extend(render_step_indicator(s));
    lines.push(Line::from(""));
    lines.extend(render_completed_steps(s));
    lines.extend(render_active_step(s));

    // Reserve 2 rows for footer (error line + key-hint line).
    let hint_h: u16 = if s.error.is_some() { 2 } else { 1 };
    let content_h = area.height.saturating_sub(hint_h).max(3);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(content_h), Constraint::Length(hint_h)])
        .split(area);

    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        chunks[0],
    );

    // Footer: error (if any) + key hints as plain dim lines.
    let mut footer_lines = Vec::new();
    if let Some(e) = &s.error {
        footer_lines.push(Line::from(Span::styled(
            format!("  ! {}", e),
            Style::default().fg(Color::Red),
        )));
    }
    footer_lines.push(Line::from(Span::styled(
        "  enter next  \u{00B7}  esc back  \u{00B7}  ctrl+c quit",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(footer_lines), chunks[1]);
}

/// Top progress strip: `1 \u{00B7} workspace   2 \u{00B7} token   ...` with the current step highlighted.
/// Labels match the collapsed-step summaries in `render_completed_steps`.
fn render_step_indicator(s: &SetupState) -> Vec<Line<'static>> {
    let total = 5;
    let cur = s.step.min(total - 1);
    let labels = ["workspace", "token", "products", "goals", "model"];
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        "  ",
        Style::default(),
    )];
    for (i, label) in labels.iter().enumerate() {
        let is_current = i == cur;
        let style = if is_current {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let marker = if is_current { "\u{25B8}" } else { " " };
        spans.push(Span::styled(
            format!("{} {} \u{00B7} {}",
                marker,
                i + 1,
                label),
            style,
        ));
        if i + 1 < total {
            spans.push(Span::styled("   ", Style::default().fg(Color::DarkGray)));
        }
    }
    vec![Line::from(spans)]
}

/// One-line summaries for completed steps (workspace / token / accounts / products / goals).
fn render_completed_steps(s: &SetupState) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut i = 0usize;
    if s.step >= 1 {
        // step 0 (workspace) is completed
        out.push(Line::from(Span::styled(
            format!("  \u{2713} 1 \u{00B7} workspace         {}", s.workspace_path),
            Style::default().fg(Color::Green),
        )));
        i = 1;
    }
    if s.step >= 2 && s.meta_connected {
        let bullets = "\u{2022}".repeat(s.meta_token_len.min(10));
        let n = s.meta_token_len;
        out.push(Line::from(Span::styled(
            format!("  \u{2713} 2 \u{00B7} meta token         {} ({} chars)", bullets, n),
            Style::default().fg(Color::Green),
        )));
        i = 2;
    }
    if s.step >= 3 {
        let n = s.products.len();
        out.push(Line::from(Span::styled(
            format!("  \u{2713} 3 \u{00B7} products            {} configured", n),
            Style::default().fg(Color::Green),
        )));
        i = 3;
    }
    if s.step >= 4 {
        let provider = s.llm_provider_id.as_str();
        let model = if s.llm_model.is_empty() {
            "?".to_string()
        } else {
            s.llm_model.clone()
        };
        let n = s.llm_key_len;
        let bullets = "\u{2022}".repeat(n.min(10));
        out.push(Line::from(Span::styled(
            format!("  \u{2713} 4 \u{00B7} model               {} \u{00B7} {} ({})", provider, model, bullets),
            Style::default().fg(Color::Green),
        )));
    }
    let _ = i;
    out
}

/// Active step's content as plain lines. Each step helper returns a `Vec<Line>`
/// so we don't pay the cost of a `Paragraph` block just to compose it.
fn render_active_step(s: &SetupState) -> Vec<Line<'static>> {
    match s.step {
        0 => render_step_workspace(s),
        1 => render_step_meta(s),
        2 => render_step_products(s),
        3 => render_step_goals(s),
        4 => render_step_llm(s),
        _ => vec![Line::from("done")],
    }
}

fn styled_title(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Rounded cyan border opener for the active input field. The interior
/// width is hardcoded to 60 chars; the closing border uses the same
/// constant so they line up visually. Title appears on the top border
/// (e.g. "╭ workspace path ────╮").
const INPUT_BOX_INNER: usize = 60;

fn input_box_open(title: &str) -> Line<'static> {
    let title_len = title.chars().count();
    // Interior: " " + title + " " + "─"×fill  (total = INPUT_BOX_INNER)
    let fill = INPUT_BOX_INNER.saturating_sub(title_len + 2);
    Line::from(Span::styled(
        format!(
            "  \u{256D} {} {} \u{256E}",
            title,
            "\u{2500}".repeat(fill),
        ),
        Style::default().fg(Color::Cyan),
    ))
}

/// Closing border line for the active input field. Width matches
/// `input_box_open` so the corners line up.
fn input_box_close() -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "  \u{2570}{}\u{256F}",
            "\u{2500}".repeat(INPUT_BOX_INNER)
        ),
        Style::default().fg(Color::Cyan),
    ))
}

fn render_step_workspace(s: &SetupState) -> Vec<Line<'static>> {
    let mut lines = vec![
        styled_title("Workspace path"),
        Line::from(""),
        Line::from("  This is where moneyball will read snapshots + write ledger/runs."),
        Line::from("  The directory will be auto-created if it does not exist."),
        Line::from(""),
    ];
    // Rounded cyan border around the input field (codex auth.rs pattern).
    lines.push(input_box_open("workspace path"));
    lines.push(Line::from(Span::styled(
        format!("  \u{2502}  > {}\u{2588}", s.workspace_path),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(input_box_close());
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  enter to accept \u{00B7} backspace to edit \u{00B7} esc clears input",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_step_meta(s: &SetupState) -> Vec<Line<'static>> {
    match s.meta_substep {
        0 => {
            let n = s.meta_input.chars().count();
            let mut lines = vec![
                styled_title("Meta API access token"),
                Line::from(""),
                Line::from("  Paste a long-lived Meta Marketing API access token"),
                Line::from("  (the one with ads_read permission; get one at"),
                Line::from("  developers.facebook.com -> Tools -> Marketing API)."),
                Line::from(""),
            ];
            // Rounded cyan border around the token input.
            lines.push(input_box_open("meta token"));
            // Pad masked value to a fixed visual width so the box stays
            // rectangular even when the token is short.
            let masked: String = "\u{2022}".repeat(n.min(48));
            let suffix: String = if n > 48 { "+".into() } else { String::new() };
            lines.push(Line::from(Span::styled(
                format!("  \u{2502}  > {}{}\u{2588}", masked, suffix),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            lines.push(input_box_close());
            lines.push(Line::from(Span::styled(
                format!("  ({} chars)", n),
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Token is saved to OS keychain - never written to disk in plaintext.",
                Style::default().fg(Color::DarkGray),
            )));
            lines
        }
        1 => {
            // Multi-select list with scroll. Visible rows must match VISIBLE_ROWS
            // in handle_select_keys.
            const VISIBLE_ROWS: usize = 12;
            let n = s.meta_discovered.len();
            let selected = s.meta_selections.iter().filter(|&&b| b).count();
            let mut lines = vec![
                styled_title(&format!("Select ad accounts ({} of {} chosen)",
                    selected, n)),
                Line::from(""),
                Line::from(Span::styled(
                    "  \u{2191}\u{2193}/PgUp/PgDn move  Space=toggle  a=all/none  Enter=confirm  Esc=back",
                    Style::default().fg(Color::DarkGray))),
                Line::from(""),
            ];
            let end = (s.meta_scroll + VISIBLE_ROWS).min(n);
            let start = s.meta_scroll.min(end);
            for i in start..end {
                let a = &s.meta_discovered[i];
                let status: &'static str = match a.account_status {
                    Some(1) => "ACTIVE",
                    Some(2) => "DISABLED",
                    Some(3) => "UNSETTLED",
                    Some(9) => "PENDING_RISK_REVIEW",
                    Some(101) => "PENDING_SETUP",
                    Some(_) => "OTHER",
                    None => "?",
                };
                let checkbox = if s.meta_selections[i] { "[x]" } else { "[ ]" };
                let marker = if i == s.meta_highlight { "\u{25B8}" } else { " " };
                let text = format!(
                    "  {} {} [{:>2}] {} - {} ({})",
                    marker,
                    checkbox,
                    i + 1,
                    a.id,
                    a.name,
                    status
                );
                let style = if i == s.meta_highlight {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if s.meta_selections[i] {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
            if end < n {
                lines.push(Line::from(Span::styled(
                    format!("  ... {} more below (PgDn to scroll)", n - end),
                    Style::default().fg(Color::DarkGray),
                )));
            } else if start > 0 {
                lines.push(Line::from(Span::styled(
                    "  ... PgUp to scroll up",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines
        }
        2 => {
            let mut lines = vec![
                styled_title("Name your products (optional)"),
                Line::from(""),
                Line::from("  Defaults: each product uses the account's display name."),
                Line::from("  To rename, type e.g.  1=BrandName 3=OtherName"),
                Line::from(Span::styled(
                    "  Press Enter on blank line to keep defaults.",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(""),
            ];
            for (i, &idx) in s.meta_selected.iter().enumerate() {
                let a = &s.meta_discovered[idx];
                lines.push(Line::from(format!(
                    "  [{}] {} (default: {})",
                    i + 1,
                    a.id,
                    a.name
                )));
            }
            lines.push(Line::from(""));
            lines.push(input_box_open("rename"));
            lines.push(Line::from(Span::styled(
                format!("  \u{2502}  > {}\u{2588}", s.meta_rename_input),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            lines.push(input_box_close());
            lines
        }
        _ => vec![Line::from("done")],
    }
}

fn render_step_products(s: &SetupState) -> Vec<Line<'static>> {
    // Step 3 is now mostly empty because token is mandatory; if meta
    // succeeded, products were auto-populated. We just confirm here.
    // The input box is hidden (read-only confirmation) and the Enter key
    // proceeds to step 4 (goals).
    let mut lines = vec![
        styled_title("Confirm products"),
        Line::from(""),
    ];
    if s.products.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no products yet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} product{} configured:",
                s.products.len(),
                if s.products.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Green),
        )));
        for (n, a) in &s.products {
            lines.push(Line::from(Span::styled(
                format!("    \u{2713} {}  \u{2192}  {}", n, a),
                Style::default(),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  press enter to continue \u{00B7} esc to go back",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_step_goals(s: &SetupState) -> Vec<Line<'static>> {
    let mut lines = vec![
        styled_title("Goals per product"),
        Line::from(""),
        Line::from("  Format: ProdName=Number, space- or comma-separated. Multi-word"),
        Line::from("  product names are fine: the parser reads up to the '='."),
        Line::from("  Example: Namma Mane=10 Valmark CityVille=15"),
        Line::from(Span::styled(
            "  Press Enter on blank line to accept all defaults.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (n, _) in &s.products {
        lines.push(Line::from(Span::styled(
            format!("    {} = 10 (default)", n),
            Style::default(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(input_box_open("goals"));
    lines.push(Line::from(Span::styled(
        format!("  \u{2502}  > {}\u{2588}", s.goals_input),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(input_box_close());
    lines
}

/// Step 4: LLM provider config. Required - /brief and /ask depend on this.
///
/// Substeps:
///   0 -> pick a built-in preset (openai / anthropic / minimax / ollama)
///        or "custom"
///   1 -> paste the API key (masked + char count)
///   2 -> pick a curated model from the provider's list
///   3 -> (custom only) enter the base URL
///
/// Saves the API key to the OS keychain via `secrets::store_llm_key` on
/// advance. The provider entry is persisted to config.json alongside the
/// rest of the workspace config in advance_save.
fn render_step_llm(s: &SetupState) -> Vec<Line<'static>> {
    match s.llm_substep {
        0 => render_llm_pick_provider(s),
        1 => render_llm_paste_key(s),
        2 => render_llm_pick_model(s),
        _ => render_llm_pick_provider(s),
    }
}

fn render_llm_pick_provider(s: &SetupState) -> Vec<Line<'static>> {
    const VISIBLE_ROWS: usize = 6;
    let presets = built_in_presets();
    let total = presets.len() + 1; // +1 for "custom"
    let mut lines = vec![
        styled_title("LLM provider"),
        Line::from(""),
        Line::from("  Pick the model provider that drives /brief and /ask."),
        Line::from("  Custom lets you point at any OpenAI/Anthropic-compatible URL."),
        Line::from(""),
        Line::from(Span::styled(
            "  \u{2191}\u{2193} move  Enter=select  Esc=back",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    let end = (s.llm_scroll + VISIBLE_ROWS).min(total);
    let start = s.llm_scroll.min(end);
    let mut rows: Vec<(String, String)> = presets
        .iter()
        .map(|(id, p)| {
            let wire = match p.wire_api {
                WireApi::Responses => "Responses",
                WireApi::ChatCompletions => "Chat",
                WireApi::Messages => "Messages",
            };
            (id.to_string(), format!("  {} - {} ({})", p.name, p.base_url, wire))
        })
        .collect();
    rows.push(("custom".to_string(), "  custom - your own URL (any wire protocol)".to_string()));

    for (i, (id, text)) in rows.iter().enumerate().skip(start).take(end - start) {
        let marker = if i == s.llm_highlight { "\u{25B8}" } else { " " };
        let style = if i == s.llm_highlight {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let line = format!("  {} {}", marker, text);
        let _ = id; // silence unused warning; id is available for future row-action
        lines.push(Line::from(Span::styled(line, style)));
    }
    if end < total {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more below", total - end),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn render_llm_paste_key(s: &SetupState) -> Vec<Line<'static>> {
    let n = s.llm_input.chars().count();
    let provider_label = if s.llm_provider_id == "custom" {
        format!("custom ({})", s.llm_url)
    } else {
        s.llm_provider_id.clone()
    };
    let mut lines = vec![
        styled_title("LLM API key"),
        Line::from(""),
        Line::from(format!("  Provider: {}", provider_label)),
        Line::from(""),
        Line::from("  Paste the API key for this provider."),
        Line::from("  It is saved to the OS keychain (account: llm:<id>)"),
        Line::from("  and never written to disk in plaintext."),
        Line::from(""),
    ];
    let masked = "\u{2022}".repeat(n);
    lines.push(input_box_open("api key"));
    lines.push(Line::from(Span::styled(
        format!("  \u{2502}  > {}{}", masked, "\u{2588}"),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(input_box_close());
    lines.push(Line::from(Span::styled(
        format!("  ({} chars)", n),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  enter to validate + continue  \u{00B7}  esc back",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_llm_pick_model(s: &SetupState) -> Vec<Line<'static>> {
    let preset = if s.llm_provider_id == "custom" {
        ModelProviderInfo {
            name: "custom".into(),
            base_url: s.llm_url.clone(),
            ..Default::default()
        }
    } else {
        s.llm_provider.clone().unwrap_or_else(ModelProviderInfo::openai)
    };
    let models = models_for(&preset);
    let total = models.len();
    let mut lines = vec![
        styled_title("Model"),
        Line::from(""),
        Line::from(format!("  Provider: {}", s.llm_provider_id)),
        Line::from(""),
        Line::from("  Pick the model. /brief and /ask will use it."),
        Line::from(Span::styled(
            "  \u{2191}\u{2193} move  Enter=select  Esc=back",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (i, m) in models.iter().enumerate() {
        let marker = if i == s.llm_highlight { "\u{25B8}" } else { " " };
        let style = if i == s.llm_highlight {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("  {} {}", marker, m),
            style,
        )));
    }
    if total == 1 && models[0] == "custom" {
        // For custom provider, ask for the model slug via free text.
        lines.push(Line::from(""));
        lines.push(input_box_open("model"));
        lines.push(Line::from(Span::styled(
            format!("  \u{2502}  > {}\u{2588}", s.llm_model),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        lines.push(input_box_close());
        let _ = total; // silence unused
    }
    lines
}

#[cfg(test)]
mod paste_tests {
    use super::*;
    use std::path::PathBuf;

    fn make_state(step: usize, substep: u8) -> SetupState {
        let mut s = SetupState::new(PathBuf::from("/tmp/mb-test"));
        s.workspace_path = "/tmp/mb-test".into();
        s.step = step;
        s.meta_substep = substep;
        s
    }

    fn app_with_setup(state: SetupState) -> App {
        let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
        let mut app = App::new_for_test(cfg);
        app.force_setup_for_test(state);
        app
    }

    #[test]
    fn paste_meta_token_into_substep0() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "EAA12345abcdefghij".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_input, "EAA12345abcdefghij");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_strips_newlines_and_control_chars() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        // Real clipboard often wraps a token with a trailing newline.
        handle_paste(&mut app, "EAA12345\n".into());
        handle_paste(&mut app, "abc\tdef\r".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_input, "EAA12345abcdef");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_workspace_step() {
        let s = make_state(0, 0);
        let mut app = app_with_setup(s);
        // Clear the default workspace path before pasting.
        match &mut app.view {
            View::Setup(state) => state.workspace_path.clear(),
            _ => unreachable!(),
        }
        handle_paste(&mut app, "/tmp/pasted-workspace".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.workspace_path, "/tmp/pasted-workspace");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_product_input() {
        let s = make_state(2, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "FincityOfficial act_1".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.product_input, "FincityOfficial act_1");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_goals_input() {
        let s = make_state(3, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "Namma Mane=12".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.goals_input, "Namma Mane=12");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_multi_select_is_ignored() {
        // substep 1 has no text input; paste should not change selection.
        let s = make_state(1, 1);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "should be dropped".into());
        match app.view {
            View::Setup(state) => {
                assert!(state.meta_input.is_empty());
                assert!(state.meta_rename_input.is_empty());
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_rename_substep() {
        let s = make_state(1, 2);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "1=BrandName".into());
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_rename_input, "1=BrandName");
            }
            _ => panic!("expected Setup view"),
        }
    }

    #[test]
    fn paste_into_brief_input() {
        let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
        let mut app = App::new_for_test(cfg);
        // resolve_optional yields a setup view when no workspace config exists,
        // so we override that here for the brief-view paste test.
        app.force_welcome_for_test();
        handle_paste(&mut app, "what is my best product?".into());
        assert_eq!(app.input, "what is my best product?");
    }

    #[test]
    fn empty_paste_is_noop() {
        let s = make_state(1, 0);
        let mut app = app_with_setup(s);
        handle_paste(&mut app, "".into());
        match app.view {
            View::Setup(state) => assert!(state.meta_input.is_empty()),
            _ => panic!("expected Setup view"),
        }
    }

    /// Esc on the multi-select step (substep 1) returns to the token paste
    /// (substep 0) WITHOUT nuking the discovered accounts or the user's
    /// per-row selections. The token input is restored as N bullets so the
    /// box doesn't look empty. Regression test for the bug where going
    /// back from substep 1 dropped all of `meta_discovered` / selections.
    #[test]
    fn esc_from_multi_select_preserves_state() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use moneyball_core::meta::AdAccount;
        let mut s = make_state(1, 1);
        s.meta_token_len = 124;
        s.meta_discovered = vec![
            AdAccount {
                id: "act_1".into(),
                name: "Acme".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_2".into(),
                name: "Beta".into(),
                account_status: Some(1),
            },
        ];
        s.meta_selections = vec![true, false];
        s.meta_selected = vec![0];
        s.meta_highlight = 1;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_substep, 0, "should be back on token paste");
                assert_eq!(state.meta_discovered.len(), 2, "discovered list preserved");
                assert_eq!(state.meta_selections, vec![true, false], "selections preserved");
                assert_eq!(state.meta_selected, vec![0], "selected indices preserved");
                // Token input restored as N bullets.
                assert_eq!(state.meta_input.chars().count(), 124);
                assert!(state.meta_input.chars().all(|c| c == '\u{2022}'));
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Esc on the rename step (substep 2) returns to the multi-select
    /// (substep 1) WITHOUT nuking selections; only the rename input is
    /// cleared so the user can re-type.
    #[test]
    fn esc_from_rename_preserves_selections() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(1, 2);
        s.meta_rename_input = "1=Acme".into();
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_substep, 1, "should be back on multi-select");
                assert!(state.meta_rename_input.is_empty(), "rename input cleared");
            }
            _ => panic!("expected Setup view"),
        }
    }
}
