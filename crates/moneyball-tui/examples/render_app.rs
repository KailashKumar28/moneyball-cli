//! Render the full App (chat view) via TestBackend to inspect the look.
//! Usage: cargo run --example render_app -p moneyball-tui -- [seed]

use moneyball_core::AppConfig;
use moneyball_tui::chat::cells;
use moneyball_tui::chat::Cell;
use moneyball_tui::App;

fn main() {
    let _arg = std::env::args().nth(1).unwrap_or_else(|| "seed".into());

    let cfg =
        AppConfig::resolve_optional(Some("/Users/Kailash/D/moneyball-test/moneyball-data"), None);
    let mut app = App::new_for_test(cfg);
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

    if true {
        app.chat.push(Cell::System(cells::System(
            "workspace configured. try /brief, /funnel <product>, or ask anything.".into(),
        )));
        app.chat.push(Cell::UserPrompt(cells::UserPrompt {
            text: "/brief".into(),
            at: chrono::Local::now(),
        }));
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "loading portfolio snapshot for 2026-07-13...".into(),
            streaming: false,
        }));
        app.chat.push_tool(
            "brief",
            "",
            vec![
                "BRIEF  (7d window)".into(),
                "".into(),
                "  > Namma Mane".into(),
                "    14,067/d  m 166  l 154  q 35  5.00/d  Rs.2,813".into(),
                "    L\u{2192}Q 22.7%   gap   5.0".into(),
                "  > Valmark CityVille".into(),
                "    4,838/d  m  27  l  24  q  2   0.29/d  Rs.16,934".into(),
                "    L\u{2192}Q  8.3%   gap   9.7".into(),
                "  > Purva Sparkling Spring by Fincity".into(),
                "    15,136/d  m  53  l  43  q  8  1.14/d  Rs.13,244".into(),
                "    L\u{2192}Q 18.6%   gap   8.9".into(),
                "  > Primus by Fincity".into(),
                "    3,165/d  m  14  l  13  q  1   0.14/d  Rs.22,152".into(),
                "    L\u{2192}Q  7.7%   gap   9.9".into(),
                "".into(),
                "FEASIBILITY  6.6 q/day @ Rs.37,206/day = Rs.5,662/q \u{00B7}  goal 40/day".into(),
                "  required @ current:  Rs.226,480/day (6.1x)".into(),
                "  required @ best Rs.2,813/q: Rs.112,520/day (3.0x)".into(),
                "  setup debt: 2 (geo_exclusions_present, higher_intent_form)".into(),
            ],
            true,
            87,
        );
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "portfolio is at 6.6 q/day against a 40/day goal. at current efficiency you'd need Rs.226k/day; at the best-observed Rs.2,813/q you still need Rs.112k/day. constraint is money, not channels - want me to propose a goal ramp?".into(),
            streaming: false,
        }));
    }

    print!("{}", app.render_to_string(96, 32));
}
