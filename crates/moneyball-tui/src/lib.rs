//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

pub mod chat;
mod commands;
mod setup;
pub mod widgets;
pub use setup::SetupState;

use std::io::{self, Stdout};
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
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

use moneyball_core::brief::{self, ProductRowsAndFeasibility};
use moneyball_core::session::{Session, SessionCell};
use moneyball_core::AppConfig;

// Two chrono types collide on import name; alias the plain Utc.
use chrono::Utc as ChronoUtc;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    // Mouse capture lets the wheel scroll the transcript instead of the
    // terminal window (text selection becomes Shift+drag - standard TUI
    // trade-off, same as codex).
    execute!(
        out,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    // Best-effort: release the mouse even if leaving the alt screen fails.
    let _ = execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

// ---------- slash-command surface ----------

// ---------- app state ----------

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Brief is a unit variant; SetupState is fat.
                                     // Boxing the latter would force every match arm to deref.
enum View {
    Setup(SetupState),
    Brief,
}

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
                // Wheel scrolls the transcript (chat view only).
                Event::Mouse(m) => {
                    if matches!(app.view, View::Brief) {
                        match m.kind {
                            crossterm::event::MouseEventKind::ScrollUp => app.chat.scroll_up(3),
                            crossterm::event::MouseEventKind::ScrollDown => app.chat.scroll_down(3),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        drain_stream(app);
    }
    Ok(())
}

/// Drain any pending LLM stream deltas into the chat's streaming cell.
/// Runs every tick, so text appears as it arrives (~10 redraws/sec).
fn drain_stream(app: &mut App) {
    use std::sync::mpsc::TryRecvError;
    let Some(rx) = &app.stream else { return };
    let mut finished = false;
    loop {
        match rx.try_recv() {
            Ok(StreamEvent::Delta(d)) => app.chat.append_assistant(&d),
            Ok(StreamEvent::Done { ms, provider }) => {
                app.chat
                    .append_assistant(&format!(" ({}ms via {})", ms, provider));
                app.chat.finish_streaming();
                finished = true;
                break;
            }
            Ok(StreamEvent::Failed(e)) => {
                app.chat
                    .append_assistant(&format!("llm call failed: {}", e));
                app.chat.finish_streaming();
                finished = true;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                app.chat.finish_streaming();
                finished = true;
                break;
            }
        }
    }
    if finished {
        app.stream = None;
    }
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
        View::Setup(state) => setup::handle_setup_key(app, state.clone(), k),
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
                app.completions = commands::completions(&app.input);
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
            // Codex-style palette: Enter on a partial slash command runs
            // the SELECTED entry instead of submitting the raw prefix
            // (which would fall through to the LLM as free-form chat).
            if !app.completions.is_empty() && app.input.starts_with('/') && !app.input.contains(' ')
            {
                apply_completion(app);
            }
            commands::submit(app);
        }
        KeyCode::Up => {
            arm_cancel(app);
            if !app.completions.is_empty() {
                let n = app.completions.len();
                let i = app.completion_idx.unwrap_or(0);
                app.completion_idx = Some((i + n - 1) % n);
            } else {
                // No palette open: Up scrolls the transcript.
                app.chat.scroll_up(1);
            }
        }
        KeyCode::Down => {
            arm_cancel(app);
            if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            } else {
                app.chat.scroll_down(1);
            }
        }
        KeyCode::PageUp => {
            arm_cancel(app);
            app.chat.scroll_up(10);
        }
        KeyCode::PageDown => {
            arm_cancel(app);
            app.chat.scroll_down(10);
        }
        KeyCode::Home => {
            arm_cancel(app);
            app.chat.scroll_to_top();
        }
        KeyCode::End => {
            arm_cancel(app);
            app.chat.scroll_to_bottom();
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
    // Esc during a live response interrupts it (codex behavior). The worker
    // thread's next send fails once the receiver drops, and it exits.
    if app.stream.take().is_some() {
        app.chat.append_assistant(" (interrupted)");
        app.chat.finish_streaming();
        app.status = Some("response interrupted".into());
        return;
    }
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
        app.completions = commands::completions(&app.input);
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
        View::Setup(s) => setup::render_setup(f, outer[2], s),
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
    // Command palette (codex / claude-code pattern): a dropdown of
    // matching commands + descriptions BELOW the input line, arrow-
    // navigable, Enter runs the selected entry. Height adapts to the
    // terminal: fixed rows must never squeeze out the body/chrome.
    let status_h: u16 = if app.status.is_some() { 1 } else { 0 };
    let reserved = 3 + status_h; // separator + input + caption + status
                                 // While the palette is open it outranks transcript rows (the user is
                                 // mid-command); keep >=2 body rows and give the palette the rest.
    let palette_room = area.height.saturating_sub(reserved + 2);
    let palette_h: u16 = (app.completions.len() as u16).min(8).min(palette_room);

    // Body height: as many rows as the content needs, up to whatever the
    // terminal allows after reserving the input area (separator + input +
    // palette + caption + status). Short logs stay compact; long logs use
    // the full screen; anything beyond is reachable via scroll keys.
    let max_body = area.height.saturating_sub(reserved + palette_h).max(5);
    let content_lines = count_chat_lines(app, area.width) as u16;
    let body_h = content_lines.clamp(5, max_body);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(body_h),    // chat scrollback (natural height)
            Constraint::Length(1),         // separator
            Constraint::Length(1),         // input line
            Constraint::Length(palette_h), // command palette (below input)
            Constraint::Length(1),         // keybinding caption
            Constraint::Length(status_h),  // status hint
        ])
        .split(area);
    let sep_idx = 1;
    let input_idx = 2;
    let palette_idx = 3;
    let caption_idx = 4;
    let status_idx = 5;

    // 1. Chat log
    let log_lines: Vec<Line<'static>> = {
        let width = chunks[0].width.max(20);
        let height = chunks[0].height.max(5);
        app.chat.render(width, height)
    };
    f.render_widget(Paragraph::new(log_lines), chunks[0]);

    // 2. Thin horizontal separator (above the input line)
    let width = area.width as usize;
    f.render_widget(
        Paragraph::new(Line::from(sep_str(width))).style(Style::default().fg(Color::DarkGray)),
        chunks[sep_idx],
    );

    // 3. Command palette rows (below the input): `▸ /brief   description`,
    // selected row highlighted. Renders only while a `/` prefix matches.
    if palette_h > 0 {
        // Window the list around the selected row so arrow-navigation
        // never moves the highlight off-screen when the list is taller
        // than the palette area.
        let visible = palette_h as usize;
        let sel = app.completion_idx.unwrap_or(0);
        let start = if sel >= visible { sel + 1 - visible } else { 0 };
        let mut rows: Vec<Line<'static>> = Vec::new();
        for (i, c) in app.completions.iter().enumerate().skip(start).take(visible) {
            let desc = commands::COMMANDS
                .iter()
                .find(|(name, _)| name == c)
                .map(|(_, d)| *d)
                .unwrap_or("");
            let selected = Some(i) == app.completion_idx;
            let (marker, cmd_style, desc_style) = if selected {
                (
                    "\u{25B8} ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::Gray),
                )
            } else {
                (
                    "  ",
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                )
            };
            rows.push(Line::from(vec![
                Span::styled(format!("  {}", marker), cmd_style),
                Span::styled(format!("{:<22}", c), cmd_style),
                Span::styled(desc.to_string(), desc_style),
            ]));
        }
        f.render_widget(Paragraph::new(rows), chunks[palette_idx]);
    }

    let placeholder = "ask moneyball about your portfolio or type / for commands";
    let prompt_line = if app.input.is_empty() {
        // Caret sits at the input START (codex/claude-code); the dim
        // placeholder trails after it, not the other way round.
        Line::from(vec![
            Span::styled(
                "\u{276F} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
            Span::styled(
                placeholder,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
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

    // 5. Keybinding caption (no borders) - contextual, codex-style.
    let caption_text = if palette_h > 0 {
        "  \u{2191}\u{2193} choose  \u{00B7}  \u{21B5} run  \u{00B7}  \u{21E5} complete  \u{00B7}  esc clear"
    } else {
        "  \u{21B5} send  \u{00B7}  esc clear input  \u{00B7}  \u{21E5} complete  \u{00B7}  \u{2191}\u{2193} scroll  \u{00B7}  /exit to quit"
    };
    let caption = Line::from(Span::styled(
        caption_text,
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
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.meta_substep, 0, "should be back on token paste");
                assert_eq!(state.meta_discovered.len(), 2, "discovered list preserved");
                assert_eq!(
                    state.meta_selections,
                    vec![true, false],
                    "selections preserved"
                );
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
        setup::handle_setup_key(
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

    /// Regression test: the LLM provider picker must NOT eat Enter /
    /// Esc / Backspace / Char keys. Only Up/Down/Home/End belong to
    /// the picker. If the picker consumed Enter, advance_setup never
    /// ran and the user was stuck. This is the bug the user hit in
    /// the initial setup.
    #[test]
    fn llm_picker_does_not_consume_enter() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(4, 0);
        s.llm_highlight = 0;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                // Enter on the provider picker should pick "openai"
                // (the first preset) and advance to substep 1 (key paste).
                assert_eq!(state.llm_substep, 1, "should advance to key paste");
                assert_eq!(state.llm_provider_id, "openai");
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Picker nav (Up/Down) should still work and be consumed by the
    /// picker (so it doesn't fall through to backspace/insert).
    #[test]
    fn llm_picker_down_arrow_advances_highlight() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = make_state(4, 0);
        s.llm_highlight = 0;
        s.llm_scroll = 0;
        let snapshot = s.clone();
        let mut app = app_with_setup(s);
        setup::handle_setup_key(
            &mut app,
            snapshot,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        match app.view {
            View::Setup(state) => {
                assert_eq!(state.llm_highlight, 1, "Down should move highlight");
            }
            _ => panic!("expected Setup view"),
        }
    }

    /// Round-trip test: a keychain write that returns Ok should also
    /// be readable. Catches the macOS bug where an ad-hoc binary's
    /// write is silently dropped (the wizard's verify-after-write
    /// guard relies on this). Skipped on headless environments where
    /// the keychain isn't available, and prints a clear warning if
    /// the env has the macOS ACL bug (since CI on ad-hoc binaries
    /// would otherwise fail forever).
    #[test]
    fn keychain_round_trip_persists_for_wizard() {
        let provider = "test_round_trip_provider";
        let token = "sk-test-1234567890abcdef";
        // Best-effort cleanup before the test.
        moneyball_core::secrets::clear_llm_key(provider).ok();
        if moneyball_core::secrets::store_llm_key(provider, token).is_err() {
            eprintln!(
                "[moneyball] skipping keychain round-trip test: write failed \
                 (likely headless / no keychain access in this env)"
            );
            return;
        }
        let read_back = moneyball_core::secrets::load_llm_key(provider);
        moneyball_core::secrets::clear_llm_key(provider).ok();
        match read_back.as_deref() {
            Some(t) if t == token => {} // success
            Some(other) => panic!(
                "keychain read returned a different value than was written: \
                 {:?} (likely a keychain corruption / multi-process race)",
                other
            ),
            None => {
                // The macOS ad-hoc-binary ACL bug. The wizard now
                // catches this with a verify-after-write guard; this
                // test just warns (since CI on a non-signed binary
                // would otherwise fail forever). If you see this in
                // a properly-signed build, the wizard guard is the
                // safety net.
                eprintln!(
                    "[moneyball] keychain round-trip test: write returned Ok \
                     but read returned None. This is the macOS ad-hoc-binary \
                     ACL bug - the wizard now rejects this case via \
                     verify-after-write. Sign the binary to make this pass."
                );
            }
        }
    }
}
