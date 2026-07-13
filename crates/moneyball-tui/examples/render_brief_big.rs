use moneyball_core::AppConfig;
fn main() {
    let cfg = AppConfig::resolve_optional(Some("/Users/Kailash/DEV/MOD_AI/fin_campaign_analysis"), None);
    let mut app = moneyball_tui::App::new_for_test(cfg);
    app.load_brief();
    print!("{}", app.render_to_string(110, 36));
}
