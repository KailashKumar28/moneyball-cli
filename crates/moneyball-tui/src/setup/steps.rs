//! Setup steps: enter on each step - the workspace path, the products,
//! the goals - and the save that ends the wizard. Split from
//! setup/mod.rs (size cap).

use super::*;

/// Built-in Fincity example products. Loaded when the user types `demo` in step 2.
const DEMO_PRODUCTS: &[(&str, &str)] = &[
    ("Namma Mane", "2087011578504572"),
    ("Valmark CityVille", "852565919728055"),
    ("Purva Sparkling Springs", "1043714050577651"),
    ("Primus by Fincity", "405885579167395"),
];

pub(super) fn advance_setup(app: &mut App, s: &mut SetupState) {
    s.error = None;
    match s.step {
        0 => advance_workspace(app, s),
        1 => advance_meta(app, s),
        2 => advance_products(s),
        3 => {
            // Goals is now step 3; advance goes to LLM (step 4), not save.
            s.step = 4;
            s.llm_substep = 0;
            s.llm_highlight = 0;
            s.llm_scroll = 0;
        }
        4 => advance_llm(app, s),
        _ => {}
    }
}

fn advance_workspace(app: &mut App, s: &mut SetupState) {
    let p = PathBuf::from(s.workspace_path.trim());
    if !p.is_dir() {
        match std::fs::create_dir_all(&p) {
            Ok(()) => {}
            Err(e) => {
                s.error = Some(format!("can't create {}: {}", p.display(), e));
                return;
            }
        }
    }
    std::fs::create_dir_all(p.join(moneyball_core::config::DOT_DIR)).ok();
    app.cfg.data_root = p;
    s.step = 1;
}

fn advance_products(s: &mut SetupState) {
    let raw = s.product_input.trim();
    if raw.is_empty() {
        if s.products.is_empty() {
            s.error = Some("add at least one product (try 'demo' to load Fincity example)".into());
            return;
        }
        s.step = 3;
        return;
    }
    if raw.eq_ignore_ascii_case("demo") {
        s.products = DEMO_PRODUCTS
            .iter()
            .map(|(n, a)| (n.to_string(), a.to_string()))
            .collect();
        s.product_input.clear();
        return;
    }
    let parts: Vec<&str> = raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        [name, acct] => {
            if !acct.chars().all(|c| c.is_ascii_digit()) || acct.len() < 6 {
                s.error = Some(format!(
                    "ad account '{}' should be digits only (15-20 chars)",
                    acct
                ));
                return;
            }
            if s.products.iter().any(|(n, _)| n == name) {
                s.error = Some(format!("product '{}' already added", name));
                return;
            }
            s.products.push((name.to_string(), acct.to_string()));
            s.product_input.clear();
        }
        _ => {
            s.error = Some(format!("expected 'Name AdAccount' - got '{}'", raw));
        }
    }
}

/// Parse the goals step. Returns false if validation fails; sets s.error.
fn advance_goals(s: &mut SetupState) -> bool {
    let raw = s.goals_input.trim();
    if raw.is_empty() {
        // Blank input -> defaults of 10 for every product.
        s.goals_input = s
            .products
            .iter()
            .map(|(n, _)| format!("{}=10", n))
            .collect::<Vec<_>>()
            .join(" ");
    }
    match parse_goals(&s.products, &s.goals_input) {
        Ok(_) => true,
        Err(e) => {
            s.error = Some(e);
            false
        }
    }
}

