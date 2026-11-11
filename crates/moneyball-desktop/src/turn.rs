//! One advisor turn: start it on the session (core spawns the worker),
//! then a forwarder thread feeds every event to the session - which
//! records items as they finish - and on to the interface, with text
//! deltas coalesced so the webview gets one message per frame or two.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use moneyball_core::agent::{
    looks_like_context_overflow, model_configured, Ev, Item, Turn, TurnRefused,
};
use moneyball_core::{brief, AppConfig};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppError;
use crate::front_end::{crm_note, system_prompt, DESKTOP};
use crate::session::new_session;
use crate::state::{lock, AppState, SharedSession};

/// How long text deltas gather before they are sent as one.
const COALESCE: Duration = Duration::from_millis(30);
const DIED: &str = "The advisor stopped unexpectedly. Send your message again.";
const OVERFLOW_HINT: &str = "Start a new session to continue.";

/// What the interface hears during a turn: core's events as they are,
/// and two of the shell's own.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum TurnEvent {
    Core(Ev),
    Shell(ShellEv),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShellEv {
    /// The worker is gone with no final event.
    Died { message: String },
    /// An item missed the session file. Sent once; the chat goes on.
    SaveFailed { message: String },
}

/// The way out to the interface. A failed send means the webview is
/// gone (closed or reloaded): nothing more is sent, and the turn goes on
/// so its session file stays complete.
struct Outbox<S: Fn(TurnEvent) -> bool> {
    send: S,
    gone: bool,
    /// Deltas gathered since the last flush, and when to send them.
    text: String,
    due: Option<Instant>,
    save_warned: bool,
}

impl<S: Fn(TurnEvent) -> bool> Outbox<S> {
    fn push(&mut self, ev: TurnEvent) {
        if !self.gone {
            self.gone = !(self.send)(ev);
        }
    }

    fn delta(&mut self, text: &str) {
        self.text.push_str(text);
        let due = *self.due.get_or_insert_with(|| Instant::now() + COALESCE);
        if Instant::now() >= due {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.due = None;
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.push(TurnEvent::Core(Ev::AssistantDelta { text }));
        }
    }

    fn save_failed(&mut self, message: String) {
        if !self.save_warned {
            self.save_warned = true;
            self.push(TurnEvent::Shell(ShellEv::SaveFailed { message }));
        }
    }
}

/// Drain a turn. Every event is applied to the session before it is
/// sent, so by the time the interface sees a final event the session is
/// free for the next turn; a channel that closes without one frees it
/// too, and says so. A failure is kept on the session as well: a window
/// that reloaded mid-turn has no channel to hear it on.
pub fn forward(turn: Turn, session: &SharedSession, send: impl Fn(TurnEvent) -> bool) {
    let mut out = Outbox {
        send,
        gone: false,
        text: String::new(),
        due: None,
        save_warned: false,
    };
    if let Some(e) = turn.save_failed {
        out.save_failed(e.to_string());
    }
    let mut ended = false;
    while let Some(ev) = next(&turn.events, &mut out) {
        let ev = with_hint(ev);
        {
            let mut live = lock(session);
            if let Ev::Failed { error } = &ev {
                live.failed = Some(error.clone());
            }
            if let Err(e) = live.session.apply(&ev) {
                out.save_failed(e.to_string());
            }
        }
        match ev {
            Ev::AssistantDelta { text } => out.delta(&text),
            ev => {
                out.flush();
                ended |= matches!(
                    ev,
                    Ev::TurnComplete { .. } | Ev::TurnAborted | Ev::Failed { .. }
                );
                out.push(TurnEvent::Core(ev));
            }
        }
    }
    out.flush();
    {
        let mut live = lock(session);
        live.session.stream_closed();
        if !ended {
            live.failed = Some(DIED.into());
        }
    }
    if !ended {
        out.push(TurnEvent::Shell(ShellEv::Died {
            message: DIED.into(),
        }));
    }
}

/// The next event, flushing gathered text whenever it comes due first;
/// None when the worker has hung up.
fn next<S: Fn(TurnEvent) -> bool>(events: &Receiver<Ev>, out: &mut Outbox<S>) -> Option<Ev> {
    loop {
        let Some(due) = out.due else {
            return events.recv().ok();
        };
        match events.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(ev) => return Some(ev),
            Err(RecvTimeoutError::Timeout) => out.flush(),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// A failure that every later turn of this session would repeat says
/// what to do about it.
fn with_hint(ev: Ev) -> Ev {
    match ev {
        Ev::Failed { error } if looks_like_context_overflow(&error) => {
            let error = error.trim_end().trim_end_matches('.');
            Ev::Failed {
                error: format!("{}. {}", error, OVERFLOW_HINT),
            }
        }
        ev => ev,
    }
}

pub fn refused(why: TurnRefused) -> AppError {
    const SETUP: &str = "pick a provider and paste its key";
    match why {
        TurnRefused::Busy => AppError::busy("The advisor is still answering."),
        TurnRefused::NoModel => AppError::new("no_model", "No model is set up for this workspace.")
            .with_fix("/setup", SETUP),
        TurnRefused::UnknownProvider { provider } => AppError::new(
            "unknown_provider",
            format!(
                "The provider \"{}\" this workspace names is not in its list of model providers.",
                provider
            ),
        )
        .with_fix("/setup", SETUP),
    }
}

/// Start a turn on the session on screen; returns at once.
pub fn start(
    state: &AppState,
    cfg: &AppConfig,
    text: &str,
    send: impl Fn(TurnEvent) -> bool + Send + 'static,
) -> Result<(), AppError> {
    if text.trim_start().starts_with('/') {
        return Err(AppError::new(
            "not_a_command",
            "This app has no slash commands. Ask in plain words.",
        ));
    }
    if text.trim().is_empty() {
        return Err(AppError::new("empty", "Type a question first."));
    }
    // Core's check (`Session::start_turn` makes it again), made first so
    // a turn that cannot start does not create a session file.
    model_configured(cfg).map_err(refused)?;
    // The state block names the snapshot the answer will be read from.
    let loaded = brief::load(cfg);
    let snap_date = loaded.brief.as_ref().map(|b| b.snapshot_date.as_str());
    let note = loaded.crm_note.as_ref().and_then(|_| crm_note(cfg));
    let system = system_prompt(cfg, snap_date, note.as_deref());
    let user = Item::User { text: text.into() };
    // The slot stays held until the turn has started: two starts landing
    // together share one session (the second is refused as busy), and no
    // session_open can swap it out in between.
    let mut slot = state.session_slot();
    let session = slot.get_or_insert_with(|| new_session(cfg)).clone();
    let turn = {
        let mut live = lock(&session);
        live.failed = None;
        live.session.start_turn(cfg, &DESKTOP, &system, vec![user])
    };
    drop(slot);
    let turn = turn.map_err(refused)?;
    std::thread::spawn(move || forward(turn, &session, send));
    Ok(())
}

#[tauri::command(async)]
pub fn turn_start(
    state: State<'_, AppState>,
    text: String,
    on_event: Channel<TurnEvent>,
) -> Result<(), AppError> {
    let send = move |ev| on_event.send(ev).is_ok();
    start(&state, &state.workspace()?, &text, send)
}

/// The stream then ends with `turn_aborted`.
#[tauri::command]
pub fn turn_cancel(state: State<'_, AppState>) {
    if let Some(session) = state.session() {
        lock(&session).session.interrupt();
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
