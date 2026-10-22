//! Event tests: paste routing and keys. Split from event.rs (size cap).

use super::*;
use std::path::PathBuf;

fn make_state(step: usize, substep: u8) -> SetupState {
    let mut s = SetupState::new(PathBuf::from("/tmp/mb-test"));
    s.workspace_path = "/tmp/mb-test".into();
    s.step = step;
    s.meta_substep = substep;
    s
}

fn app_with_setup(state: SetupState) -> App {
    let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
    let mut app = App::new_for_test(cfg);
    app.force_setup_for_test(state);
    app
}

#[test]
fn paste_meta_token_into_substep0() {
    let s = make_state(1, 0);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "EAA12345abcdefghij".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.meta_input, "EAA12345abcdefghij");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_strips_newlines_and_control_chars() {
    let s = make_state(1, 0);
    let mut app = app_with_setup(s);
    // Real clipboard often wraps a token with a trailing newline.
    handle_paste(&mut app, "EAA12345\n".into());
    handle_paste(&mut app, "abc\tdef\r".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.meta_input, "EAA12345abcdef");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_workspace_step() {
    let s = make_state(0, 0);
    let mut app = app_with_setup(s);
    // Clear the default workspace path before pasting.
    match &mut app.view {
        View::Setup(state) => state.workspace_path.clear(),
        _ => unreachable!(),
    }
    handle_paste(&mut app, "/tmp/pasted-workspace".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.workspace_path, "/tmp/pasted-workspace");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_product_input() {
    let s = make_state(2, 0);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "FincityOfficial act_1".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.product_input, "FincityOfficial act_1");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_goals_input() {
    let s = make_state(3, 0);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "Namma Mane=12".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.goals_input, "Namma Mane=12");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_multi_select_is_ignored() {
    // substep 1 has no text input; paste should not change selection.
    let s = make_state(1, 1);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "should be dropped".into());
    match app.view {
        View::Setup(state) => {
            assert!(state.meta_input.is_empty());
            assert!(state.meta_rename_input.is_empty());
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_rename_substep() {
    let s = make_state(1, 2);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "1=BrandName".into());
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.meta_rename_input, "1=BrandName");
        }
        _ => panic!("expected Setup view"),
    }
}

#[test]
fn paste_into_brief_input() {
    let cfg = AppConfig::resolve_optional(Some("/tmp/mb-test"), None);
    let mut app = App::new_for_test(cfg);
    // resolve_optional yields a setup view when no workspace config exists,
    // so we override that here for the brief-view paste test.
    app.force_welcome_for_test();
    handle_paste(&mut app, "what is my best product?".into());
    assert_eq!(app.input, "what is my best product?");
}

#[test]
fn empty_paste_is_noop() {
    let s = make_state(1, 0);
    let mut app = app_with_setup(s);
    handle_paste(&mut app, "".into());
    match app.view {
        View::Setup(state) => assert!(state.meta_input.is_empty()),
        _ => panic!("expected Setup view"),
    }
}

/// Esc on the multi-select step (substep 1) returns to the token paste
/// (substep 0) WITHOUT nuking the discovered accounts or the user's
/// per-row selections. The token input is restored as N bullets so the
/// box doesn't look empty. Regression test for the bug where going
/// back from substep 1 dropped all of `meta_discovered` / selections.
#[test]
fn esc_from_multi_select_preserves_state() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use moneyball_core::meta::AdAccount;
    let mut s = make_state(1, 1);
    s.meta_token_len = 124;
    s.meta_discovered = vec![
        AdAccount {
            id: "act_1".into(),
            name: "Acme".into(),
            account_status: Some(1),
        },
        AdAccount {
            id: "act_2".into(),
            name: "Beta".into(),
            account_status: Some(1),
        },
    ];
    s.meta_selections = vec![true, false];
    s.meta_selected = vec![0];
    s.meta_highlight = 1;
    let snapshot = s.clone();
    let mut app = app_with_setup(s);
    setup::handle_setup_key(
        &mut app,
        snapshot,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.meta_substep, 0, "should be back on token paste");
            assert_eq!(state.meta_discovered.len(), 2, "discovered list preserved");
            assert_eq!(
                state.meta_selections,
                vec![true, false],
                "selections preserved"
            );
            assert_eq!(state.meta_selected, vec![0], "selected indices preserved");
            // Token input restored as N bullets.
            assert_eq!(state.meta_input.chars().count(), 124);
            assert!(state.meta_input.chars().all(|c| c == '\u{2022}'));
        }
        _ => panic!("expected Setup view"),
    }
}

/// Esc on the rename step (substep 2) returns to the multi-select
/// (substep 1) WITHOUT nuking selections; only the rename input is
/// cleared so the user can re-type.
#[test]
fn esc_from_rename_preserves_selections() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut s = make_state(1, 2);
    s.meta_rename_input = "1=Acme".into();
    let snapshot = s.clone();
    let mut app = app_with_setup(s);
    setup::handle_setup_key(
        &mut app,
        snapshot,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.meta_substep, 1, "should be back on multi-select");
            assert!(state.meta_rename_input.is_empty(), "rename input cleared");
        }
        _ => panic!("expected Setup view"),
    }
}

/// Regression test: the LLM provider picker must NOT eat Enter /
/// Esc / Backspace / Char keys. Only Up/Down/Home/End belong to
/// the picker. If the picker consumed Enter, advance_setup never
/// ran and the user was stuck. This is the bug the user hit in
/// the initial setup.
#[test]
fn llm_picker_does_not_consume_enter() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut s = make_state(4, 0);
    s.llm_highlight = 0;
    let snapshot = s.clone();
    let mut app = app_with_setup(s);
    setup::handle_setup_key(
        &mut app,
        snapshot,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    match app.view {
        View::Setup(state) => {
            // Enter on the provider picker should pick "openai"
            // (the first preset) and advance to substep 1 (key paste).
            assert_eq!(state.llm_substep, 1, "should advance to key paste");
            assert_eq!(state.llm_provider_id, "openai");
        }
        _ => panic!("expected Setup view"),
    }
}

/// Picker nav (Up/Down) should still work and be consumed by the
/// picker (so it doesn't fall through to backspace/insert).
#[test]
fn llm_picker_down_arrow_advances_highlight() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut s = make_state(4, 0);
    s.llm_highlight = 0;
    s.llm_scroll = 0;
    let snapshot = s.clone();
    let mut app = app_with_setup(s);
    setup::handle_setup_key(
        &mut app,
        snapshot,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    );
    match app.view {
        View::Setup(state) => {
            assert_eq!(state.llm_highlight, 1, "Down should move highlight");
        }
        _ => panic!("expected Setup view"),
    }
}
