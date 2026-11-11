//! The turn forwarder against a hand-fed channel: no model, no network.

use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use moneyball_core::agent::Session;
use serde_json::Value;

use super::*;

fn session() -> SharedSession {
    Arc::new(Mutex::new(Session::new(None, Vec::new())))
}

fn delta(text: &str) -> Ev {
    Ev::AssistantDelta { text: text.into() }
}

fn answer(text: &str) -> Ev {
    Ev::ItemDone {
        item: Item::Assistant { text: text.into() },
    }
}

/// Feed `events`, hang up, and collect what the interface was sent
/// (`heard` false = the webview is gone and every send fails).
fn run(events: Vec<Ev>, session: &SharedSession, heard: bool) -> Vec<Value> {
    let (tx, rx) = channel();
    for ev in events {
        tx.send(ev).unwrap();
    }
    drop(tx);
    let turn = Turn {
        events: rx,
        save_failed: None,
    };
    let seen = Mutex::new(Vec::new());
    forward(turn, session, |ev| {
        lock(&seen).push(serde_json::to_value(&ev).unwrap());
        heard
    });
    seen.into_inner().unwrap()
}

fn types(seen: &[Value]) -> Vec<&str> {
    seen.iter().map(|e| e["type"].as_str().unwrap()).collect()
}

#[test]
fn deltas_arrive_as_one_and_before_what_follows_them() {
    let s = session();
    let seen = run(
        vec![
            delta("Spend "),
            delta("is "),
            delta("up."),
            Ev::AssistantDone {
                text: "Spend is up.".into(),
            },
            answer("Spend is up."),
            Ev::TurnAborted,
        ],
        &s,
        true,
    );
    assert_eq!(
        types(&seen),
        [
            "assistant_delta",
            "assistant_done",
            "item_done",
            "turn_aborted"
        ]
    );
    assert_eq!(seen[0]["text"], "Spend is up.");
    assert_eq!(lock(&s).history().len(), 1, "the item was recorded");
}

#[test]
fn text_still_waiting_is_flushed_when_its_time_comes() {
    let (tx, rx) = channel();
    let turn = Turn {
        events: rx,
        save_failed: None,
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (s, heard) = (session(), seen.clone());
    let worker = std::thread::spawn(move || {
        forward(turn, &s, |ev| {
            lock(&heard).push(serde_json::to_value(&ev).unwrap());
            true
        })
    });
    tx.send(delta("slow")).unwrap();
    // Nothing follows the delta: the timer alone must send it.
    let waited = Instant::now();
    while lock(&seen).is_empty() && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lock(&seen)[0]["text"], "slow");
    drop(tx);
    worker.join().unwrap();
    assert_eq!(types(&lock(&seen)), ["assistant_delta", "died"]);
}

#[test]
fn a_worker_that_hangs_up_early_is_reported_once_the_text_is_out() {
    let seen = run(vec![delta("half an ans")], &session(), true);
    assert_eq!(types(&seen), ["assistant_delta", "died"]);
    assert_eq!(seen[1]["message"], DIED);
}

#[test]
fn a_webview_that_is_gone_stops_the_sending_not_the_recording() {
    let s = session();
    let seen = run(
        vec![answer("one"), answer("two"), Ev::TurnAborted],
        &s,
        false,
    );
    assert_eq!(seen.len(), 1, "one failed send, then silence");
    assert_eq!(lock(&s).history().len(), 2, "the session file stays whole");
}

#[test]
fn an_overflow_failure_says_to_start_a_new_session() {
    let overflow = Ev::Failed {
        error: "prompt is too long: 210000 tokens".into(),
    };
    let seen = run(vec![overflow], &session(), true);
    assert_eq!(types(&seen), ["failed"]);
    assert_eq!(
        seen[0]["error"],
        "prompt is too long: 210000 tokens Start a new session to continue."
    );
    let other = Ev::Failed {
        error: "HTTP 500".into(),
    };
    assert_eq!(run(vec![other], &session(), true)[0]["error"], "HTTP 500");
}

#[test]
fn a_slash_command_or_nothing_never_reaches_the_model() {
    let state = AppState::default();
    let cfg = AppConfig::resolve_optional(Some("/nonexistent/mb-desktop-turn"), None);
    let slash = start(&state, &cfg, " /refresh", |_| true).unwrap_err();
    assert_eq!(slash.kind, "not_a_command");
    assert_eq!(
        start(&state, &cfg, "  ", |_| true).unwrap_err().kind,
        "empty"
    );
    let no_model = start(&state, &cfg, "how are we doing?", |_| true).unwrap_err();
    assert_eq!(no_model.kind, "no_model");
    assert!(no_model.fix.is_some());
    assert!(state.session().is_none(), "no session file for a refusal");
}
