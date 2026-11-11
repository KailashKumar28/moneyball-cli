//! The fixtures that are streams or sessions: a refresh's events and
//! the states a reload reads during one, an advisor turn's events, and
//! the session list and snapshots. Each stream is core events hand-fed
//! to the real forwarder (a real refresh or turn needs the network).

use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use moneyball_core::agent::tool_text::funnel_output;
use moneyball_core::agent::{Ev, Item, SaveFailed, Session, Turn, Usage, TURN_ABORTED_MARKER};
use moneyball_core::funnel::By;
use moneyball_core::refresh::Ev as RefreshEv;
use moneyball_core::schema::RunRecord;
use moneyball_core::{debug, session, snapshot, AppConfig};
use serde_json::{json, Value};

use super::{demo, pinned, Pins};
use crate::front_end::DESKTOP;
use crate::refresh::{forward, state_view, RefreshEvent, RefreshRun};
use crate::state::{lock, AppState};
use crate::turn::{self, TurnEvent};

/// The events a run sends, as core's worker would have sent them: Begin
/// and Done for a step that ran, Done alone for one that was skipped.
fn worker_events(run: &RunRecord) -> Vec<RefreshEv> {
    let mut out = Vec::new();
    for step in &run.steps {
        let name = moneyball_core::refresh::STEPS
            .iter()
            .find(|s| **s == step.name)
            .unwrap();
        if step.ms > 0 || step.name == "assets" {
            out.push(RefreshEv::Begin { step: name });
        }
        out.push(RefreshEv::Done(Box::new(step.clone())));
    }
    out.push(RefreshEv::Finished {
        run: Box::new(run.clone()),
        ms: 81_000,
    });
    out
}

/// Forward `events`; returns what was sent and the state a reload would
/// have read as each one arrived.
fn refresh_run(events: Vec<RefreshEv>) -> (Vec<RefreshEvent>, Vec<Value>, Value) {
    let shared = Mutex::new(RefreshRun::begin(Arc::default()));
    let clock = [("/day", json!("Thu 08 Oct"))];
    let state = |ms: Value| {
        let mut v = pinned(state_view(&lock(&shared)), &clock);
        if !v["started_ms_ago"].is_null() {
            v["started_ms_ago"] = ms;
        }
        v
    };
    let waiting = state(json!(0));
    let (tx, rx) = channel();
    events.into_iter().for_each(|ev| tx.send(ev).unwrap());
    drop(tx);
    let seen = Mutex::new((Vec::new(), Vec::new()));
    forward(rx, &shared, Some("07 Oct".into()), |ev| {
        let mut seen = lock(&seen);
        seen.0.push(ev);
        seen.1.push(state(json!(52_000)));
        true
    });
    let (events, states) = seen.into_inner().unwrap();
    (events, states, waiting)
}

pub fn refresh(pins: &mut Pins, cfg: &AppConfig, nocrm: &AppConfig) {
    let (events, states, waiting) = refresh_run(worker_events(&demo::run_record(cfg, true)));
    pins.add("refresh_state_waiting", cfg, waiting);
    // As the CRM step begins: two steps done, one running, one waiting.
    let crm_begins = events
        .iter()
        .position(|e| matches!(e, RefreshEvent::Begin { step } if step.name == "crm"))
        .unwrap();
    pins.add("refresh_state_running", cfg, &states[crm_begins]);
    pins.add("refresh_state", cfg, states.last().unwrap());
    pins.add("refresh_events", cfg, events);

    let (degraded, _, _) = refresh_run(worker_events(&demo::run_record(nocrm, false)));
    pins.add("refresh_finished_degraded", nocrm, degraded.last().unwrap());

    let refused = RefreshEv::CouldNotStart {
        error: "a refresh is already running for this workspace - wait for it to finish".into(),
    };
    let mut other = refresh_run(vec![refused]).0;
    other.extend(refresh_run(vec![RefreshEv::Begin { step: "meta" }]).0.pop());
    pins.add("refresh_events_other", cfg, other);
}

/// Forward a hand-fed turn on `session`; returns what was sent.
fn turn_run(events: Vec<Ev>, save_failed: Option<SaveFailed>) -> Vec<TurnEvent> {
    let (tx, rx) = channel();
    events.into_iter().for_each(|ev| tx.send(ev).unwrap());
    drop(tx);
    let turn = Turn {
        events: rx,
        save_failed,
    };
    let session = Arc::new(Mutex::new(Session::new(None, Vec::new())));
    let seen = Mutex::new(Vec::new());
    turn::forward(turn, &session, |ev| {
        lock(&seen).push(ev);
        true
    });
    seen.into_inner().unwrap()
}

const QUESTION: &str = "Which Namma Mane ad set should I cut?";
const ANSWER: &str = "Cut **Pincode Bengaluru East**: it spent past the kill line with no \
qualified lead, and none of its leads are still maturing.\n\n| Ad set | Verdict |\n|---|---|\n\
| Lookalike 1% | keep |\n| Pincode Bengaluru East | kill |";

