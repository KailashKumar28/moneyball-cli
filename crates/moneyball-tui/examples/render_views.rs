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

    let cfg =
        AppConfig::resolve_optional(Some("/Users/Kailash/D/moneyball-test/moneyball-data"), None);
    let mut app = App::new_for_test(cfg);

    if mode == "welcome" {
        app.force_welcome_for_test();
        app.force_workspace_for_test(vec![
            ("Namma Mane".into(), "2087011578504572".into()),
            ("Valmark CityVille".into(), "852565919728055".into()),
            (
                "Purva Sparkling Spring by Fincity".into(),
                "1043714050577651".into(),
            ),
            ("Primus by Fincity".into(), "405885579167395".into()),
        ]);
    } else if mode == "setup" {
        let step: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let substep: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut s = SetupState::new(PathBuf::from(
            "/Users/Kailash/D/moneyball-test/moneyball-data",
        ));
        s.workspace_path = "/Users/Kailash/D/moneyball-test/moneyball-data".into();
        s.step = step;
        s.meta_substep = substep;
        if step == 1 && substep >= 1 {
            use moneyball_core::AdAccount;
            s.meta_discovered = (1..=26u64)
                .map(|i| {
                    let id = format!(
                        "act_{}",
                        match i {
                            15 => "669828955137726",
                            16 => "1174772473427623",
                            17 => "274770215671428",
                            18 => "508128451820487",
                            19 => "405885579167395",
                            20 => "852565919728055",
                            21 => "1075690630595010",
                            22 => "1097953782046130",
                            _ => "1000000000000000",
                        }
                    );
                    let name = match i {
                        15 => "Shriram Hebbal One".into(),
                        16 => "Evantha by Fincity- Prepaid".into(),
                        17 => "SLN by Fincity".into(),
                        18 => "Modlix".into(),
                        19 => "Primus by Fincity".into(),
                        20 => "Cityville by Fincity".into(),
                        21 => "Purva Sparkling (Raja Group) - Prepaid".into(),
                        22 => "DB Vishistha".into(),
                        _ => format!("Account #{}", i),
                    };
                    AdAccount {
                        id,
                        name,
                        account_status: Some(if i == 17 { 101 } else { 1 }),
                    }
                })
                .collect();
            s.meta_selections = vec![false; 26];
            // Mark indices 18 and 19 (zero-based) as selected (Primus, Cityville).
            s.meta_selections[18] = true;
            s.meta_selections[19] = true;
            // Pre-populate meta_selected for the rename substep (2).
            s.meta_selected = (0..2).collect();
            s.meta_highlight = 18;
            s.meta_scroll = 14;
        }
        s.meta_input = "EAA12345abcDefGhI".into();
        app.force_setup_for_test(s);
    }

    print!("{}", app.render_to_string(96, 30));
}
