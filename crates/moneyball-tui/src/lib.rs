//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

pub mod widgets;
pub mod chat;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use moneyball_core::brief::{self, ProductRowsAndFeasibility};
use moneyball_core::session::{Session, SessionCell};
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
    ("/brief",         "7-day portfolio brief"),
    ("/funnel",        "per-entity funnel for a product"),
    ("/diagnose",      "run all 5 diagnostic commands for a product"),
    ("/ask",           "free-form question (LLM picks commands)"),
    ("/snapshot",      "list or validate snapshots"),
    ("/ledger",        "prediction ledger view"),
    ("/setup",         "re-run the setup wizard"),
    ("/quit",          "exit moneyball"),
];

fn completions(prefix: &str) -> Vec<&'static str> {
    COMMANDS.iter().map(|(c, _)| *c).filter(|c| c.starts_with(prefix)).collect()
}

// ---------- app state ----------

#[derive(Debug, Clone, PartialEq)]
enum View { Setup(SetupState), Brief }

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
    /// Step 1: whether the user pasted a valid token (true) or skipped (false).
    pub meta_connected: bool,
    /// Step 2 entry buffer: "Name AdAccount" (space- or comma-separated).
    pub product_input: String,
    pub products: Vec<(String, String)>, // (name, ad_account)
    /// Step 3 entry buffer: "Prod1=10 Prod2=12".
    pub goals_input: String,
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
            product_input: String::new(),
            products: Vec::new(),
            goals_input: String::new(),
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
    /// When the user first pressed Esc on empty input. If a second Esc
    /// arrives within the window, we quit (matches codex's double-press
    /// quit shortcut model).
    pub esc_armed_at: Option<std::time::Instant>,
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
            products: products.into_iter().map(|(n, a)| Product { name: n, ad_account: a }).collect(),
            goals: Default::default(),
            target_rs_per_q: None,
            crm: Default::default(),
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
        let view = if cfg.has_workspace() { View::Brief } else { View::Setup(SetupState::new(cfg.data_root.clone())) };
        let mut chat = chat::ChatLog::new();
        let now = ChronoUtc::now();
        let session_id = format!("mb-{}", now.format("%Y%m%dT%H%M%SZ"));
        // Push the moneyball ASCII logo as the first cell on every fresh session.
        chat.push(chat::Cell::System(chat::cells::System(moneyball_core::LOGO.into())));
        if cfg.has_workspace() {
            chat.push(chat::Cell::System(chat::cells::System(
                "workspace configured. try /brief, /funnel <product>, /ask or anything you want.".into(),
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
            esc_armed_at: None,
        }
    }

    pub fn load_brief(&mut self) {
        // Try to load the brief; failure shouldn't kill the REPL.
        match self.cfg.snap_for(self.cfg.date.as_deref()) {
            Ok(p) => {
                match crate::snapshot_load(&p) {
                    Ok(snap) => {
                        let history = brief::load_history(&self.cfg.history_dir().join("scoreboard.csv"));
                        let r = brief::compute(&snap, &self.cfg, &history);
                        self.snap_date = Some(r::date_of(&snap));
                        self.brief = Some(r);
                        self.status = None;
                    }
                    Err(e) => self.status = Some(format!("snapshot load failed: {}", e)),
                }
            }
            Err(e) => self.status = Some(format!("no snapshot: {}", e)),
        }
    }
}

mod r {
    use moneyball_core::Snapshot;
    pub fn date_of(s: &Snapshot) -> String { s.date.clone() }
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
    let cfg = AppConfig::resolve_optional(None, None);
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
    app.chat = chat::ChatLog::new();
    app.chat.push(chat::Cell::System(chat::cells::System(format!(
        "resumed session {} - started {}",
        s.meta.id, s.meta.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    ))));
    for sc in s.cells {
        let c = session_cell_to_chat(&sc);
        app.chat.push(c);
    }
}

fn session_cell_to_chat(sc: &SessionCell) -> chat::Cell {
    use chat::cells as c;
    match sc {
        SessionCell::System { text } => chat::Cell::System(c::System(text.clone())),
        SessionCell::UserPrompt { text, at } => chat::Cell::UserPrompt(c::UserPrompt {
            text: text.clone(),
            // Session is UTC; Local::from(DateTime<Utc>) gives local time.
            at: (*at).with_timezone(&chrono::Local),
        }),
        SessionCell::AssistantText { text, streaming } => chat::Cell::AssistantText(c::AssistantText {
            text: text.clone(),
            streaming: *streaming,
        }),
        SessionCell::ToolCall { name, args, status } => chat::Cell::ToolCall(c::ToolCall {
            name: name.clone(),
            args: args.clone(),
            status: parse_status(status),
        }),
        SessionCell::ToolResult { name, output, success, duration_ms } => {
            chat::Cell::ToolResult(c::ToolResult {
                name: name.clone(),
                output: output.clone(),
                success: *success,
                duration_ms: *duration_ms,
            })
        }
    }
}

fn chat_cell_to_session(c: &chat::Cell) -> SessionCell {
    use chat::cells as cc;
    match c {
        chat::Cell::System(cc::System(text)) => SessionCell::System { text: text.clone() },
        chat::Cell::UserPrompt(cc::UserPrompt { text, at }) => SessionCell::UserPrompt {
            text: text.clone(),
            at: at.with_timezone(&ChronoUtc),
        },
        chat::Cell::AssistantText(cc::AssistantText { text, streaming }) => SessionCell::AssistantText {
            text: text.clone(),
            streaming: *streaming,
        },
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
        chat::Cell::ToolResult(cc::ToolResult { name, output, success, duration_ms }) => SessionCell::ToolResult {
            name: name.clone(),
            output: output.clone(),
            success: *success,
            duration_ms: *duration_ms,
        },
    }
}

fn parse_status(s: &str) -> chat::cells::ToolStatus {
    use chat::cells::ToolStatus;
    match s {
        "pending" => ToolStatus::Pending,
        "running" => ToolStatus::Running,
        "done" => ToolStatus::Done,
        _ => ToolStatus::Failed,
    }
}

/// Snapshot the current chat log + workspace into a Session and persist it.
/// Overwrites if a session with the same id already exists (so the same
/// session id can be updated across multiple invocations).
fn save_current_session(app: &App) -> Result<()> {
    let meta = moneyball_core::session::SessionMeta {
        id: app.session_id.clone().unwrap_or_else(|| {
            // Generate one if we don't have one (shouldn't happen at this point).
            moneyball_core::session::list().ok()
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
        if app.quit { break; }
        if event::poll(tick)? {
            if let Event::Key(k) = event::read()? {
                handle_key(app, k);
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, k: KeyEvent) {
    if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
        app.quit = true; return;
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
                app.completion_idx = if app.completions.is_empty() { None } else { Some(0) };
            } else if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            }
            apply_completion(app);
        }
        KeyCode::Backspace => { backspace(app); refresh_completions(app); arm_cancel(app); }
        KeyCode::Char(c) => { insert(app, c); refresh_completions(app); arm_cancel(app); }
        KeyCode::Enter => { arm_cancel(app); submit(app); }
        KeyCode::Left => { arm_cancel(app); if app.cursor > 0 { app.cursor -= 1; } }
        KeyCode::Right => { arm_cancel(app); if app.cursor < app.input.len() { app.cursor += 1; } }
        _ => { arm_cancel(app); }
    }
}

/// Esc on the chat view is the universal "let me rethink" gesture:
///   - Input non-empty: clear the input, drop completions. Status hint.
///   - Input empty + first Esc: arm a quit shortcut. Status hint.
///   - Input empty + second Esc within 1.5s: quit (session saved by run_with).
///   - Any other key after arming: cancel the arming.
fn handle_esc(app: &mut App) {
    if !app.input.is_empty() {
        app.input.clear();
        app.cursor = 0;
        app.completions.clear();
        app.completion_idx = None;
        app.esc_armed_at = None;
        app.status = Some("input cleared - press esc again on empty input to quit".into());
        return;
    }
    let now = std::time::Instant::now();
    let within_window = app.esc_armed_at
        .map(|t| now.duration_since(t) < std::time::Duration::from_millis(1500))
        .unwrap_or(false);
    if within_window {
        app.status = Some("exiting moneyball.".into());
        app.quit = true;
        return;
    }
    app.esc_armed_at = Some(now);
    app.status = Some("press esc again to quit (1.5s window)".into());
}

fn arm_cancel(app: &mut App) {
    app.esc_armed_at = None;
}

fn insert(app: &mut App, c: char) {
    app.input.insert(app.cursor, c);
    app.cursor += c.len_utf8();
}

fn backspace(app: &mut App) {
    if app.cursor == 0 { return; }
    let prev = app.input[..app.cursor].chars().next_back().unwrap();
    app.cursor -= prev.len_utf8();
    app.input.remove(app.cursor);
}

fn refresh_completions(app: &mut App) {
    if app.input.starts_with('/') {
        app.completions = completions(&app.input);
        app.completion_idx = if app.completions.is_empty() { None } else { Some(0) };
    } else {
        app.completions.clear();
        app.completion_idx = None;
    }
}

fn apply_completion(app: &mut App) {
    if let Some(i) = app.completion_idx {
        if let Some(&c) = app.completions.get(i) {
            // Replace the current token (up to cursor) with completion.
            let before = app.input[..app.cursor].rfind(' ').map(|n| n + 1).unwrap_or(0);
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
    if line.is_empty() { return; }

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
            app.chat.push(Cell::System(cells::System("exiting moneyball.".into())));
            app.quit = true;
        }
        "/brief" => {
            let started = std::time::Instant::now();
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "loading portfolio snapshot...".into(),
                streaming: false,
            }));
            app.load_brief();
            // If brief loaded, push tool call + result.
            if let Some(b) = &app.brief {
                let out = format_brief_as_lines(b);
                app.chat.push_tool("brief", "", out, true, started.elapsed().as_millis() as u64);
                app.chat.push(Cell::AssistantText(cells::AssistantText {
                    text: format_feasibility_summary(b),
                    streaming: false,
                }));
            } else {
                let err = app.status.clone().unwrap_or_else(|| "snapshot failed".into());
                app.chat.push_tool("brief", "", vec![err], false, started.elapsed().as_millis() as u64);
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
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "/ask - LLM streaming wires up in the next iteration. for now use slash commands.".into(),
                streaming: false,
            }));
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
            app.chat.push(Cell::System(cells::System(format!("unknown: {} (try /help)", cmd))));
        }
    }
}

