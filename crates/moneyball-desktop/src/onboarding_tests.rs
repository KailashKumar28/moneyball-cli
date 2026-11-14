//! onboarding tests: the plain functions behind the commands, with Meta
//! and the home folder stood in for.

use std::cell::Cell;

use moneyball_core::{workspace as core_workspace, Error};

use super::*;

#[test]
fn cancel_stops_the_connect_on_screen_and_a_new_one_stops_the_last() {
    let state = AppState::default();
    let first = begin_connect(&state);
    assert!(!first.load(Ordering::SeqCst));
    // Continue with Facebook again while the first still waits.
    let second = begin_connect(&state);
    assert!(first.load(Ordering::SeqCst) && !second.load(Ordering::SeqCst));
    cancel_connect(&state);
    assert!(second.load(Ordering::SeqCst));
    // A declined connect is core's sentence in the sheet, with no terminal fix.
    let declined = Error::Authorization("Facebook sign-in was cancelled.".into());
    let e = in_sheet(declined);
    assert_eq!(
        (e.kind.as_str(), e.reason.as_str(), e.fix),
        ("authorization", "Facebook sign-in was cancelled.", None)
    );
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
