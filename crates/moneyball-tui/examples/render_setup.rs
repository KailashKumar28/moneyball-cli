//! Render the setup wizard at each step via TestBackend.
//! Usage:
//!   cargo run --example render_setup -p moneyball-tui -- <step> [substep]

use moneyball_core::meta::AdAccount;
use moneyball_core::AppConfig;
use moneyball_tui::{App, SetupState};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let step: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let substep: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    let cfg = AppConfig::resolve_optional(
        Some("/Users/Kailash/D/moneyball-test/moneyball-data"),
        None,
    );
    let mut app = App::new_for_test(cfg);
    let mut s = SetupState::new(PathBuf::from("/Users/Kailash/D/moneyball-test/moneyball-data"));
    s.workspace_path = "/Users/Kailash/D/moneyball-test/moneyball-data".into();
    s.step = step;
    s.meta_substep = substep;

    // Pre-populate state based on step.
    if step == 1 && substep >= 1 {
        // Generate 26 dummy ad accounts (matching the user's real discovery).
        s.meta_discovered = (1..=26).map(|i| AdAccount {
            id: format!("act_{:016}", 10000000000000000i64 + (i as i64) * 100000000),
            name: match i {
                1 => "Lake and Bloom - Prepaid".into(),
                2 => "Fincity Official".into(),
                3 => "Fincity Akash".into(),
                4 => "New Account 3".into(),
                5 => "Akash Gupta".into(),
                6 => "Purva - Prepaid".into(),
                7 => "Fincity FB AdRoll".into(),
                n if n <= 18 => format!("Fincity Account #{}", n),
                n => format!("Old Account {}", n),
            },
            account_status: Some(if i == 7 || i == 17 { 2 } else { 1 }),
        }).collect();
        s.meta_selections = vec![false; s.meta_discovered.len()];
        s.meta_highlight = 0;
        s.meta_scroll = 0;
        if substep >= 2 {
            // Pre-confirm selection so the rename view shows what the user picked.
            let picks: Vec<usize> = vec![18, 19, 20, 23, 25];
            for (i, sel) in s.meta_selections.iter_mut().enumerate() {
                *sel = picks.contains(&i);
            }
            s.meta_highlight = 25;
            s.meta_scroll = 14;
            s.meta_selected = picks;
            s.meta_rename_input = "1=Namma Mane 2=Valmark 3=Purva 4=Primus".into();
        }
    }
    if step >= 2 {
        s.products = vec![
            ("Namma Mane".into(), "2087011578504572".into()),
            ("Valmark CityVille".into(), "852565919728055".into()),
            ("Purva Springles".into(), "1043714050577651".into()),
            ("Primus by Fincity".into(), "405885579167395".into()),
        ];
    }
    if step >= 3 {
        s.goals_input = "".into();
    }
    if step >= 4 {
        // step 4 (target) removed; now only 4 steps (workspace, meta, products, goals)
    }
    app.force_setup_for_test(s);
    print!("{}", app.render_to_string(90, 26));
}