fn format_brief_as_lines(b: &brief::ProductRowsAndFeasibility) -> Vec<String> {
    let mut out = Vec::new();
    for r in &b.rows {
        out.push(format!(
            "{:<24} {:>7}/d  m{:>4}  l{:>4}  q{:>3}  {:>5}/d  \u{20B9}{:>5}/q  L\u{2192}Q {:>4}%   gap {:>5}",
            truncate_str(&r.product, 24),
            r.spend_per_day,
            r.m7d, r.l7d, r.q7d,
            format!("{:.2}", r.q_per_day),
            r.rs_per_q.map(|x| x.to_string()).unwrap_or_else(|| "-".into()),
            r.l_to_q.map(|x| format!("{:.1}", x)).unwrap_or_else(|| "-".into()),
            format!("{:.1}", r.gap),
        ));
    }
    let f = &b.feasibility;
    out.push(format!("FEASIBILITY  {:.1} q/day @ \u{20B9}{}/day = \u{20B9}{}/q \u{00B7} goal {:.0}/day",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day));
    if let Some(req) = f.required_at_cur {
        out.push(format!("  required @ current:  \u{20B9}{}/day ({:.1}x)", req, req as f64 / f.tot_spend_per_day.max(1) as f64));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        out.push(format!("  required @ best \u{20B9}{}/q: \u{20B9}{}/day ({:.1}x)", b, req, req as f64 / f.tot_spend_per_day.max(1) as f64));
    }
    let suffix = if f.open_debt.is_empty() { String::new() } else { format!(" ({})", f.open_debt.join(", ")) };
    out.push(format!("  setup debt: {}{}", f.open_debt.len(), suffix));
    out
}

