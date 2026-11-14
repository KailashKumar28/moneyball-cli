//! The drift alarm of plans/desktop/IPC.md: one JSON example of every
//! return type and event variant, built from `fixtures/` through the
//! real command bodies and pinned under `ui/src/ipc/fixtures/`, where
//! the interface checks each against its TypeScript types. A Rust shape
//! change fails here until the files are regenerated and reviewed:
//!
//!     MB_UPDATE_FIXTURES=1 cargo test -p moneyball-desktop fixtures

pub(crate) mod demo;
mod streams;
mod wording;

use std::path::{Path, PathBuf};

use moneyball_core::AppConfig;
use serde::Serialize;
use serde_json::{json, Value};

use crate::brief::brief_view;
use crate::error::AppError;
use crate::front_end::crm_note;
use crate::funnel::funnel_view;
use crate::today::today_view;
use crate::{report, workspace};

/// Where the demo workspace says it lives, in every fixture.
const SHOWN_ROOT: &str = "/Users/demo/acme-homes";

/// The fixtures gathered by one run: (file name, pinned JSON text).
pub struct Pins(Vec<(&'static str, String)>);

impl Pins {
    /// Pin `value` under `name`, with the temp workspace's path replaced
    /// by one that does not change from run to run.
    pub fn add(&mut self, name: &'static str, cfg: &AppConfig, value: impl Serialize) {
        let json = serde_json::to_string_pretty(&value).unwrap();
        let root = cfg.data_root.display().to_string();
        self.0
            .push((name, format!("{}\n", json.replace(&root, SHOWN_ROOT))));
    }
}

/// `value` as JSON with the fields that read the real clock set to what
/// they were on the demo's day: (pointer, pinned value).
pub fn pinned(value: impl Serialize, clock: &[(&str, Value)]) -> Value {
    let mut v = serde_json::to_value(value).unwrap();
    for (pointer, to) in clock {
        *v.pointer_mut(pointer)
            .unwrap_or_else(|| panic!("no {}", pointer)) = to.clone();
    }
    v
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src/ipc/fixtures")
}

/// What Today reads from the real clock: when the data was refreshed
/// (a local time, so its offset is the machine's), and its report's
/// page, which says when it was generated and pulled.
fn today_clock(cfg: &AppConfig) -> Vec<(&'static str, Value)> {
    let [(_, page)] = demo::report_clock(cfg);
    vec![
        ("/health/refreshed_at", json!("2026-10-09T07:31:21+05:30")),
        ("/day/report/html", page),
    ]
}

/// Workspace, Today, Brief, Funnel and Report for the demo with CRM
/// data and without it.
fn screens(pins: &mut Pins) {
    let cfg = demo::workspace("crm", true, true);
    let nocrm = demo::workspace("nocrm", true, false);
    pins.add("workspace", &cfg, workspace::view(&cfg));

    let today = today_view(&cfg, &demo::loaded(&cfg, true), false);
    pins.add("today", &cfg, pinned(today, &today_clock(&cfg)));
    let degraded = today_view(&nocrm, &demo::loaded(&nocrm, false), false);
    pins.add("today_degraded", &nocrm, pinned(degraded, &today_clock(&nocrm)));

    // The demo is always "old" by the real clock; on its own day it is not.
    let fresh = [("/staleness", Value::Null)];
    let brief = brief_view(demo::loaded(&cfg, true), crm_note(&cfg));
    pins.add("brief", &cfg, pinned(brief, &fresh));

    let funnel = |cfg: &AppConfig| funnel_view(cfg, "namma", 7, "adset").unwrap();
    pins.add("funnel", &cfg, funnel(&cfg));
    pins.add("funnel_nocrm", &nocrm, funnel(&nocrm));

    pins.add("report_days", &cfg, report::report_days_of(&cfg));
    let written = report::report_for_date(&cfg, None).unwrap().unwrap();
    let generated = report::generate(&cfg, Some(demo::DATE.into()), 1).unwrap();
    assert_eq!(written.path, generated.path);
    pins.add("report", &cfg, pinned(generated, &demo::report_clock(&cfg)));

    streams::refresh(pins, &cfg, &nocrm);
    streams::sessions(pins, &cfg);
    pins.add("errors", &cfg, errors(&cfg));
    for cfg in [cfg, nocrm] {
        demo::remove(&cfg);
    }
}

/// One of each failure the interface words a screen for, keyed as its
/// mock asks for them.
fn errors(cfg: &AppConfig) -> Value {
    use moneyball_core::agent::TurnRefused;
    let state = crate::state::AppState::default();
    let broken = demo::workspace("broken", false, false);
    let snap = broken.snap_dir().join(demo::DATE);
    std::fs::create_dir_all(&snap).unwrap();
    std::fs::write(snap.join("ads_daily.json"), "{ not json").unwrap();
    let fault = brief_view(moneyball_core::brief::load(&broken), None).fault;
    demo::remove(&broken);
    let no_config = moneyball_core::workspace::open(Path::new("/nonexistent"), None).unwrap_err();
    let unknown = TurnRefused::UnknownProvider {
        provider: "acme".into(),
    };
    json!({
        "busy": crate::turn::refused(TurnRefused::Busy),
        "no_model": crate::turn::refused(TurnRefused::NoModel),
        "unknown_provider": crate::turn::refused(unknown),
        "not_a_command": crate::turn::start(&state, cfg, "/refresh", |_| true).unwrap_err(),
        "no_workspace_config": AppError::from(no_config),
        "no_workspace": AppError::no_workspace(),
        // Brief's fault, and Funnel's and Report's error, before any pull.
        "no_data": AppError::no_data(),
        "refresh_busy": report::refresh_busy(),
        "snapshot": fault.expect("a snapshot that will not parse is a fault"),
    })
}

#[test]
fn fixtures_match_the_command_bodies() {
    let mut pins = Pins(Vec::new());
    screens(&mut pins);
    streams::turns(&mut pins);
    wording::nothing_shown_is_a_slash_command(&pins.0);
    let update = std::env::var_os("MB_UPDATE_FIXTURES").is_some();
    let dir = fixtures_dir();
    let mut stale = Vec::new();
    for (name, json) in &pins.0 {
        let path = dir.join(format!("{}.json", name));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, json).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(pinned) if pinned == *json => {}
            Ok(_) => stale.push(format!("{}.json (differs)", name)),
            Err(_) => stale.push(format!("{}.json (missing)", name)),
        }
    }
    assert!(
        stale.is_empty(),
        "IPC fixtures out of date in {}:\n{}\nregenerate with MB_UPDATE_FIXTURES=1 cargo test \
         -p moneyball-desktop fixtures, and review the diff",
        dir.display(),
        stale.join("\n")
    );
}
