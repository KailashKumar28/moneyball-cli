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
        s.meta_discovered = vec![
            AdAccount {
                id: "act_2087011578504572".into(),
                name: "Namma Mane - Brand".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_852565919728055".into(),
                name: "Valmark CityVille".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_1043714050577651".into(),
                name: "Purva Sparkling Springs".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_405885579167395".into(),
                name: "Primus by Fincity".into(),
                account_status: Some(1),
            },
            AdAccount {
                id: "act_999999999999999".into(),
                name: "Old Test Account".into(),
                account_status: Some(2),
            },
        ];
        if substep >= 2 {
            s.meta_selected = vec![0, 1, 2, 3];
            s.meta_rename_input = "1=Namma Mane 2=Valmark CityVille 3=Purva Springles 4=Primus".into();
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
        s.target_rpq_input = "2500".into();
    }
    app.force_setup_for_test(s);
    print!("{}", app.render_to_string(90, 26));
}