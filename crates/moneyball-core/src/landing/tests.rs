//! The landing's states, each ending with the command that fixes it.

use chrono::NaiveDate;

use super::day::signed;
use super::story::missed;
use super::*;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
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

fn cfg() -> AppConfig {
    AppConfig {
        data_root: std::env::temp_dir().join("mb-landing-none"),
        date: None,
        workspace: None,
        agent: true,
    }
}

#[test]
fn missed_mornings_count_the_days_between() {
    assert_eq!(
        missed(d("2026-09-28"), d("2026-10-08")).as_deref(),
        Some("9 mornings missed, 29 Sep to 07 Oct.")
    );
    assert_eq!(
        missed(d("2026-10-06"), d("2026-10-08")).as_deref(),
        Some("1 morning missed, 07 Oct.")
    );
    assert_eq!(missed(d("2026-10-07"), d("2026-10-08")), None);
    assert_eq!(signed(-3532, "Rs."), "-Rs.3,532");
    assert_eq!(signed(2, ""), "+2");
}

#[test]
fn each_state_ends_with_the_command_that_fixes_it() {
    let cfg = cfg();
    let none = landing(&cfg, &health(State::NoData, None), false);
    let head = none.headline.unwrap();
    assert_eq!(
        (head.tint, head.text.as_str()),
        (Tint::Warn, "No data yet.")
    );
    assert_eq!(none.next.unwrap().command, "/refresh");
    assert_eq!(
        none.schedule.map(|s| s.command).as_deref(),
        Some("/schedule"),
        "nothing scheduled: offer it"
    );
    assert!(none.day.is_none());

    let stale = landing(&cfg, &health(State::Stale, Some("2026-09-28")), true);
    let head = stale.headline.unwrap();
    assert_eq!(head.text, "Your data is 11 days old.");
    assert_eq!(head.rest, "10 mornings missed, 29 Sep to 08 Oct.");
    assert!(stale.details[0].starts_with("CRM has never been pulled"));
    assert_eq!(stale.details.len(), 1, "scheduled: no 'on its own' line");
    assert!(stale.schedule.is_none(), "already scheduled");

    // A first refresh that failed: its fix, never "last good data".
    let failed = landing(&cfg, &health(State::Failed, None), false);
    let head = failed.headline.unwrap();
    assert_eq!(
        (head.tint, head.text.as_str()),
        (Tint::Bad, "Today's refresh failed.")
    );
    assert!(failed.details.is_empty());
}

#[test]
fn a_chosen_or_fresh_day_asks_for_nothing() {
    let cfg = cfg();
    let pinned = landing(&cfg, &health(State::Pinned, Some("2026-08-12")), false);
    let head = pinned.headline.unwrap();
    assert_eq!(head.tint, Tint::Plain);
    assert_eq!(head.text, "Viewing Tue 11 Aug (--date).");
    assert_eq!(head.rest, "The newest data is Thu 08 Oct.");
    assert!(pinned.next.is_none() && pinned.schedule.is_none());

    let fresh = landing(&cfg, &health(State::Fresh, Some("2026-10-09")), false);
    assert!(fresh.headline.is_none() && fresh.next.is_none());
    // No report on disk: the day says so and names what writes one.
    let day = fresh.day.unwrap();
    assert_eq!(day.day, "Thu 08 Oct");
    assert!(matches!(day.report, DayReport::Missing { next, .. } if next.command == "/refresh"));
}

#[test]
fn a_missing_crm_connection_is_fixed_by_connecting_it() {
    let mut h = health(State::CrmMissing, Some("2026-10-09"));
    h.crm_connected = false;
    let l = landing(&cfg(), &h, true);
    assert_eq!(l.fixes, vec![Command::fix("/connect crm")]);
    assert!(l.fixes[0].does.starts_with("pick your CRM"));
    // Today's Meta pull is in: the refresh only redoes CRM and the report.
    assert!(l.next.unwrap().does.starts_with("reuses today's Meta pull"));
}
