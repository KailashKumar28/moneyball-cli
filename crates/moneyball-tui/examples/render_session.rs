//! Render the chat with logo + welcome + sample exchange to verify the
//! session-start look. Run: cargo run --example render_session -p moneyball-tui

use moneyball_core::AppConfig;
use moneyball_tui::App;

fn main() {
    let cfg =
        AppConfig::resolve_optional(Some("/Users/Kailash/D/moneyball-test/moneyball-data"), None);
    let mut app = App::new_for_test(cfg);
    // App::new already seeded the logo and a welcome system message.
    // Force workspace so we render the chat view (not setup).
    app.force_workspace_for_test(vec![
        ("Namma Mane".into(), "2087011578504572".into()),
        ("Valmark CityVille".into(), "852565919728055".into()),
        (
            "Purva Sparkling Spring by Fincity".into(),
            "1043714050577651".into(),
        ),
        ("Primus by Fincity".into(), "405885579167395".into()),
    ]);
    app.force_welcome_for_test();
    // Synthesize a session id so the harness behaves like a real first-run.
    app.session_id = Some("mb-20260714T133000Z-abc1".into());
    app.session_started = Some(chrono::Utc::now());

    print!("{}", app.render_to_string(96, 32));
}