pub(super) fn advance_save(app: &mut App, s: &mut SetupState) {
    if !advance_goals(s) {
        return;
    } // validation failed - keep user on goals step
    let products: Vec<_> = s
        .products
        .iter()
        .map(|(n, a)| moneyball_core::config::Product {
            name: n.clone(),
            ad_account: a.clone(),
        })
        .collect();
    let goals_map = parse_goals(&s.products, &s.goals_input).unwrap_or_default();
    let providers = llm_providers_map(s);
    // target_rs_per_q is intentionally NOT asked during setup - it's a
    // derived/observed metric per product, not a hardcoded universal value;
    // the advisor derives it from observed performance.
    // Re-running /setup must never wipe state the wizard doesn't ask
    // about: config::update starts from the config on disk, so the minted
    // workspace_id, the brand and the CRM config survive.
    let saved = moneyball_core::config::update(&app.cfg.data_root, |w| {
        w.products = products;
        w.goals = goals_map;
        w.model_provider = Some(s.llm_provider_id.clone());
        w.model = Some(s.llm_model.clone());
        w.model_providers = providers;
    });
    let cfg = match saved {
        Ok(cfg) => cfg,
        Err(e) => {
            s.error = Some(format!("save failed: {}", e));
            return;
        }
    };
    // If user skipped Meta, scrub any stale token from auth.json.
    if !s.meta_connected {
        let _ = moneyball_core::secrets::clear_meta_token();
    }
    app.cfg.workspace = Some(cfg);
    // The startup chat log said "no workspace yet - run /setup"; that's now
    // stale. Tell the user setup landed and what to do next - a CRM too,
    // which setup itself does not connect.
    let crm = match moneyball_core::crm::fetch::spec_path(&app.cfg).is_file() {
        true => "",
        false => "\n  /connect crm   adds your CRM: qualified leads and every verdict come from it",
    };
    app.chat.push(chat::Cell::System(chat::cells::System(format!(
        "workspace configured. next:\n  /refresh       pulls Meta's numbers and writes yesterday's report{}\n  /schedule      runs the refresh every morning",
        crm
    ))));
    app.view = View::Brief;
    app.load_brief();
}

fn parse_goals(
    products: &[(String, String)],
    s: &str,
) -> std::result::Result<std::collections::HashMap<String, f64>, String> {
    let mut out = std::collections::HashMap::new();
    let known: std::collections::HashSet<&str> = products.iter().map(|(n, _)| n.as_str()).collect();

    // Smart parser: scan for the next '=' which separates a product name
    // from its number. Multi-word product names work because the name is
    // everything up to that '=' (trimmed). Separators between pairs can be
    // any combination of spaces and/or commas.
    let mut rest = s;
    while !rest.trim().is_empty() {
        // Skip leading whitespace/commas between pairs
        let trimmed = rest.trim_start();
        if trimmed.len() != rest.len() {
            rest = trimmed;
        }
        if rest.is_empty() {
            break;
        }

        // Find the '=' that ends this product's name.
        let eq = rest.find('=').ok_or_else(|| {
            let snippet: String = rest.chars().take(40).collect();
            format!(
                "expected 'ProdName=Number', no '=' found in: '{}...'",
                snippet
            )
        })?;

        // Name = chars from start to '=' (trim trailing whitespace).
        // eq/val_end come from find(), so split_at lands on char boundaries.
        let name = rest.split_at(eq).0.trim();
        if name.is_empty() {
            return Err("empty product name before '='".into());
        }
        if !known.contains(name) {
            return Err(format!(
                "unknown product '{}' (known: {:?})",
                name,
                products.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
            ));
        }

        // Number = chars after '=' until next whitespace/comma or end-of-input.
        let after_eq = rest.split_at(eq + 1).1;
        let val_end = after_eq
            .find(|c: char| c.is_whitespace() || c == ',')
            .unwrap_or(after_eq.len());
        let val = after_eq.split_at(val_end).0.trim();

        let v: f64 = val.parse().map_err(|_| {
            format!(
                "not a number: '{}' in '{}={}{}'",
                val,
                name,
                val,
                after_eq
                    .split_at(val_end)
                    .1
                    .chars()
                    .take(20)
                    .collect::<String>()
            )
        })?;
        if v <= 0.0 || v > 1000.0 {
            return Err(format!("goal {} out of range (1-1000) for '{}'", v, name));
        }
        out.insert(name.to_string(), v);

        // Advance past the number (and any whitespace/comma immediately after).
        rest = after_eq.split_at(val_end).1.trim_start();
    }

    // Fill in defaults for any missing products so partial input still saves.
    for (n, _) in products {
        out.entry(n.clone()).or_insert(10.0);
    }
    Ok(out)
}