/// One whole turn in history order: the question, a funnel tool round
/// with the tool's real output, the answer.
fn conversation(cfg: &AppConfig) -> Vec<Item> {
    let snap = snapshot::load(&cfg.snap_for(None).unwrap()).unwrap();
    let output = funnel_output(cfg, &snap, "Namma Mane", By::Adset, &DESKTOP.say);
    vec![
        Item::User {
            text: QUESTION.into(),
        },
        Item::ToolCall {
            call_id: "call_1".into(),
            name: "funnel".into(),
            args: json!({"product": "Namma Mane", "by": "adset"}),
        },
        Item::ToolOutput {
            call_id: "call_1".into(),
            output,
            is_error: false,
        },
        Item::Assistant {
            text: ANSWER.into(),
        },
    ]
}

/// The events of `conversation` as the agent loop emits them.
fn turn_events(items: &[Item]) -> Vec<Ev> {
    let mut out = Vec::new();
    for item in items {
        let done = Ev::ItemDone { item: item.clone() };
        match item {
            Item::ToolCall {
                call_id,
                name,
                args,
            } => out.push(Ev::ToolBegin {
                call_id: call_id.clone(),
                name: name.clone(),
                args: args.clone(),
            }),
            Item::ToolOutput {
                call_id, output, ..
            } => {
                out.push(Ev::ToolEnd {
                    call_id: call_id.clone(),
                    output: output.clone(),
                    ok: true,
                });
                // The loop records the call, then its output.
                let call = out.remove(out.len() - 2);
                out.push(Ev::ItemDone {
                    item: items[1].clone(),
                });
                out.insert(out.len() - 2, call);
            }
            Item::Assistant { text } => {
                let deltas = text.split_inclusive(' ').map(|t| Ev::AssistantDelta {
                    text: t.to_string(),
                });
                out.extend(deltas);
                out.push(Ev::AssistantDone { text: text.clone() });
            }
            Item::User { .. } => {}
        }
        if !matches!(item, Item::ToolCall { .. }) {
            out.push(done);
        }
    }
    out.push(Ev::TurnComplete {
        ms: 8_400,
        provider: "anthropic".into(),
        usage: Usage {
            input: 5_120,
            output: 96,
        },
    });
    out
}

pub fn turns(pins: &mut Pins) {
    let cfg = demo::workspace("turns", true, true);
    pins.add(
        "turn_events",
        &cfg,
        turn_run(turn_events(&conversation(&cfg)), None),
    );
    let unsaved = SaveFailed("No space left on device (os error 28)".into());
    let mut other = turn_run(vec![Ev::TurnAborted], Some(unsaved));
    let overflow = Ev::Failed {
        error: "prompt is too long: 210000 tokens > 200000 maximum".into(),
    };
    other.extend(turn_run(vec![overflow], None));
    other.extend(turn_run(vec![], None));
    pins.add("turn_events_other", &cfg, other);
    demo::remove(&cfg);
}

/// A session file as core writes it, under a fixed id.
fn saved(cfg: &AppConfig, id: &str, started: &str, items: &[Item]) {
    let dir = session::sessions_dir(cfg.sessions_root()).unwrap();
    let header = json!({"session": {"id": id, "started_at": started, "data_root": cfg.data_root}});
    let lines = items.iter().map(|i| serde_json::to_string(i).unwrap());
    let body: Vec<String> = std::iter::once(header.to_string()).chain(lines).collect();
    std::fs::write(dir.join(format!("{}.jsonl", id)), body.join("\n") + "\n").unwrap();
}

pub fn sessions(pins: &mut Pins, cfg: &AppConfig) {
    let state = AppState::default();
    pins.add("session_new", cfg, crate::session::snapshot(None));
    let (plain, marked) = ("mb-20261009T020500Z-k3x9", "mb-20261008T113000Z-p7qd");
    saved(cfg, plain, "2026-10-09T02:05:00Z", &conversation(cfg));
    let user = |text: &str| Item::User { text: text.into() };
    let call = |id: &str, product: &str| Item::ToolCall {
        call_id: id.into(),
        name: "funnel".into(),
        args: json!({"product": product, "by": "ad"}),
    };
    let interrupted = [
        user("How is Cityville doing by ad?"),
        call("call_1", "City"),
        Item::ToolOutput {
            call_id: "call_1".into(),
            output: "no product matches \"City\"".into(),
            is_error: true,
        },
        call("call_2", "Cityville"),
        user(TURN_ABORTED_MARKER),
        user(&debug::marker_text("it never answered")),
    ];
    saved(cfg, marked, "2026-10-08T11:30:00Z", &interrupted);
    let open = |id| crate::session::open(&state, cfg, id).unwrap();
    pins.add("session", cfg, open(plain));
    pins.add("session_markers", cfg, open(marked));
    // modified_at is the file's own time: pin it to when each was started.
    let list = session::summaries(cfg.sessions_root()).unwrap();
    let mut list = serde_json::to_value(list).unwrap();
    let rows = list.as_array_mut().unwrap();
    rows.sort_by_key(|s| std::cmp::Reverse(s["started_at"].as_str().map(String::from)));
    for row in rows {
        row["modified_at"] = row["started_at"].clone();
    }
    pins.add("sessions", cfg, list);
}
