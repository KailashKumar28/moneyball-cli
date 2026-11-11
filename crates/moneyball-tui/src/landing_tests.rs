//! How the landing's words are laid out (the words themselves are tested
//! in moneyball_core::landing).

use chrono::NaiveDate;
use moneyball_core::health::State;

use super::*;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn text(paras: &[Para]) -> String {
    paras
        .iter()
        .flat_map(|p| p.lines(140))
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn health(state: State, snap: Option<&str>) -> Health {
    Health {
        today: d("2026-10-09"),
        snap_date: snap.map(d),
        crm_snap_date: None,
        crm_connected: true,
        run: None,
        refreshed_at: None,
        failed_today: None,
        newest_snap_date: Some(d("2026-10-09")),
        state,
    }
}

#[test]
fn each_state_is_laid_out_with_its_command_set_apart() {
    let cfg = AppConfig {
        data_root: std::env::temp_dir().join("mb-landing-none"),
        date: None,
        workspace: None,
        agent: true,
    };
    let none = text(&landing(&cfg, &health(State::NoData, None), false).0);
    assert!(none.starts_with("  ! No data yet. Nothing has been pulled"));
    assert!(none.contains("\n\n    next:  /refresh    pulls Meta"));
    assert!(none.contains("\n           /schedule   so it happens every morning"));

    let stale = text(&landing(&cfg, &health(State::Stale, Some("2026-09-28")), true).0);
    assert!(stale.starts_with("  ! Your data is 11 days old. 10 mornings missed"));
    assert!(stale.contains("\n    CRM has never been pulled"));
    assert!(!stale.contains("/schedule"), "already scheduled");

    let failed = text(&landing(&cfg, &health(State::Failed, None), false).0);
    assert!(failed.starts_with("  \u{2716} Today's refresh failed."));

    let pinned = text(&landing(&cfg, &health(State::Pinned, Some("2026-08-12")), false).0);
    assert!(pinned.starts_with("  Viewing Tue 11 Aug (--date). The newest data is Thu 08 Oct."));
    assert!(
        !pinned.contains("/refresh   "),
        "a chosen day asks for nothing"
    );

    let mut h = health(State::CrmMissing, Some("2026-10-09"));
    h.crm_connected = false;
    let crm = text(&landing(&cfg, &h, true).0);
    assert!(crm.contains("\n    fix:  /connect crm   pick your CRM"));
    assert!(crm.contains("\n          then /refresh   reuses today's Meta pull"));
    assert!(crm.contains("no daily report for this day yet - /refresh writes one"));
}
