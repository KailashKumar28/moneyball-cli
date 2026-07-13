//! Render the brief or welcome view via TestBackend to inspect the layout.
//! Usage:
//!   cargo run --example render_views -p moneyball-tui -- welcome
//!   cargo run --example render_views -p moneyball-tui -- setup [step] [substep]

use moneyball_core::AppConfig;
use moneyball_tui::{App, SetupState};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("welcome");

    let cfg = AppConfig::resolve_optional(
        Some("/Users/Kailash/D/moneyball-test/moneyball-data"),
        None,
    );
    let mut app = App::new_for_test(cfg);

    if mode == "welcome" {
        // Synthesize a configured workspace so the welcome screen renders.
        app.force_welcome_for_test();
        app.force_workspace_for_test(vec![
            ("Namma Mane".into(), "2087011578504572".into()),
            ("Valmark CityVille".into(), "852565919728055".into()),
            ("Purva Sparkling Springs".into(), "1043714050577651".into()),
            ("Primus by Fincity".into(), "405885579167395".into()),
        ]);
    } else if mode == "setup" {
        let step: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let substep: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut s = SetupState::new(PathBuf::from("/Users/Kailash/D/moneyball-test/moneyball-data"));
        s.workspace_path = "/Users/Kailash/D/moneyball-test/moneyball-data".into();
        s.step = step;
        s.meta_substep = substep;
        if step == 1 && substep >= 1 {
            use moneyball_core::AdAccount;
            s.meta_discovered = (1..=8).map(|i| AdAccount {
                id: format!("act_{:016}", 1_000_000_000_000_000i64 + i as i64 * 1_000_000_000),
                name: match i {
                    1 => "Lake and Bloom - Prepaid".into(),
                    2 => "Fincity Official".into(),
                    3 => "Fincity Akash".into(),
                    _ => format!("Account #{}", i),
                },
                account_status: Some(if i == 4 { 2 } else { 1 }),
            }).collect();
            s.meta_selections = vec![false; s.meta_discovered.len()];
            s.meta_highlight = 0;
            s.meta_scroll = 0;
        }
        app.force_setup_for_test(s);
    }

    print!("{}", app.render_to_string(96, 30));
}