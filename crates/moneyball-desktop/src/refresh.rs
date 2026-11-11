//! The refresh: core's worker on its own thread, and a forwarder thread
//! that turns each of its events into the interface's words, keeps the
//! state a reload reads while it runs (`refresh_state`), and sends it
//! down the channel. At rest the state is the workspace's own last run
//! record, so nothing about a run lives only in memory or the webview.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use moneyball_core::refresh::{self, Ev, Options, STEPS};
use moneyball_core::schema::{Outcome, Trigger};
use moneyball_core::{runs, secrets, AppConfig};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppError;
use crate::refresh_end::{finished, recorded_ms, Finished};
use crate::run_view::{all_waiting, done, run_view, running, StepView};
use crate::state::{lock, AppState};

const DIED: &str =
    "The refresh stopped unexpectedly. Its run record keeps the steps that finished.";

/// The longest window a snapshot can answer for: a refresh pulls this
/// many trailing days, so a funnel or a report asking for more would
/// come up short without saying so.
pub const SNAPSHOT_DAYS: u32 = 28;

/// The refresh in flight. Read only while `running`: once a run is
/// over, its record on disk is the truth.
#[derive(Default)]
pub struct RefreshRun {
    pub running: bool,
    /// The data day it fills: "Thu 08 Oct".
    day: String,
    steps: Vec<StepView>,
    started: Option<Instant>,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RefreshState {
    pub running: bool,
    pub day: String,
    pub steps: Vec<StepView>,
    pub started_ms_ago: Option<u64>,
    /// The last run as its `finished` event gave it; None while one
    /// runs, before any run, and for a run that never finished.
    pub finished: Option<Finished>,
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

/// The run in flight as a reload reads it; None when none is running.
fn in_flight(run: &RefreshRun) -> Option<RefreshState> {
    run.running.then(|| RefreshState {
        running: true,
        day: run.day.clone(),
        steps: run.steps.clone(),
        started_ms_ago: run.started.map(|t| t.elapsed().as_millis() as u64),
        finished: None,
    })
}

/// The workspace's last recorded run, whoever made it (this app, the
/// terminal, the schedule), in full: built through `finished`, the code
/// behind the live event. A record still marked running is a run in
/// flight elsewhere or one that died: its steps, and no verdict.
fn recorded(cfg: &AppConfig) -> Option<RefreshState> {
    let run = runs::latest(cfg)?;
    let view = run_view(&run);
    let over = run.outcome != Outcome::Running;
    Some(RefreshState {
        running: false,
        day: view.day,
        steps: view.steps,
        started_ms_ago: None,
        finished: over.then(|| finished(&run, recorded_ms(&run), last_good(cfg).as_deref())),
    })
}

/// The refresh as the screen shows it on mount, on focus and after a
/// reload: the run in flight while there is one, else the open
/// workspace's last recorded run, else nothing.
pub fn state_view(run: &Mutex<RefreshRun>, cfg: Option<&AppConfig>) -> RefreshState {
    if let Some(live) = in_flight(&lock(run)) {
        return live;
    }
    cfg.and_then(recorded).unwrap_or_default()
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
                over();
                ended = true;
                RefreshEvent::Finished(Box::new(finished(&run, ms, last_good.as_deref())))
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

/// The data day of the newest snapshot ("27 Sep"): what is on screen
/// before a run, and still is after one that failed.
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
        days: SNAPSHOT_DAYS,
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
    state_view(&state.refresh, state.workspace().ok().as_ref())
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
