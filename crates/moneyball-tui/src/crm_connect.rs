//! /connect crm: core's CRM connect flow (connections::crm) inside the
//! app. It runs on a worker thread behind a Prompter that sends each line
//! and question here; the lines grow a tool cell, and a question becomes
//! the composer's prompt - masked when it is a secret - until enter
//! answers it or esc cancels the flow.

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

use moneyball_core::connections::crm::{connect, Done};
use moneyball_core::connections::Prompter;

use crate::chat::{cells, Cell};
use crate::App;
use crossterm::event::{KeyCode, KeyEvent};

enum Ev {
    Show(String),
    Ask {
        prompt: String,
        secret: bool,
    },
    /// Err: the error's kind and its sentence.
    Finished(Result<Done, (&'static str, String)>),
}

/// A connect in flight: its events, where answers go, and the question
/// waiting on the composer.
pub struct ConnectJob {
    rx: Receiver<Ev>,
    answers: Sender<Option<String>>,
    pub(crate) asking: Option<(String, bool)>,
}

struct Worker {
    tx: Sender<Ev>,
    answers: Receiver<Option<String>>,
}

impl Prompter for Worker {
    fn show(&mut self, line: &str) {
        let _ = self.tx.send(Ev::Show(line.to_string()));
    }
    fn ask(&mut self, prompt: &str, secret: bool) -> Option<String> {
        let _ = self.tx.send(Ev::Ask {
            prompt: prompt.trim_end().to_string(),
            secret,
        });
        // A closed channel (the app quit, /clear) is a cancel.
        self.answers.recv().ok().flatten()
    }
}

pub(crate) fn run(app: &mut App) {
    if app.connect.is_some() {
        app.chat
            .note("a connect is already running - answer it, or esc to cancel");
        return;
    }
    app.chat.push(Cell::ToolCall(cells::ToolCall {
        name: "connect".into(),
        args: "crm".into(),
        status: cells::ToolStatus::Running,
    }));
    app.chat.push(Cell::ToolResult(cells::ToolResult {
        name: "connect".into(),
        output: Vec::new(),
        success: true,
        duration_ms: 0,
    }));
    let (tx, rx) = channel();
    let (answer_tx, answers) = channel();
    // Quiet: the CRM pull's per-page progress goes to stderr otherwise,
    // over the TUI's screen.
    let cfg = moneyball_core::AppConfig {
        agent: true,
        ..app.cfg.clone()
    };
    std::thread::spawn(move || {
        let mut worker = Worker {
            tx: tx.clone(),
            answers,
        };
        let done = connect(&cfg, &mut worker).map_err(|e| (e.kind(), e.to_string()));
        let _ = tx.send(Ev::Finished(done));
    });
    app.connect = Some(ConnectJob {
        rx,
        answers: answer_tx,
        asking: None,
    });
}

/// Apply the worker's events; runs each tick.
pub(crate) fn drain(app: &mut App) {
    loop {
        let ev = match &app.connect {
            Some(job) => job.rx.try_recv(),
            None => return,
        };
        match ev {
            Ok(Ev::Show(line)) => append(app, line),
            Ok(Ev::Ask { prompt, secret }) => {
                if let Some(job) = &mut app.connect {
                    job.asking = Some((prompt.clone(), secret));
                }
                append(app, prompt);
            }
            Ok(Ev::Finished(done)) => return finish(app, done),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                return finish(app, Err(("", "the connect stopped unexpectedly".into())))
            }
        }
    }
}

/// While a question waits: enter answers it, esc cancels the flow. Other
/// keys type into the composer as usual. True when the key was ours.
pub(crate) fn on_key(app: &mut App, k: KeyEvent) -> bool {
    // Never the wizard's keys: /setup open over a waiting question keeps
    // its own enter and esc.
    if !matches!(app.view, crate::View::Brief) {
        return false;
    }
    let Some(job) = &mut app.connect else {
        return false;
    };
    let Some((_, secret)) = job.asking.clone() else {
        return false;
    };
    let answer = match k.code {
        KeyCode::Enter => Some(app.input.trim().to_string()),
        KeyCode::Esc => None,
        _ => return false,
    };
    let _ = job.answers.send(answer.clone());
    job.asking = None;
    app.input.clear();
    app.cursor = 0;
    app.status = None;
    // The transcript keeps what was answered, never a secret itself.
    let echo = match &answer {
        None => "(cancelled)".to_string(),
        Some(a) if secret && !a.is_empty() => "*".repeat(a.chars().count()),
        Some(a) => a.clone(),
    };
    append(app, format!("  > {}", echo));
    true
}

fn finish(app: &mut App, done: Result<Done, (&'static str, String)>) {
    app.connect = None;
    app.status = None;
    let (ok, line) = match done {
        Ok(Done::Connected) => (true, "CRM connected for this workspace.".to_string()),
        Ok(Done::NotSaved) => (true, "nothing saved.".to_string()),
        Ok(Done::Cancelled) => (true, "cancelled - nothing more was saved.".to_string()),
        Ok(Done::TestFailed) => (false, "the test pull failed - see above.".to_string()),
        Err((kind, e)) => {
            // In here, the retry is this command with a fresh key.
            let again = if kind == "crm_auth" {
                "\n/connect crm again with a fresh key"
            } else {
                ""
            };
            (false, format!("{}{}", e, again))
        }
    };
    append(app, String::new());
    for l in line.lines() {
        append(app, l.to_string());
    }
    // Its own cells by name: a chat turn may have started mid-connect.
    for cell in app.chat.cells.iter_mut().rev() {
        match cell {
            Cell::ToolCall(c) if c.name == "connect" && c.status == cells::ToolStatus::Running => {
                c.status = if ok {
                    cells::ToolStatus::Done
                } else {
                    cells::ToolStatus::Failed
                };
                break;
            }
            Cell::ToolResult(r) if r.name == "connect" => r.success = ok,
            _ => {}
        }
    }
    // The landing, the bar and /connect read the CRM afresh from here on.
    app.load_brief();
}

fn append(app: &mut App, line: String) {
    let cell = app.chat.cells.iter_mut().rev().find_map(|c| match c {
        Cell::ToolResult(r) if r.name == "connect" => Some(r),
        _ => None,
    });
    if let Some(r) = cell {
        r.output.push(line);
    }
}
