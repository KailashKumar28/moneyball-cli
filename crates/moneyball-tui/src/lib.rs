//! moneyball-tui - ratatui REPL with brief view, slash-commands,
//! completion, and first-run setup wizard.

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
use moneyball_core::{AppConfig, WorkspaceConfig};

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

#[derive(Debug, Clone)]
enum View { Setup(SetupState), Brief }

#[derive(Debug, Clone)]
pub struct SetupState {
    pub step: usize,
    pub workspace_path: String,
    /// Step 2 entry buffer: "Name AdAccount" (space- or comma-separated).
    pub product_input: String,
    pub products: Vec<(String, String)>, // (name, ad_account)
    /// Step 3 entry buffer: "Prod1=10 Prod2=12".
    pub goals_input: String,
    pub target_rpq_input: String,
    pub error: Option<String>,
}

impl SetupState {
    pub fn new(default: PathBuf) -> Self {
        Self {
            step: 0,
            workspace_path: default.display().to_string(),
            product_input: String::new(),
            products: Vec::new(),
            goals_input: String::new(),
            target_rpq_input: "2500".into(),
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
}

impl App {
    pub fn new_for_test(cfg: AppConfig) -> Self {
        Self::new(cfg)
    }

    pub fn force_setup_for_test(&mut self, state: SetupState) {
        self.view = View::Setup(state);
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
    let cfg = AppConfig::resolve_optional(None, None);
    let mut app = App::new(cfg);
    if matches!(app.view, View::Brief) {
        app.load_brief();
    }
    let mut terminal = init()?;
    let res = event_loop(&mut terminal, &mut app);
    restore()?;
    res
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
        KeyCode::Esc => { app.quit = true; }
        KeyCode::Tab => {
            if app.input.starts_with('/') && app.completions.is_empty() {
                app.completions = completions(&app.input);
                app.completion_idx = if app.completions.is_empty() { None } else { Some(0) };
            } else if !app.completions.is_empty() {
                let i = (app.completion_idx.unwrap_or(0) + 1) % app.completions.len();
                app.completion_idx = Some(i);
            }
            apply_completion(app);
        }
        KeyCode::Backspace => { backspace(app); refresh_completions(app); }
        KeyCode::Char(c) => { insert(app, c); refresh_completions(app); }
        KeyCode::Enter => { submit(app); }
        KeyCode::Left => { if app.cursor > 0 { app.cursor -= 1; } }
        KeyCode::Right => { if app.cursor < app.input.len() { app.cursor += 1; } }
        _ => {}
    }
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
    let line = app.input.trim().to_string();
    app.input.clear();
    app.cursor = 0;
    app.completions.clear();
    app.completion_idx = None;
    if line.is_empty() { return; }
    let mut parts = line.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();
    match cmd {
        "/quit" | "/exit" | "/q" => app.quit = true,
        "/brief" => app.load_brief(),
        "/setup" => {
            app.view = View::Setup(SetupState::new(app.cfg.data_root.clone()));
        }
        "/funnel" => app.status = Some(format!("/funnel {} - coming next iteration", arg)),
        "/diagnose" => app.status = Some(format!("/diagnose {} - coming next iteration", arg)),
        "/ask" => app.status = Some("/ask - coming next iteration".into()),
        "/snapshot" => app.status = Some("/snapshot - coming next iteration".into()),
        "/ledger" => app.status = Some("/ledger - coming next iteration".into()),
        _ => app.status = Some(format!("unknown command: {} (tab for completions)", cmd)),
    }
}

// ---------- setup-wizard keys ----------

fn handle_setup_key(app: &mut App, mut state: SetupState, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => { app.quit = true; }
        KeyCode::Enter => { advance_setup(app, &mut state); }
        KeyCode::Backspace => { backspace_setup(&mut state); }
        KeyCode::Char(c) => { insert_setup(&mut state, c); }
        _ => {}
    }
    // Keep view in sync.
    app.view = View::Setup(state);
}

fn insert_setup(s: &mut SetupState, c: char) {
    match s.step {
        0 => { s.workspace_path.push(c); }
        1 => { s.product_input.push(c); }
        2 => { s.goals_input.push(c); }
        3 => { s.target_rpq_input.push(c); }
        _ => {}
    }
}

fn backspace_setup(s: &mut SetupState) {
    match s.step {
        0 => { s.workspace_path.pop(); }
        1 => { s.product_input.pop(); }
        2 => { s.goals_input.pop(); }
        3 => { s.target_rpq_input.pop(); }
        _ => {}
    }
}

fn advance_setup(app: &mut App, s: &mut SetupState) {
    s.error = None;
    match s.step {
        0 => {
            let p = PathBuf::from(s.workspace_path.trim());
            if !p.is_dir() {
                // Try to create it. Most likely: parent exists, dir doesn't.
                match std::fs::create_dir_all(&p) {
                    Ok(()) => {} // created
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
        1 => {
            let raw = s.product_input.trim();
            if raw.is_empty() {
                // Blank + Enter advances only if we already have at least one.
                if s.products.is_empty() {
                    s.error = Some("add at least one product (try 'demo' to load Fincity example)".into());
                    return;
                }
                s.step = 2;
                return;
            }
            if raw.eq_ignore_ascii_case("demo") {
                // Demo shortcut: load Fincity example products.
                s.products = DEMO_PRODUCTS.iter()
                    .map(|(n, a)| (n.to_string(), a.to_string()))
                    .collect();
                s.product_input.clear();
                s.error = None;
                return;
            }
            // Parse "Name AdAccount" or "Name,AdAccount".
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
                    s.error = None;
                }
                _ => {
                    s.error = Some(format!("expected 'Name AdAccount' - got '{}'", raw));
                }
            }
        }
        2 => {
            // Parse goals_input - format "Prod1=10 Prod2=12"
            // Blank + Enter advances with all products at default goal 10.
            let raw = s.goals_input.trim();
            if raw.is_empty() {
                // Defaults to 10 for every product.
                s.goals_input = s.products.iter()
                    .map(|(n, _)| format!("{}=10", n))
                    .collect::<Vec<_>>().join(" ");
                s.step = 3;
                return;
            }
            let parsed = parse_goals(&s.products, raw);
            match parsed {
                Ok(map) => {
                    s.goals_input = format_goals(&map);
                    s.step = 3;
                }
                Err(e) => s.error = Some(e),
            }
        }
        3 => {
            let rpq: f64 = s.target_rpq_input.trim().parse().unwrap_or(2500.0);
            // Save config.
            let products: Vec<_> = s.products.iter().map(|(n, a)| moneyball_core::config::Product {
                name: n.clone(),
                ad_account: a.clone(),
            }).collect();
            let goals_map = parse_goals(&s.products, &s.goals_input).unwrap_or_default();
            let cfg = WorkspaceConfig {
                products,
                goals: goals_map,
                target_rs_per_q: rpq,
                crm: Default::default(),
            };
            if let Err(e) = cfg.save(&app.cfg.data_root) {
                s.error = Some(format!("save failed: {}", e));
                return;
            }
            // Reload config and switch to brief view.
            app.cfg.workspace = Some(cfg);
            app.view = View::Brief;
            app.load_brief();
            app.status = Some("setup complete - showing brief".into());
        }
        _ => {}
    }
}

fn parse_goals(products: &[(String, String)], s: &str) -> std::result::Result<std::collections::HashMap<String, f64>, String> {
    let mut out = std::collections::HashMap::new();
    let known: std::collections::HashSet<&str> = products.iter().map(|(n, _)| n.as_str()).collect();
    for part in s.split_whitespace() {
        let (name, val) = part.split_once('=').ok_or_else(|| format!("expected ProdName=Number, got '{}'", part))?;
        if !known.contains(name) {
            return Err(format!("unknown product '{}' (known: {:?})", name, products.iter().map(|(n,_)| n.as_str()).collect::<Vec<_>>()));
        }
        let v: f64 = val.parse().map_err(|_| format!("not a number: '{}' in {}", val, part))?;
        if v <= 0.0 || v > 1000.0 {
            return Err(format!("goal {} out of range (1-1000)", v));
        }
        out.insert(name.to_string(), v);
    }
    // Fill in defaults for any missing products.
    for n in known {
        out.entry(n.to_string()).or_insert(10.0);
    }
    Ok(out)
}

fn format_goals(m: &std::collections::HashMap<String, f64>) -> String {
    m.iter().map(|(k, v)| format!("{}={}", k, *v as i64)).collect::<Vec<_>>().join(" ")
}

// ---------- render ----------

fn render(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    match &app.view {
        View::Setup(s) => render_setup(f, area, s),
        View::Brief => render_brief(f, area, app),
    }
}

fn render_setup(f: &mut ratatui::Frame, area: Rect, s: &SetupState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);
    let header = Paragraph::new(Line::from(vec![
        Span::styled("moneyball ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("- first-time setup"),
    ])).block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let body = match s.step {
        0 => Paragraph::new(vec![
            Line::from(Span::styled("Step 1 of 4: workspace path",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from("  This is where moneyball will read snapshots + write ledger/runs."),
            Line::from("  The directory will be auto-created if it does not exist."),
            Line::from(""),
            Line::from(Span::styled(format!("  > {}", s.workspace_path), Style::default().add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Press Enter to accept. Backspace to edit. Esc to quit.",
                Style::default().fg(Color::DarkGray))),
        ]),
        1 => {
            let mut lines = vec![
                Line::from(Span::styled("Step 2 of 4: add your products",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from("  Type: ProductName AdAccountId, then press Enter."),
                Line::from("  Examples:"),
                Line::from(Span::styled("    Namma Mane 2087011578504572", Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled("    My Product,123456789012345", Style::default().fg(Color::DarkGray))),
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
            lines.push(Line::from(Span::styled(
                format!("  > {}", s.product_input),
                Style::default().add_modifier(Modifier::BOLD))));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Type 'demo' to load the Fincity example (4 products).",
                Style::default().fg(Color::Yellow))));
            lines.push(Line::from(Span::styled(
                "  Press Enter on blank line when done adding products.",
                Style::default().fg(Color::DarkGray))));
            Paragraph::new(lines)
        }
        2 => {
            let mut lines = vec![
                Line::from(Span::styled("Step 3 of 4: goals (qualified leads per day)",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from("  Format: ProductName=10 (space-separated). Defaults to 10 if omitted."),
                Line::from("  Example: Namma Mane=10 Valmark CityVille=15"),
                Line::from(""),
            ];
            for (n, _) in &s.products {
                lines.push(Line::from(format!("    {} = 10 (default)", n)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(format!("  > {}", s.goals_input),
                Style::default().add_modifier(Modifier::BOLD))));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Press Enter on blank line to accept all defaults.",
                Style::default().fg(Color::DarkGray))));
            Paragraph::new(lines)
        }
        3 => Paragraph::new(vec![
            Line::from(Span::styled("Step 4 of 4: target \u{20B9} per qualified lead",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from("  Default 2500. Lower = stricter quality bar."),
            Line::from(""),
            Line::from(Span::styled(format!("  > {}", s.target_rpq_input),
                Style::default().add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Press Enter to save and load your portfolio.",
                Style::default().fg(Color::DarkGray))),
        ]),
        _ => Paragraph::new("done"),
    };
    let body = body.block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[1]);

    let mut footer_lines = vec![];
    if let Some(e) = &s.error {
        footer_lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
    }
    footer_lines.push(Line::from(format!("  step {} of 4 - Enter to continue, Esc to quit", s.step + 1)));
    let footer = Paragraph::new(footer_lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn render_brief(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(COMMANDS.len() as u16 + 2),
            Constraint::Length(3),
        ])
        .split(area);

    // Body: brief table + feasibility, or status message
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
        let msg = app.status.clone().unwrap_or_else(|| "no brief loaded yet - press /brief to try loading".into());
        let body = Paragraph::new(msg).block(Block::default().borders(Borders::ALL).title("status"));
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

fn render_input_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(30)])
        .split(area);
    let prompt = format!("> {}", app.input);
    let p = Paragraph::new(prompt)
        .block(Block::default().borders(Borders::ALL).title("input (Enter send, Tab complete, Esc quit)"));
    f.render_widget(p, chunks[0]);
    // Completion suggestions
    let suggestions = if app.completions.is_empty() {
        vec![Line::from(Span::styled("  (type / for commands)", Style::default().fg(Color::DarkGray)))]
    } else {
        app.completions.iter().enumerate().map(|(i, c)| {
            let style = if Some(i) == app.completion_idx {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(format!("  {}", c), style))
        }).collect()
    };
    let sugg = Paragraph::new(suggestions).block(Block::default().borders(Borders::ALL).title("completions"));
    f.render_widget(sugg, chunks[1]);
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