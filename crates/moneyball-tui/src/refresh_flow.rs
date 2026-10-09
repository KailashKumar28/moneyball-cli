//! /refresh: the daily refresh inside the TUI. A worker runs
//! refresh::run on its own channel (not the shared `app.stream` slot), so
//! the chat still answers while it runs; each step starting and finishing
//! updates the refresh cell in place, and Esc stops it after the step in
//! flight. /fetch is its old name, and /brief self-heals through it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use crate::chat::Cell;
use crate::refresh_cell::{End, RefreshCell, Row};
use crate::widgets::day_numbers;
use crate::*;
use moneyball_core::refresh::{self, Options, Progress};
use moneyball_core::schema::{RunRecord, RunStep, Trigger};

/// What the worker tells the event loop.
pub enum Ev {
    Begin(&'static str),
    Done(Box<RunStep>),
    Finished { run: Box<RunRecord>, ms: u64 },
    CouldNotStart { err: String },
}

/// A refresh in flight: its events, its cancel flag, and whether the
/// brief runs once it lands (/brief's self-heal).
pub struct RefreshJob {
    rx: Receiver<Ev>,
    cancel: Arc<AtomicBool>,
    then_brief: bool,
    /// The step running now (1-based), for the composer's status line.
    pub(crate) step: usize,
}

const STOPPING: &str = "stopping after the current step...";

pub(crate) fn run_refresh(app: &mut App, force: bool, then_brief: bool) {
    if let Some(job) = &mut app.refresh {
        // /brief asking mid-run: let this run bring the brief, not a second one.
        job.then_brief |= then_brief;
        app.status = Some(if then_brief {
            "a refresh is already running - the brief follows it".into()
        } else {
            "a refresh is already running - its steps are above".into()
        });
        return;
    }
    let day = refresh::data_day_label();
    // "Thu 08 Oct 2026" -> "Thu 08 Oct": the year is noise on screen.
    let short = day
        .rsplit_once(' ')
        .map_or(day.clone(), |(d, _)| d.to_string());
    app.chat.push(Cell::Refresh(Box::new(RefreshCell::new(
        short,
        app.schedule.is_none(),
    ))));
    let (tx, rx) = std::sync::mpsc::channel::<Ev>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cfg = app.cfg.clone();
    let opts = Options {
        trigger: Trigger::Manual,
        force,
        days: 28,
        cancel: Some(cancel.clone()),
    };
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        // The token is resolved at the edge (core never reads it itself).
        let token = moneyball_core::secrets::load_meta_token();
        let step_tx = tx.clone();
        let result = refresh::run(&cfg, token.as_deref(), &opts, &mut |p| {
            let _ = step_tx.send(match p {
                Progress::Begin(name) => Ev::Begin(name),
                Progress::Done(step) => Ev::Done(Box::new(step.clone())),
            });
        });
        let _ = tx.send(match result {
            Ok(run) => Ev::Finished {
                run: Box::new(run),
                ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => Ev::CouldNotStart { err: e.reason() },
        });
    });
    app.refresh = Some(RefreshJob {
        rx,
        cancel,
        then_brief,
        step: 1,
    });
}

/// Esc with nothing else to stop: finish the step in flight, skip the
/// rest. False when no refresh is running.
pub(crate) fn cancel(app: &mut App) -> bool {
    let Some(job) = &app.refresh else {
        return false;
    };
    job.cancel.store(true, Ordering::SeqCst);
    app.status = Some(STOPPING.into());
    true
}

/// Apply every pending worker event; runs each tick next to drain_stream.
pub(crate) fn drain(app: &mut App) {
    loop {
        let ev = match &app.refresh {
            Some(job) => job.rx.try_recv(),
            None => return,
        };
        match ev {
            Ok(Ev::Begin(name)) => {
                if let Some(job) = &mut app.refresh {
                    job.step = refresh::STEPS
                        .iter()
                        .position(|s| *s == name)
                        .map_or(1, |i| i + 1);
                }
                with_cell(app, |c| c.set(name, Row::Running));
            }
            Ok(Ev::Done(step)) => {
                let name = step.name.clone();
                with_cell(app, |c| c.set(&name, Row::Done(*step)));
            }
            Ok(Ev::Finished { run, ms }) => {
                let then_brief = app.refresh.take().is_some_and(|j| j.then_brief);
                return finished(app, *run, ms, then_brief);
            }
            Ok(Ev::CouldNotStart { err }) => {
                app.refresh = None;
                return with_cell(app, |c| c.end = Some(End::CouldNotStart(err)));
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // The worker died without its last event (a panic): say so,
                // never leave the cell spinning.
                app.refresh = None;
                return with_cell(app, |c| {
                    c.end = Some(End::Stopped(
                        "its run record keeps the steps that finished".into(),
                    ))
                });
            }
        }
    }
}

fn finished(app: &mut App, run: RunRecord, ms: u64, then_brief: bool) {
    let last_good = app
        .health
        .as_ref()
        .and_then(|h| h.data_day())
        .map(|d| d.format("%d %b").to_string());
    // The run says where its report went; the numbers come from its json.
    let report = run.artifacts.get("report").cloned();
    let numbers = run
        .artifacts
        .get("report_json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<moneyball_core::schema::CreativeReport>(&raw).ok())
        .map(|r| day_numbers(&r));
    if let Some(html) = &report {
        app.report_path = Some(html.into());
    }
    with_cell(app, |c| {
        c.end = Some(End::Finished {
            run: Box::new(run),
            ms,
            numbers,
            report,
            last_good,
        })
    });
    if app.status.as_deref() == Some(STOPPING) {
        app.status = None;
    }
    // The bar and the brief read the new snapshot from here on.
    app.load_brief();
    // Only with a snapshot, so a failed pull never loops back into /brief;
    // and never over a chat answer that is still streaming.
    if then_brief && app.brief.is_some() {
        if app.stream.is_some() {
            app.status = Some("the data is in - /brief reads it".into());
        } else {
            crate::brief_flow::run_brief(app);
        }
    }
}

/// The newest refresh cell (a /clear mid-run leaves none: nothing to draw).
fn with_cell(app: &mut App, f: impl FnOnce(&mut RefreshCell)) {
    let cell = app.chat.cells.iter_mut().rev().find_map(|c| match c {
        Cell::Refresh(r) => Some(r),
        _ => None,
    });
    if let Some(c) = cell {
        f(c);
    }
}

/// Ctrl+O: open the newest daily report in the default browser.
pub(crate) fn open_report(app: &mut App) {
    let Some(path) = app.report_path.clone() else {
        app.status = Some("no report yet - /refresh writes one".into());
        return;
    };
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    // `open` hands the file to the browser and returns at once; waiting
    // for it reaps the child (a bare spawn would leave a zombie behind).
    let opened = std::process::Command::new(opener)
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    app.status = Some(match opened {
        Ok(s) if s.success() => format!("opened {}", name),
        Ok(s) => format!("could not open {} ({})", path.display(), s),
        Err(e) => format!("could not open {}: {}", path.display(), e),
    });
}
