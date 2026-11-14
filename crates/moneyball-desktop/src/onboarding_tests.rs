//! onboarding tests: the plain functions behind the commands, with Meta
//! and the home folder stood in for.

use std::cell::Cell;

use moneyball_core::{workspace as core_workspace, Error};
use serde_json::json;

use super::*;
use crate::state::lock;

fn checked() -> moneyball_core::Result<TokenCheck> {
    TokenCheck::read(
        "Acme System User",
        2,
        &json!({"expires_at": 0, "scopes": ["ads_read"]}),
    )
}

#[test]
fn a_token_is_stored_trimmed_and_only_once_meta_accepts_it() {
    let stored = std::cell::RefCell::new(None);
    let store = |t: &str| {
        *stored.borrow_mut() = Some(t.to_string());
        Ok(())
    };
    // Shape first: refused with no call to Meta and nothing stored.
    let e = connect(" 1234567890 ", meta::check_token, store).unwrap_err();
    assert_eq!((e.kind.as_str(), e.fix), ("meta_token_shape", None));
    assert_eq!(*stored.borrow(), None);
    // Meta refuses: core's sentence, no terminal fix, nothing stored.
    let refused = |_: &str| {
        Err(Error::MetaAuth(
            "Meta refused the access token (code 190)".into(),
        ))
    };
    let e = connect("EAAbad", refused, store).unwrap_err();
    assert_eq!(e.kind, "meta_auth");
    assert_eq!(e.reason, "Meta refused the access token (code 190)");
    assert_eq!(e.fix, None);
    assert_eq!(*stored.borrow(), None);
    // Accepted: stored as Meta saw it, and the token never comes back.
    let seen = Cell::new(false);
    let check = |t: &str| {
        seen.set(t == "EAAgood");
        checked()
    };
    let ok = connect("  EAAgood\n", check, store).unwrap();
    assert!(seen.get());
    assert_eq!(stored.borrow().as_deref(), Some("EAAgood"));
    assert!(!serde_json::to_string(&ok).unwrap().contains("EAAgood"));
}

#[test]
fn accounts_carry_metas_id_and_the_clients_own_are_ticked() {
    let choice = |id: &str, name: &str, used_by: Option<&str>| onboarding::AccountChoice {
        id: id.into(),
        name: name.into(),
        status: "active",
        used_by: used_by.map(String::from),
    };
    let all = || {
        vec![
            choice("101", "ACME Homes - Namma Mane", None),
            choice("202", "Fincity", Some("used by Fincity")),
        ]
    };
    let got = account_choices(" acme homes ", all());
    assert_eq!(got[0].id, "act_101");
    assert!(got[0].suggested, "case and spaces aside");
    assert!(!got[1].suggested);
    assert_eq!(got[1].used_by.as_deref(), Some("used by Fincity"));
    assert!(account_choices("", all()).iter().all(|a| !a.suggested));
}

#[test]
fn create_opens_the_client_and_refuses_while_a_refresh_runs() {
    let root = std::env::temp_dir().join(format!("mb-desktop-create-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let made = root.join("Acme Homes");
    moneyball_core::config::update(&made, |w| {
        w.products = vec![Product {
            name: "P".into(),
            ad_account: "1".into(),
        }]
    })
    .unwrap();
    let state = AppState::default();
    let make = || core_workspace::open(&made, None);
    let remembered = Cell::new(false);
    let remember = |_: &Path| {
        remembered.set(true);
        Ok(())
    };

    lock(&state.refresh).running = true;
    let called = Cell::new(false);
    let e = create(
        &state,
        || {
            called.set(true);
            make()
        },
        remember,
    )
    .unwrap_err();
    assert_eq!(e.kind, "busy");
    assert!(!called.get() && !remembered.get(), "nothing made");
    lock(&state.refresh).running = false;

    let refused = || {
        Err(Error::Config(
            "Pick at least one ad account for Acme.".into(),
        ))
    };
    let e = create(&state, refused, remember).unwrap_err();
    assert_eq!((e.kind.as_str(), e.fix), ("config", None));
    assert_eq!(state.workspace().unwrap_err().kind, "no_workspace");

    let shown = create(&state, make, remember).unwrap();
    assert_eq!(shown.name, "Acme Homes");
    assert!(remembered.get());
    assert_eq!(workspace::view(&state.workspace().unwrap()), shown);
    std::fs::remove_dir_all(&root).ok();
}
