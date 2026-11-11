//! Session driver tests. Hermetic: the one turn that runs ends at the
//! missing API key, before any request is built.

use super::*;
use crate::config::WorkspaceConfig;

const FRONT: prompt::FrontEnd = prompt::FrontEnd {
    commands: &[],
    output_style: "",
    say: |step| step.to_string(),
};

fn user(text: &str) -> Item {
    Item::User { text: text.into() }
}

/// A workspace whose provider has no key anywhere (not in auth.json,
/// no env_key): a turn on it fails without touching the network.
fn cfg_with_model() -> AppConfig {
    let mut cfg = AppConfig::resolve_optional(Some("/nonexistent/mb-driver-test"), None);
    let mut provider = ModelProviderInfo::openai();
    provider.env_key = None;
    let id = "mb-driver-test-no-key".to_string();
    cfg.workspace = Some(WorkspaceConfig {
        workspace_id: None,
        brand: None,
        products: vec![],
        goals: Default::default(),
        target_rs_per_q: None,
        crm: Default::default(),
        model_provider: Some(id.clone()),
        model: Some("m".into()),
        model_providers: [(id, provider)].into_iter().collect(),
    });
    cfg
}

#[test]
fn a_refused_turn_records_nothing() {
    let mut cfg = cfg_with_model();
    let mut s = Session::new(None, vec![]);
    if let Some(w) = cfg.workspace.as_mut() {
        w.model_providers.clear();
    }
    let refused = s
        .start_turn(&cfg, &FRONT, "sys", vec![user("hi")])
        .unwrap_err();
    assert_eq!(
        refused,
        TurnRefused::UnknownProvider {
            provider: "mb-driver-test-no-key".into()
        }
    );
    if let Some(w) = cfg.workspace.as_mut() {
        w.model = None;
    }
    let refused = s
        .start_turn(&cfg, &FRONT, "sys", vec![user("hi")])
        .unwrap_err();
    assert_eq!(refused, TurnRefused::NoModel);
    cfg.workspace = None;
    let refused = s
        .start_turn(&cfg, &FRONT, "sys", vec![user("hi")])
        .unwrap_err();
    assert_eq!(refused, TurnRefused::NoModel);
    assert!(s.history().is_empty());
    assert!(!s.turn_active());
    // The IPC shape of a refusal.
    assert_eq!(
        serde_json::to_value(TurnRefused::Busy).unwrap(),
        serde_json::json!({"type": "busy"})
    );
}

/// One turn at a time; the final event frees the session; a model
/// failure is an event, never a dead turn.
#[test]
fn one_turn_at_a_time_until_its_final_event() {
    let cfg = cfg_with_model();
    let mut s = Session::new(None, vec![]);
    assert!(!s.interrupt(), "nothing to interrupt yet");
    let turn = s.start_turn(&cfg, &FRONT, "sys", vec![user("hi")]).unwrap();
    assert!(turn.save_failed.is_none());
    assert!(s.turn_active());
    let busy = s
        .start_turn(&cfg, &FRONT, "sys", vec![user("again")])
        .unwrap_err();
    assert_eq!(busy, TurnRefused::Busy);
    assert_eq!(s.history().len(), 1, "the refused turn is not recorded");

    let evs: Vec<Ev> = turn.events.iter().collect();
    assert!(matches!(evs.last(), Some(Ev::Failed { error }) if error.contains("no API key")));
    assert!(s.interrupt(), "in flight until the final event is applied");
    for ev in &evs {
        s.apply(ev).unwrap();
    }
    assert!(!s.turn_active());
    assert!(s
        .start_turn(&cfg, &FRONT, "sys", vec![user("again")])
        .is_ok());
}

#[test]
fn a_dead_worker_frees_the_session() {
    let mut s = Session::new(None, vec![]);
    s.turn_active = true;
    s.stream_closed();
    assert!(!s.turn_active());
}

/// Finished items go to the history and the session file as they arrive.
#[test]
fn applied_items_reach_the_history_and_the_file() {
    let _env = crate::session::ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ws = std::env::temp_dir().join(format!("mb-driver-test-{}", std::process::id()));
    let log = SessionLog::create(ws.clone(), Some(&ws)).unwrap();
    let mut s = Session::new(Some(log), vec![]);
    let id = s.id().unwrap().to_string();
    s.record(user("hi")).unwrap();
    let done = Ev::ItemDone {
        item: Item::Assistant {
            text: "hello".into(),
        },
    };
    s.apply(&done).unwrap();
    assert_eq!(s.history().len(), 2);
    let (_, saved) = SessionLog::open(&id, Some(&ws)).unwrap();
    assert!(matches!(&saved[..], [Item::User { .. }, Item::Assistant { text }] if text == "hello"));

    // The file gone: the item stays in the history, and the miss is said.
    std::fs::remove_dir_all(&ws).unwrap();
    let missed = s.record(user("more")).unwrap_err();
    assert!(missed.to_string().starts_with("session save failed: "));
    assert_eq!(s.history().len(), 3);
}

#[test]
fn a_typed_command_opens_a_user_item_and_its_tool_round() {
    let s = Session::new(None, vec![user("earlier")]);
    let turn = s.typed_tool_turn("/brief", "brief", serde_json::json!({}), "numbers".into());
    assert!(matches!(&turn[0], Item::User { text } if text == "/brief"));
    assert!(matches!(&turn[1], Item::ToolCall { call_id, name, .. }
        if call_id == "typed-1" && name == "brief"));
    assert!(
        matches!(&turn[2], Item::ToolOutput { call_id, output, is_error: false }
        if call_id == "typed-1" && output == "numbers")
    );
}

#[test]
fn context_overflow_is_recognized_across_providers() {
    assert!(looks_like_context_overflow(
        "400: This model's maximum context length is 128000 tokens"
    ));
    assert!(looks_like_context_overflow("Prompt is too long: 250000"));
    assert!(!looks_like_context_overflow("401 invalid api key"));
}

/// The IPC shape of the stream: tagged like `Item`, snake_case.
#[test]
fn events_serialize_tagged_snake_case() {
    let json = |ev: Ev| serde_json::to_value(ev).unwrap();
    assert_eq!(
        json(Ev::AssistantDelta { text: "hi".into() }),
        serde_json::json!({"type": "assistant_delta", "text": "hi"})
    );
    assert_eq!(
        json(Ev::ItemDone { item: user("q") }),
        serde_json::json!({"type": "item_done", "item": {"type": "user", "text": "q"}})
    );
    assert_eq!(
        json(Ev::TurnComplete {
            ms: 5,
            provider: "p".into(),
            usage: crate::agent::Usage {
                input: 10,
                output: 2
            },
        }),
        serde_json::json!({"type": "turn_complete", "ms": 5, "provider": "p",
            "usage": {"input": 10, "output": 2}})
    );
    assert_eq!(
        json(Ev::TurnAborted),
        serde_json::json!({"type": "turn_aborted"})
    );
}
