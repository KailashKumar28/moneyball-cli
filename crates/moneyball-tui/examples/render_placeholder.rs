//! Render the current TUI state via TestBackend so it can be inspected
//! without a real terminal. Run: cargo run --example render_placeholder -p moneyball-tui

use moneyball_core::AppConfig;

fn main() {
    // Read data-root from env or default to the Fincity dataset.
    let data_root = std::env::var("MONEYBALL_DATA_ROOT").ok();
    let cfg = AppConfig::resolve_optional(data_root.as_deref(), None);
    let mut app = moneyball_tui::App::new_for_test(cfg);
    app.load_brief();
    print!("{}", app.render_to_string(90, 28));
}
