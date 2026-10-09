//! The landing's states, each ending with the command that fixes it.

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
fn missed_mornings_count_the_days_between() {
    assert_eq!(
        missed(d("2026-09-28"), d("2026-10-08")),
        " 9 mornings missed, 29 Sep to 07 Oct."
    );
    assert_eq!(
        missed(d("2026-10-06"), d("2026-10-08")),
        " 1 morning missed, 07 Oct."
    );
    assert_eq!(missed(d("2026-10-07"), d("2026-10-08")), "");
    assert_eq!(signed(-3532, "Rs."), "-Rs.3,532");
    assert_eq!(signed(2, ""), "+2");
}

#[test]
fn each_state_ends_with_the_command_that_fixes_it() {
    let cfg = AppConfig {
        data_root: std::env::temp_dir().join("mb-landing-none"),
        date: None,
        workspace: None,
        agent: true,
    };
    let none = text(&landing(&cfg, &health(State::NoData, None), false).0);
    assert!(none.contains("No data yet") && none.contains("next:  /refresh"));
    assert!(none.contains("/schedule"), "nothing scheduled: offer it");

    let stale = text(&landing(&cfg, &health(State::Stale, Some("2026-09-28")), true).0);
    assert!(stale.contains("Your data is 11 days old"));
    assert!(stale.contains("CRM has never been pulled"));
    assert!(!stale.contains("/schedule"), "already scheduled");

    // A first refresh that failed: its fix, never "last good data".
    let failed = text(&landing(&cfg, &health(State::Failed, None), false).0);
    assert!(failed.contains("Today's refresh failed"));
    assert!(!failed.contains("last good data"));

    let pinned = text(&landing(&cfg, &health(State::Pinned, Some("2026-08-12")), false).0);
    assert!(pinned.contains("Viewing Tue 11 Aug (--date). The newest data is Thu 08 Oct."));
    assert!(
        !pinned.contains("/refresh   "),
        "a chosen day asks for nothing"
    );

    let fresh = landing(&cfg, &health(State::Fresh, Some("2026-10-09")), false).0;
    assert!(
        !text(&fresh).contains("/refresh   "),
        "fresh asks for nothing"
    );
}
