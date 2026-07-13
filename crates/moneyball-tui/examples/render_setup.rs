//! Render the setup wizard at each step via TestBackend.
//! Run: cargo run --example render_setup -p moneyball-tui -- <step>

use moneyball_core::AppConfig;
use moneyball_tui::{App, SetupState};
use std::path::PathBuf;

fn main() {
    let step: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    let cfg = AppConfig::resolve_optional(
        Some("/Users/Kailash/D/moneyball-test/moneyball-data"),
        None,
    );
    let mut app = App::new_for_test(cfg);
    let mut s = SetupState::new(PathBuf::from("/Users/Kailash/D/moneyball-test/moneyball-data"));
    s.step = step;
    s.workspace_path = "/Users/Kailash/D/moneyball-test/moneyball-data".into();
    if step >= 1 {
        s.products = vec![
            ("Namma Mane".into(), "2087011578504572".into()),
            ("Valmark CityVille".into(), "852565919728055".into()),
        ];
        s.product_input = if step == 1 { "demo".into() } else { "".into() };
    }
    if step >= 2 {
        s.goals_input = "".into();
    }
    if step >= 3 {
        s.target_rpq_input = "2500".into();
    }
    app.force_setup_for_test(s);
    print!("{}", app.render_to_string(80, 22));
}