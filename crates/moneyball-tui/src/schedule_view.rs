//! /schedule (frame R4): one confirm before anything is written - what
//! the schedule writes, what it runs, where it logs, how to remove it -
//! then y installs, t changes the time, p prints the plist instead, esc
//! cancels. `/schedule off` removes it. The launchd work is
//! moneyball_core::schedule, shared with `moneyball schedule`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Color;

use moneyball_core::schedule::{self, DEFAULT_AT};

use crate::chat::{cells, Cell};
use crate::widgets::{bold, dim, fg, gray, Para};
use crate::App;

/// A confirm on screen, waiting for its key; `editing` while a new time
/// is being typed into the composer.
pub struct Ask {
    at: (u32, u32),
    editing: bool,
}

pub(crate) fn run_schedule(app: &mut App, arg: &str) {
    if arg.trim() == "off" {
        let said = match schedule::remove(&app.cfg) {
            Ok(true) => "Schedule removed: refreshes run when you ask for one.".to_string(),
            Ok(false) => "Nothing was scheduled.".to_string(),
            Err(e) => format!("Could not remove the schedule: {}", e),
        };
        app.schedule = schedule::installed(&app.cfg);
        return push(app, vec![Para::new(6, 6).run(said, gray())]);
    }
    ask(app, app.schedule.unwrap_or(DEFAULT_AT));
}

fn ask(app: &mut App, at: (u32, u32)) {
    let plan = match plan(app, at) {
        Ok(p) => p,
        Err(e) => return push(app, vec![Para::new(6, 8).run(e, fg(Color::Red))]),
    };
    let time = format!("{:02}:{:02}", at.0, at.1);
    let question = match app.schedule {
        Some(now) if now == at => format!("Refreshes every morning at {} (installed).", time),
        Some(now) => format!(
            "Move the morning refresh from {:02}:{:02} to {}?",
            now.0, now.1, time
        ),
        None => format!("Refresh every morning at {}?", time),
    };
    let row = |key: &str, value: String| {
        Para::new(6, 15)
            .run(format!("{:<9}", key), gray())
            .run(value, fg(Color::White))
    };
    let logs = plan
        .log
        .parent()
        .map_or(String::new(), |d| format!("{}/", d.display()));
    let keys = Para::new(6, 6)
        .run("y", bold(Color::Cyan))
        .run(format!(" install at {}   ", time), gray())
        .run("t", bold(Color::Cyan))
        .run(" change the time   ", gray())
        .run("p", bold(Color::Cyan))
        .run(" print the plist instead (install it yourself)   ", gray())
        .run("esc", bold(Color::Cyan))
        .run(" cancel", gray());
    push(
        app,
        vec![
            Para::new(6, 6).run(question, crate::widgets::bold(Color::Reset)),
            Para::new(6, 6).run(
                format!(
                    "Pulls Meta, then CRM, then writes yesterday's report, the same as /refresh. \
                     It runs on this Mac: if the Mac is asleep at {t} it runs when it wakes; if it \
                     is off, the next morning catches up.",
                    t = time
                ),
                gray(),
            ),
            Para::default(),
            row("writes", plan.plist.display().to_string()),
            row("runs", plan.command()),
            row("logs", logs),
            row("remove", "/schedule off".into()).run("  (or moneyball schedule --off)", gray()),
            Para::default(),
            keys,
        ],
    );
    app.schedule_ask = Some(Ask { at, editing: false });
}

/// The confirm's keys. True when the key was the confirm's to take.
pub(crate) fn on_key(app: &mut App, k: KeyEvent) -> bool {
    let Some(pending) = &mut app.schedule_ask else {
        return false;
    };
    if pending.editing {
        match k.code {
            KeyCode::Enter => match schedule::parse_at(&app.input) {
                Ok(at) => {
                    clear_input(app);
                    app.status = None;
                    ask(app, at);
                }
                Err(e) => app.status = Some(e.to_string()),
            },
            KeyCode::Esc => {
                pending.editing = false;
                clear_input(app);
                app.status = Some("time unchanged - y installs, esc cancels".into());
            }
            _ => return false,
        }
        return true;
    }
    if !app.input.is_empty() {
        return false;
    }
    let at = pending.at;
    match k.code {
        KeyCode::Char('y') => install(app, at),
        KeyCode::Char('t') => {
            pending.editing = true;
            app.input = format!("{:02}:{:02}", at.0, at.1);
            app.cursor = app.input.len();
            app.status = Some("type the time (HH:MM, 24-hour), then enter".into());
        }
        KeyCode::Char('p') => {
            app.schedule_ask = None;
            print(app, at);
        }
        KeyCode::Esc => {
            app.schedule_ask = None;
            app.status = Some("schedule unchanged".into());
        }
        // Anything else: the user moved on; the confirm lapses unanswered.
        _ => {
            app.schedule_ask = None;
            return false;
        }
    }
    true
}

fn install(app: &mut App, at: (u32, u32)) {
    app.schedule_ask = None;
    let done = plan(app, at).and_then(|p| schedule::install(&p).map_err(|e| e.to_string()));
    let para = match done {
        Ok(()) => {
            app.schedule = Some(at);
            let next = schedule::next_from_now(at);
            Para::new(6, 6)
                .run(
                    format!(
                        "Installed: refresh every day at {:02}:{:02}, next {}. ",
                        at.0,
                        at.1,
                        next.format("%a %d %b %H:%M")
                    ),
                    fg(Color::Green),
                )
                .run("/schedule off removes it.", gray())
        }
        Err(e) => Para::new(6, 8).run(format!("Not installed: {}", e), fg(Color::Red)),
    };
    push(app, vec![para]);
}

fn print(app: &mut App, at: (u32, u32)) {
    let plan = match plan(app, at) {
        Ok(p) => p,
        Err(e) => return push(app, vec![Para::new(6, 8).run(e, fg(Color::Red))]),
    };
    let mut output = vec![format!("save as {}", plan.plist.display())];
    output.extend(plan.plist_xml().lines().map(String::from));
    output.push(format!(
        "then: launchctl bootstrap gui/$(id -u) {}",
        plan.plist.display()
    ));
    app.chat.push(Cell::ToolResult(cells::ToolResult {
        name: "schedule".into(),
        output,
        success: true,
        duration_ms: 0,
    }));
}

fn plan(app: &App, at: (u32, u32)) -> Result<schedule::Plan, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    schedule::plan(&app.cfg, &exe, at).map_err(|e| e.to_string())
}

/// The tool header ("moneyball schedule") over the given paragraphs.
fn push(app: &mut App, body: Vec<Para>) {
    let mut paras = vec![
        Para::new(2, 2)
            .run("\u{23BF} ", dim())
            .run("\u{25CF}", fg(Color::Green))
            .run(" moneyball ", dim())
            .run("schedule", bold(Color::Cyan)),
        Para::default(),
    ];
    paras.extend(body);
    app.chat.push(Cell::Rich(cells::Rich(paras)));
}

fn clear_input(app: &mut App) {
    app.input.clear();
    app.cursor = 0;
}
