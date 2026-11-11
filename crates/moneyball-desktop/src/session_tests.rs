//! Which sessions the app lists and opens, and the session slot under
//! commands that land together. No model is reached: the one provider
//! here has no key, so a turn ends before any request is built.

use std::path::PathBuf;
use std::sync::Arc;

use moneyball_core::agent::Item;
use moneyball_core::workspace;

use super::*;
use crate::front_end::DESKTOP;

/// A workspace on disk whose model is set up, with a provider no key
/// exists for.
fn workspace_with_model(name: &str) -> (PathBuf, AppConfig) {
    let root = std::env::temp_dir().join(format!("mb-desktop-{}-{}", name, std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join(".moneyball")).unwrap();
    let config = serde_json::json!({
        "products": [{"name": "P", "ad_account": "act_1"}],
        "model_provider": "mb-desktop-test-no-key",
        "model": "m",
        "model_providers": {"mb-desktop-test-no-key": {
            "name": "None", "base_url": "http://127.0.0.1:9/v1", "wire_api": "chat_completions"
        }},
    });
    std::fs::write(root.join(".moneyball/config.json"), config.to_string()).unwrap();
    let cfg = workspace::open(&root, None).unwrap();
    (root, cfg)
}

fn user(text: &str) -> Item {
    Item::User { text: text.into() }
}

#[test]
fn only_sessions_this_app_wrote_are_listed_and_opened() {
    let (root, cfg) = workspace_with_model("sessions");
    let written_by = |front_end: &str, said: &str| {
        let log =
            SessionLog::create(cfg.data_root.clone(), cfg.sessions_root(), front_end).unwrap();
        log.append(&user(said)).unwrap();
        log.meta.id
    };
    let ours = written_by(FRONT_END, "how are we doing?");
    let terminal = written_by("tui", "/funnel Namma Mane");
    // A file from before the header named its front-end.
    let dir = session::sessions_dir(cfg.sessions_root()).unwrap();
    let old = r#"{"session":{"id":"mb-old","started_at":"2026-11-01T10:00:00Z","data_root":"/w"}}"#;
    std::fs::write(dir.join("mb-old.jsonl"), format!("{}\n", old)).unwrap();

    let listed: Vec<String> = list(&cfg).into_iter().map(|s| s.id).collect();
    assert_eq!(listed, std::slice::from_ref(&ours));
    let state = AppState::default();
    assert_eq!(open(&state, &cfg, &ours).unwrap().items.len(), 1);
    for id in [terminal.as_str(), "mb-old", "../x", ""] {
        let err = open(&state, &cfg, id).unwrap_err();
        assert_eq!(err.kind, "session", "{}", id);
    }
    assert_eq!(snapshot(state.session().as_ref()).id, Some(ours));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn turns_that_start_together_share_one_session() {
    let (root, cfg) = workspace_with_model("together");
    let state = AppState::default();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            // Refused as busy or started: either way on the one session.
            scope.spawn(|| crate::turn::start(&state, &cfg, "hello", |_| true).ok());
        }
    });
    assert_eq!(
        session::summaries(cfg.sessions_root()).unwrap().len(),
        1,
        "one session file, however many turns asked for one"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_session_is_not_swapped_out_while_its_turn_runs() {
    let (root, cfg) = workspace_with_model("swap");
    let state = AppState::default();
    let session = new_session(&cfg);
    let id = snapshot(Some(&session)).id.unwrap();
    *state.session_slot() = Some(session.clone());
    // Started and never drained: the turn stays in flight.
    let turn = lock(&session)
        .session
        .start_turn(&cfg, &DESKTOP, "sys", vec![user("hi")])
        .unwrap();
    assert_eq!(replace_if_idle(&state, None).unwrap_err().kind, "busy");
    assert_eq!(open(&state, &cfg, &id).unwrap_err().kind, "busy");
    assert!(state.session().is_some_and(|s| Arc::ptr_eq(&s, &session)));
    drop(turn);
    lock(&session).session.stream_closed();
    replace_if_idle(&state, None).unwrap();
    assert!(state.session().is_none());
    std::fs::remove_dir_all(&root).ok();
}
