//! The refresh: core's worker on its own thread, and a forwarder thread
//! that turns each of its events into the interface's words, keeps the
//! state a reload reads (`refresh_state`), and sends it down the channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use moneyball_core::refresh::{self, Ev, Options, STEPS};
use moneyball_core::schema::Trigger;
use moneyball_core::{secrets, AppConfig};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppError;
use crate::refresh_end::{finished, Finished};
use crate::run_view::{all_waiting, done, run_view, running, StepView};
use crate::state::{lock, AppState};

const DIED: &str =
    "The refresh stopped unexpectedly. Its run record keeps the steps that finished.";

/// The refresh in flight, or the last one this app ran.
#[derive(Default)]
pub struct RefreshRun {
    pub running: bool,
    /// The data day it fills: "Thu 08 Oct".
    day: String,
    steps: Vec<StepView>,
    started: Option<Instant>,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefreshState {
    pub running: bool,
    pub day: String,
    pub steps: Vec<StepView>,
    pub started_ms_ago: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RefreshEvent {
    /// A step started: its row, status "running".
    Begin {
        step: StepView,
    },
    Done {
        step: StepView,
    },
    Finished(Box<Finished>),
    CouldNotStart {
        error: AppError,
    },
    /// The worker's channel closed with no final event.
    Died {
        message: String,
    },
}

/// The day a refresh started now fills, without its year.
fn day_label() -> String {
    let day = refresh::data_day_label();
    day.rsplit_once(' ').map_or(day.clone(), |(d, _)| d.into())
}

impl RefreshRun {
    /// A run that has just started: four steps waiting.
    pub fn begin(cancel: Arc<AtomicBool>) -> Self {
        RefreshRun {
            running: true,
            day: day_label(),
            steps: all_waiting(),
            started: Some(Instant::now()),
            cancel: Some(cancel),
        }
    }
}

pub fn state_view(run: &RefreshRun) -> RefreshState {
    let fresh = run.steps.is_empty();
    RefreshState {
        running: run.running,
        day: if fresh { day_label() } else { run.day.clone() },
        steps: if fresh {
            all_waiting()
        } else {
            run.steps.clone()
        },
        started_ms_ago: run
            .started
            .filter(|_| run.running)
            .map(|t| t.elapsed().as_millis() as u64),
    }
}

fn set_step(run: &Mutex<RefreshRun>, step: &StepView) {
    let mut run = lock(run);
    if let Some(i) = STEPS.iter().position(|s| *s == step.name) {
        if let Some(slot) = run.steps.get_mut(i) {
            *slot = step.clone();
        }
    }
}

/// Drain the worker's events: reword each, keep `shared` current, send
/// it. A failed send means the webview is gone; the run goes on, and the
/// state is still kept for the reload. The run is marked over before
/// its last event is sent, so the interface can act on that event.
pub fn forward(
    events: Receiver<Ev>,
    shared: &Mutex<RefreshRun>,
    last_good: Option<String>,
    send: impl Fn(RefreshEvent) -> bool,
) {
    let over = || lock(shared).running = false;
    let mut ended = false;
    for ev in events {
        let out = match ev {
            Ev::Begin { step } => {
                let step = running(step);
                set_step(shared, &step);
                RefreshEvent::Begin { step }
            }
            Ev::Done(step) => {
                let step = done(&step);
                set_step(shared, &step);
                RefreshEvent::Done { step }
            }
            Ev::Finished { run, ms } => {
                let day = {
                    let mut state = lock(shared);
                    state.steps = run_view(&run).steps;
                    state.running = false;
                    state.day.clone()
                };
                ended = true;
                RefreshEvent::Finished(Box::new(finished(&run, ms, last_good.as_deref(), &day)))
            }
            Ev::CouldNotStart { error } => {
                over();
                ended = true;
                RefreshEvent::CouldNotStart {
                    error: AppError::new("could_not_start", error),
                }
            }
        };
        send(out);
    }
    if !ended {
        over();
        send(RefreshEvent::Died {
            message: DIED.into(),
        });
    }
}

/// The data day on screen before a run ("27 Sep"), kept when it fails.
fn last_good(cfg: &AppConfig) -> Option<String> {
    let snap = cfg.snap_for(None).ok()?;
    let date = chrono::NaiveDate::parse_from_str(snap.file_name()?.to_str()?, "%Y-%m-%d").ok()?;
    Some(date.pred_opt()?.format("%d %b").to_string())
}

/// Start a refresh unless one is running; returns at once.
pub fn start(
    cfg: AppConfig,
    shared: Arc<Mutex<RefreshRun>>,
    force: bool,
    send: impl Fn(RefreshEvent) -> bool + Send + 'static,
) -> Result<(), AppError> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut run = lock(&shared);
        if run.running {
            return Err(AppError::busy("A refresh is already running."));
        }
        *run = RefreshRun::begin(cancel.clone());
    }
    let last_good = last_good(&cfg);
    let opts = Options {
        trigger: Trigger::Manual,
        force,
        days: 28,
        cancel: Some(cancel),
    };
    // The token is read here, at the edge, and goes to core only.
    let (events, _) = refresh::spawn(cfg, secrets::load_meta_token(), opts);
    std::thread::spawn(move || forward(events, &shared, last_good, send));
    Ok(())
}

#[tauri::command(async)]
pub fn refresh_start(
    state: State<'_, AppState>,
    force: Option<bool>,
    on_event: Channel<RefreshEvent>,
) -> Result<(), AppError> {
    let send = move |ev| on_event.send(ev).is_ok();
    start(
        state.workspace()?,
        state.refresh.clone(),
        force.unwrap_or(false),
        send,
    )
}

/// Takes effect after the step in flight.
#[tauri::command]
pub fn refresh_cancel(state: State<'_, AppState>) {
    let run = lock(&state.refresh);
    if let Some(cancel) = run.cancel.as_ref().filter(|_| run.running) {
        cancel.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn refresh_state(state: State<'_, AppState>) -> RefreshState {
    state_view(&lock(&state.refresh))
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