fn format_feasibility_summary(b: &brief::ProductRowsAndFeasibility) -> String {
    let f = &b.feasibility;
    let best = f.best_rpq.map(|x| x.to_string()).unwrap_or_else(|| "-".into());
    format!(
        "portfolio is at {:.1} q/day against a {}/day goal. at current \u{20B9}{}/q you'd need \u{20B9}{}/day; at the best-observed \u{20B9}{}/q you still need \u{20B9}{}/day.",
        f.tot_q_per_day, f.tot_goal_per_day as u64, f.cur_rpq, f.required_at_cur.unwrap_or(0), best, f.required_at_best.unwrap_or(0),
    )
}

fn truncate_str(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

// ---------- setup-wizard keys ----------

fn handle_setup_key(app: &mut App, mut state: SetupState, k: KeyEvent) {
    // Substep 1 of step 1 is a list-selection mode with its own keymap.
    if state.step == 1 && state.meta_substep == 1 {
        handle_select_keys(&mut state, k);
        app.view = View::Setup(state);
        return;
    }
    match k.code {
        KeyCode::Esc => { app.quit = true; }
        KeyCode::Enter => { advance_setup(app, &mut state); }
        KeyCode::Backspace => { backspace_setup(&mut state); }
        KeyCode::Char(c) => { insert_setup(&mut state, c); }
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
    if n == 0 { return; }
    // Visible rows must match the renderer's visible_rows constant below.
    const VISIBLE_ROWS: usize = 12;
    match k.code {
        KeyCode::Up => {
            if s.meta_highlight > 0 {
                s.meta_highlight -= 1;
                if s.meta_highlight < s.meta_scroll { s.meta_scroll = s.meta_highlight; }
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
        KeyCode::Home => { s.meta_highlight = 0; s.meta_scroll = 0; }
        KeyCode::End => { s.meta_highlight = n - 1; s.meta_scroll = n.saturating_sub(VISIBLE_ROWS); }
        KeyCode::Char(' ') => {
            s.meta_selections[s.meta_highlight] = !s.meta_selections[s.meta_highlight];
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let any = s.meta_selections.iter().any(|&b| b);
            for sel in s.meta_selections.iter_mut() { *sel = !any; }
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
        _ => {}
    }
}

fn insert_setup(s: &mut SetupState, c: char) {
    match s.step {
        0 => { s.workspace_path.push(c); }
        1 => { meta_insert(s, c); }
        2 => { s.product_input.push(c); }
        3 => { s.goals_input.push(c); }
        _ => {}
    }
}

fn backspace_setup(s: &mut SetupState) {
    match s.step {
        0 => { s.workspace_path.pop(); }
        1 => { meta_backspace(s); }
        2 => { s.product_input.pop(); }
        3 => { s.goals_input.pop(); }
        _ => {}
    }
}

fn meta_insert(s: &mut SetupState, c: char) {
    match s.meta_substep {
        0 => { s.meta_input.push(c); }
        // substep 1 is keyboard-driven (Up/Down/Space/'a'/Enter); ignore chars.
        1 => {}
        2 => { s.meta_rename_input.push(c); }
        _ => {}
    }
}

fn meta_backspace(s: &mut SetupState) {
    match s.meta_substep {
        0 => { s.meta_input.pop(); }
        // substep 1 ignored.
        1 => {}
        2 => { s.meta_rename_input.pop(); }
        _ => {}
    }
}

fn advance_setup(app: &mut App, s: &mut SetupState) {
    s.error = None;
    match s.step {
        0 => advance_workspace(app, s),
        1 => advance_meta(app, s),
        2 => advance_products(s),
        3 => advance_save(app, s),  // step 3 (goals) is the final step now
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

fn advance_meta(app: &mut App, s: &mut SetupState) {
    match s.meta_substep {
        // Substep 0: paste token or 'skip'.
        0 => {
            let raw = s.meta_input.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("skip") {
                // Skip Meta entirely. Continue to manual product entry.
                s.meta_connected = false;
                s.step = 2;
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
                let default_name = if overrides.is_empty() {
                    acct.name.clone()
                } else {
                    acct.name.clone() // base for the index key below
                };
                let name = overrides.get(&(idx + 1)).cloned().unwrap_or(default_name);
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

fn parse_renames(raw: &str) -> std::result::Result<std::collections::HashMap<usize, String>, String> {
    let mut out = std::collections::HashMap::new();
    for part in raw.split_whitespace() {
        let (idx_s, name) = part.split_once('=').ok_or_else(|| format!("bad rename '{}': expected N=Name", part))?;
        let idx: usize = idx_s.parse().map_err(|_| format!("bad index '{}'", idx_s))?;
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
        s.products = DEMO_PRODUCTS.iter()
            .map(|(n, a)| (n.to_string(), a.to_string()))
            .collect();
        s.product_input.clear();
        return;
    }
    let parts: Vec<&str> = raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        [name, acct] => {
            if !acct.chars().all(|c| c.is_ascii_digit()) || acct.len() < 6 {
                s.error = Some(format!("ad account '{}' should be digits only (15-20 chars)", acct));
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
        s.goals_input = s.products.iter()
            .map(|(n, _)| format!("{}=10", n))
            .collect::<Vec<_>>().join(" ");
    }
    match parse_goals(&s.products, &s.goals_input) {
        Ok(_) => true,
        Err(e) => { s.error = Some(e); false }
    }
}

fn advance_save(app: &mut App, s: &mut SetupState) {
    if !advance_goals(s) { return; }  // validation failed - keep user on goals step
    let products: Vec<_> = s.products.iter().map(|(n, a)| moneyball_core::config::Product {
        name: n.clone(),
        ad_account: a.clone(),
    }).collect();
    let goals_map = parse_goals(&s.products, &s.goals_input).unwrap_or_default();
    // target_rs_per_q is intentionally NOT asked during setup - it's a
    // derived/observed metric per product, not a hardcoded universal value.
    // Stored as None; the advisor derives it from observed performance.
    let cfg = WorkspaceConfig {
        products,
        goals: goals_map,
        target_rs_per_q: None,
        crm: Default::default(),
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
    app.status = Some(if s.meta_connected {
        "setup complete - showing brief (Meta token in keychain)".into()
    } else {
        "setup complete - showing brief (Meta skipped)".into()
    });
}

fn parse_goals(products: &[(String, String)], s: &str) -> std::result::Result<std::collections::HashMap<String, f64>, String> {
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
        if rest.is_empty() { break; }

        // Find the '=' that ends this product's name.
        let eq = rest.find('=').ok_or_else(|| {
            let snippet: String = rest.chars().take(40).collect();
            format!("expected 'ProdName=Number', no '=' found in: '{}...'", snippet)
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
            format!("not a number: '{}' in '{}={}{}'",
                val, name, val, after_eq[val_end..].chars().take(20).collect::<String>())
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
            Constraint::Length(2),    // logo
            Constraint::Length(1),    // context bar
            Constraint::Min(6),       // body
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),                       // chat scrollback
            Constraint::Length(completion_h),          // slash-command completions
            Constraint::Length(1),                      // separator
            Constraint::Length(1),                      // input line
            Constraint::Length(1),                      // keybinding caption
            Constraint::Length(status_h),               // status hint
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
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("{} ", c), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), chunks[1]);
    }

    // 3. Thin horizontal separator (above the input line)
    let width = area.width as usize;
    let sep: String = std::iter::repeat('\u{2500}').take(width.max(10)).collect();
    f.render_widget(
        Paragraph::new(Line::from(sep)).style(Style::default().fg(Color::DarkGray)),
        chunks[2 - 0 + (if has_completion { 1 } else { 0 })],
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
        Paragraph::new(Line::from(sep_str(width)))
            .style(Style::default().fg(Color::DarkGray)),
        chunks[sep_idx],
    );

    let placeholder = "ask moneyball about your portfolio or type / for commands";
    let prompt_line = if app.input.is_empty() {
        Line::from(vec![
            Span::styled("\u{276F} ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(placeholder, Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
            Span::styled("\u{2588}", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let before = app.input[..app.cursor].to_string();
        let after = app.input[app.cursor..].to_string();
        Line::from(vec![
            Span::styled("\u{276F} ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled("\u{2588}", Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK)),
            Span::styled(after, Style::default().fg(Color::White)),
        ])
    };
    f.render_widget(Paragraph::new(prompt_line), chunks[input_idx]);

    // 5. Keybinding caption (no borders)
    let caption = Line::from(Span::styled(
        "  \u{21B5} send  \u{00B7}  esc clear input  \u{00B7}  \u{21E5} complete  \u{00B7}  ?? quit",
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
    std::iter::repeat('\u{2500}').take(width.max(10)).collect()
}

fn render_setup(f: &mut ratatui::Frame, area: Rect, s: &SetupState) {
    // Body only - logo + context bar are rendered by the shared `render`.
    // Setup has no commands panel + input bar (wizard blocks commands).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(3),     // footer: error + step counter
        ])
        .split(area);

    let body = match s.step {
        0 => render_step_workspace(s),
        1 => render_step_meta(s),
        2 => render_step_products(s),
        3 => render_step_goals(s),
        _ => Paragraph::new("done"),
    };
    let body = body.block(Block::default().borders(Borders::ALL).title(step_title(s)))
        .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[0]);

    let mut footer_lines = vec![];
    if let Some(e) = &s.error {
        footer_lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
    }
    let total = 4;
    footer_lines.push(Line::from(Span::styled(
        format!("  step {} of {} - Enter to continue, Esc to quit", s.step + 1, total),
        Style::default().fg(Color::DarkGray),
    )));
    let footer = Paragraph::new(footer_lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[1]);
}

fn step_title(s: &SetupState) -> String {
    match s.step {
        0 => "moneyball - first-time setup ".into(),
        1 => "moneyball - connect Meta (recommended) ".into(),
        2 => "moneyball - confirm products ".into(),
        3 => "moneyball - goals per product ".into(),
        _ => "moneyball ".into(),
    }
}

fn styled_title(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
}

/// Prompt line with a full-block cursor at the end of `value`.
fn prompt_line(value: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  > {}\u{2588}", value),
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow),
    ))
}

fn render_step_workspace(s: &SetupState) -> Paragraph<'static> {
    let mut lines = vec![
        styled_title("Step 1 of 4: workspace path"),
        Line::from(""),
        Line::from("  This is where moneyball will read snapshots + write ledger/runs."),
        Line::from("  The directory will be auto-created if it does not exist."),
        Line::from(""),
    ];
    lines.push(Line::from(Span::styled(
        format!("  > {}\u{2588}", s.workspace_path),  // full-block cursor
        Style::default().add_modifier(Modifier::BOLD))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Press Enter to accept. Backspace to edit. Esc to quit.",
        Style::default().fg(Color::DarkGray))));
    Paragraph::new(lines)
}

fn render_step_meta(s: &SetupState) -> Paragraph<'static> {
    match s.meta_substep {
        0 => {
            let mut lines = vec![
                styled_title("Step 2 of 4: connect to Meta (optional, recommended)"),
                Line::from(""),
                Line::from("  Paste a long-lived Meta Marketing API access token"),
                Line::from("  (the one with ads_read permission; get one at"),
                Line::from("  developers.facebook.com -> Tools -> Marketing API)."),
                Line::from(""),
                Line::from(Span::styled("  Or type 'skip' to enter ad accounts manually.",
                    Style::default().fg(Color::Yellow))),
                Line::from(""),
                Line::from("  Token is saved to macOS Keychain / Linux Secret Service"),
                Line::from(Span::styled("  via the OS keyring - never written to disk in plaintext.",
                    Style::default().fg(Color::DarkGray))),
                Line::from(""),
            ];
            lines.push(prompt_line(&s.meta_input));
            Paragraph::new(lines)
        }
        1 => {
            // Multi-select list with scroll. Visible rows must match VISIBLE_ROWS
            // in handle_select_keys.
            const VISIBLE_ROWS: usize = 12;
            let n = s.meta_discovered.len();
            let selected = s.meta_selections.iter().filter(|&&b| b).count();
            let mut lines = vec![
                styled_title(&format!("Step 2 of 4: select ad accounts ({} of {} chosen)",
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
                let status = match a.account_status {
                    Some(1) => "ACTIVE",
                    Some(2) => "DISABLED",
                    Some(3) => "UNSETTLED",
                    Some(9) => "PENDING_RISK_REVIEW",
                    Some(other) => Box::leak(format!("status{}", other).into_boxed_str()),
                    None => "?",
                };
                let checkbox = if s.meta_selections[i] { "[x]" } else { "[ ]" };
                let marker = if i == s.meta_highlight { ">" } else { " " };
                let text = format!("  {} {} [{:>2}] {} - {} ({})",
                    marker, checkbox, i + 1, a.id, a.name, status);
                let style = if i == s.meta_highlight {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else if s.meta_selections[i] {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
            if end < n {
                lines.push(Line::from(Span::styled(
                    format!("  ... {} more below (PgDn to scroll)",
                        n - end),
                    Style::default().fg(Color::DarkGray))));
            } else if start > 0 {
                lines.push(Line::from(Span::styled(
                    "  ... PgUp to scroll up",
                    Style::default().fg(Color::DarkGray))));
            }
            Paragraph::new(lines)
        }
        2 => {
            let mut lines = vec![
                styled_title("Name your products (optional)"),
                Line::from(""),
                Line::from("  Defaults: each product uses the account's display name."),
                Line::from("  To rename, type e.g.  1=BrandName 3=OtherName"),
                Line::from(Span::styled("  Press Enter on blank line to keep defaults.",
                    Style::default().fg(Color::Yellow))),
                Line::from(""),
            ];
            for (i, &idx) in s.meta_selected.iter().enumerate() {
                let a = &s.meta_discovered[idx];
                lines.push(Line::from(format!("  [{}] {} (default: {})",
                    i + 1, a.id, a.name)));
            }
            lines.push(Line::from(""));
            lines.push(prompt_line(&s.meta_rename_input));
            Paragraph::new(lines)
        }
        _ => Paragraph::new("done"),
    }
}

fn render_step_products(s: &SetupState) -> Paragraph<'static> {
    let mut lines = vec![
        styled_title("Step 3 of 4: confirm your products"),
        Line::from(""),
        Line::from("  Add: 'ProductName AdAccountId' then Enter."),
        Line::from(Span::styled("  Type 'demo' to load the Fincity example (4 products).",
            Style::default().fg(Color::Yellow))),
        Line::from(Span::styled("  Press Enter on blank line when done adding products.",
            Style::default().fg(Color::DarkGray))),
        Line::from(""),
    ];
    if s.products.is_empty() {
        lines.push(Line::from(Span::styled("  (no products yet)", Style::default().fg(Color::DarkGray))));
    } else {
        lines.push(Line::from(Span::styled(format!("  {} product{} added:",
            s.products.len(), if s.products.len() == 1 { "" } else { "s" }),
            Style::default().fg(Color::Green))));
        for (i, (n, a)) in s.products.iter().enumerate() {
            lines.push(Line::from(format!("    [{}] {} -> {}", i + 1, n, a)));
        }
    }
    lines.push(Line::from(""));
    lines.push(prompt_line(&s.product_input));
    Paragraph::new(lines)
}

fn render_step_goals(s: &SetupState) -> Paragraph<'static> {
    let mut lines = vec![
        styled_title("Step 4 of 4: goals (qualified leads per day)"),
        Line::from(""),
        Line::from("  Format: ProdName=Number, space- or comma-separated. Multi-word"),
        Line::from("  product names are fine: the parser reads up to the '='."),
        Line::from("  Example: Namma Mane=10 Valmark CityVille=15 Primus by Fincity=8"),
        Line::from(""),
    ];
    for (n, _) in &s.products {
        lines.push(Line::from(format!("    {} = 10 (default)", n)));
    }
    lines.push(Line::from(""));
    lines.push(prompt_line(&s.goals_input));
    lines.push(Line::from(Span::styled("  Press Enter on blank line to accept all defaults, then save.",
        Style::default().fg(Color::DarkGray))));
    Paragraph::new(lines)
}

fn render_brief(f: &mut ratatui::Frame, area: Rect, app: &App) {
    // Brief fills: body (table OR welcome) + commands + input. Logo + context live in `render`.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(COMMANDS.len() as u16 + 2),
            Constraint::Length(3),
        ])
        .split(area);

    // Body: brief table + feasibility, OR the welcome screen.
    if let Some(b) = &app.brief {
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                format!("BRIEF  snapshot {}", app.snap_date.as_deref().unwrap_or("?")),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  (7d window; config.json goals)",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
        ];
        for r in &b.rows {
            lines.push(Line::from(format!(
                "  {:<22} {:>7}/d  m{:>4}  l{:>4}  q{:>3}  {:>5}/d  \u{20B9}{:>5}/q  L\u{2192}Q {:>4}%  gap {:>5}",
                truncate(&r.product, 22),
                comma(r.spend_per_day),
                r.m7d, r.l7d, r.q7d,
                format!("{:.2}", r.q_per_day),
                r.rs_per_q.map(|x| comma(x)).unwrap_or_else(|| "-".into()),
                r.l_to_q.map(|x| format!("{:.1}", x)).unwrap_or_else(|| "-".into()),
                format!("{:.1}", r.gap),
            )));
        }
        lines.push(Line::from(""));
        let f1 = &b.feasibility;
        lines.push(Line::from(format!(
            "FEASIBILITY  {:.1} q/day @ \u{20B9}{}/day = \u{20B9}{}/q  \u{00B7}  goal {:.0}/day",
            f1.tot_q_per_day, comma(f1.tot_spend_per_day), comma(f1.cur_rpq), f1.tot_goal_per_day,
        )));
        if let Some(req) = f1.required_at_cur {
            lines.push(Line::from(format!(
                "  required @ current:  \u{20B9}{}/day ({:.1}x)",
                comma(req), req as f64 / f1.tot_spend_per_day.max(1) as f64,
            )));
        }
        if let (Some(b), Some(req)) = (f1.best_rpq, f1.required_at_best) {
            lines.push(Line::from(format!(
                "  required @ best \u{20B9}{}/q: \u{20B9}{}/day ({:.1}x)",
                comma(b), comma(req), req as f64 / f1.tot_spend_per_day.max(1) as f64,
            )));
        }
        let suffix = if f1.open_debt.is_empty() { String::new() } else { format!(" ({})", f1.open_debt.join(", ")) };
        lines.push(Line::from(format!("  setup debt: {}{}", f1.open_debt.len(), suffix)));
        let body = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
        f.render_widget(body, chunks[0]);
    } else {
        // Welcome screen for "configured but no snapshot data yet" state.
        let products: Vec<String> = app.cfg.workspace.as_ref()
            .map(|w| w.products.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        let lines = crate::widgets::welcome_text(&products);
        let body = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("welcome"))
            .wrap(Wrap { trim: false });
        f.render_widget(body, chunks[0]);
    }

    // Slash-command help bar
    let help_items: Vec<ListItem> = COMMANDS.iter()
        .map(|(c, h)| ListItem::new(Line::from(format!("  {:<14} {}", c, h))))
        .collect();
    let help = List::new(help_items)
        .block(Block::default().borders(Borders::ALL).title("commands"));
    f.render_widget(help, chunks[1]);

    // Input bar
    render_input_bar(f, chunks[2], app);
}

fn render_input_bar(_f: &mut ratatui::Frame, _area: Rect, _app: &App) {
    // No-op: chat view renders the input line inline (see render_chat_view).
}

// ---------- helpers ----------

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('\u{2026}');
        out
    }
}

fn comma(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(b','); }
        out.push(b);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

// Suppress unused warning for the small helper module.
#[allow(dead_code)]
const _: fn() = || { let _ = Clear; };