//! The demo workspace the fixtures are built from: `fixtures/` copied
//! into a temp folder laid out as a real workspace, with the clock held
//! at the snapshot's day so nothing in a fixture depends on today.

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, TimeZone};
use moneyball_core::brief::{self, Loaded};
use moneyball_core::health::State;
use moneyball_core::schema::RunRecord;
use moneyball_core::{report, runs, workspace, AppConfig};
use serde_json::json;

/// The snapshot on screen; its data day is Thu 08 Oct.
pub const DATE: &str = "2026-10-09";
const SNAPSHOTS: [&str; 2] = ["2026-10-08", DATE];

fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Remove a demo workspace and the temp folder made for it.
pub fn remove(cfg: &AppConfig) {
    if let Some(unique) = cfg.data_root.parent() {
        std::fs::remove_dir_all(unique).ok();
    }
}

/// A workspace under the temp dir: the config, and (`snapshots`) both
/// days with their daily reports, with or without CRM data.
pub fn workspace(name: &str, snapshots: bool, crm: bool) -> AppConfig {
    // One folder name for every demo: a workspace is named after its folder.
    let unique = format!("mb-desktop-{}-{}", name, std::process::id());
    let root = std::env::temp_dir().join(unique).join("acme-homes");
    std::fs::remove_dir_all(&root).ok();
    let dot = root.join(".moneyball");
    std::fs::create_dir_all(&dot).unwrap();
    std::fs::copy(source().join("config.json"), dot.join("config.json")).unwrap();
    for date in SNAPSHOTS.iter().filter(|_| snapshots) {
        let to = dot.join("history/snap").join(date);
        std::fs::create_dir_all(&to).unwrap();
        for file in ["ads_daily.json", "adsets.json", "crm.json"] {
            if crm || file != "crm.json" {
                std::fs::copy(source().join("snap").join(date).join(file), to.join(file)).unwrap();
            }
        }
    }
    let cfg = workspace::open(&root, None).unwrap();
    for date in SNAPSHOTS.iter().filter(|_| snapshots) {
        report::generate(&cfg, Some(date), 1).unwrap();
    }
    cfg
}

/// The run that filled DATE, as a refresh would have recorded it: clean
/// with CRM; without, CRM skipped (not connected) and a report that
/// says so. Its artifacts point at the report `workspace` generated.
pub fn run_record(cfg: &AppConfig, crm: bool) -> RunRecord {
    let (rep, html) = report::daily(cfg, DATE).unwrap();
    let cards: usize = rep.products.iter().map(|p| p.creatives.len()).sum();
    let crm_step = match crm {
        true => json!({"name": "crm", "status": "done", "ms": 9400,
            "counts": {"tickets": 59, "pages": 1}}),
        false => json!({"name": "crm", "status": "skipped", "ms": 0,
            "message": "no CRM connected for this workspace", "fix": "/connect crm"}),
    };
    let mut report_step = json!({"name": "report", "status": "done", "ms": 1300,
        "counts": {"creatives": cards}});
    if !crm {
        report_step["status"] = json!("partial");
        report_step["error_kind"] = json!("no_crm");
        report_step["message"] =
            json!("written without CRM data: CRM figures left out, verdicts withheld");
    }
    let json_path = html.with_file_name("creative-report.json");
    serde_json::from_value(json!({
        "schema": "moneyball.run/1",
        "workspace_id": cfg.workspace_id(),
        "date": DATE,
        "trigger": "manual",
        "started_at": "2026-10-09T02:00:00Z",
        "finished_at": "2026-10-09T02:01:21Z",
        "outcome": if crm { "ok" } else { "degraded" },
        "steps": [
            {"name": "meta", "status": "done", "ms": 64300,
             "counts": {"products": 2, "rows": 40}},
            {"name": "assets", "status": "done", "ms": 0,
             "counts": {"creatives": 5, "leads": 96, "adsets": 4, "changes": 3}},
            crm_step,
            report_step,
        ],
        "artifacts": {"report": html, "report_json": json_path},
    }))
    .unwrap()
}

/// The workspace loaded as it read on the morning of DATE: the run
/// record on disk, and the health reading's clock set to that day.
pub fn loaded(cfg: &AppConfig, crm: bool) -> Loaded {
    runs::write(cfg, &run_record(cfg, crm)).unwrap();
    let mut loaded = brief::load(cfg);
    let at = |h, m, s| {
        let day: NaiveDate = DATE.parse().unwrap();
        Local
            .from_local_datetime(&day.and_hms_opt(h, m, s).unwrap())
            .single()
    };
    let h = &mut loaded.health;
    h.today = DATE.parse().unwrap();
    h.refreshed_at = at(7, 31, 21);
    h.state = if crm { State::Fresh } else { State::CrmMissing };
    loaded
}

/// What a generated report reads from the real clock: when it was
/// generated, and the "pulled ..." time its masthead words from that.
/// Both are set to the demo's morning.
pub fn report_clock(cfg: &AppConfig) -> [(&'static str, serde_json::Value); 1] {
    let (rep, html) = report::daily(cfg, DATE).unwrap();
    let page = std::fs::read_to_string(html)
        .unwrap()
        .replace(&rep.generated_at, "2026-10-09T02:01:21Z");
    let (head, rest) = page.split_once("pulled ").unwrap();
    let (_, tail) = rest.split_once(" &middot;").unwrap();
    let page = format!("{}pulled 09 Oct, 07:31 am IST &middot;{}", head, tail);
    [("/html", json!(page))]
}
