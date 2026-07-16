//! First-run setup wizard: state machine, key handling, advance logic,
//! and per-step renders. Owned end-to-end by this module.

use crate::*;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use moneyball_core::provider::{built_in_presets, models_for, ModelProviderInfo};
use moneyball_core::{list_ad_accounts, validate_token, AdAccount, WorkspaceConfig};
use std::path::Path;

// Two chrono types collide on import name; alias the plain Utc.

#[derive(Debug, Clone, PartialEq)]
pub struct SetupState {
    pub step: usize,
    pub workspace_path: String,
    /// Step 1 (Meta connect): which substep we're on.
    pub meta_substep: u8,
    /// Step 1 buffer: token paste / "skip".
    pub meta_input: String,
    /// Step 1: discovered ad accounts after token validation.
    pub meta_discovered: Vec<AdAccount>,
    /// Step 1 substep 1: per-account checkbox state.
    pub meta_selections: Vec<bool>,
    /// Step 1 substep 1: cursor row (0-based into meta_discovered).
    pub meta_highlight: usize,
    /// Step 1 substep 1: first visible row (scroll).
    pub meta_scroll: usize,
    /// Step 1 substep 2: rename overrides, format `1=Name 2=OtherName`.
    pub meta_rename_input: String,
    /// Step 1: final selection (Vec of indices into meta_discovered).
    pub meta_selected: Vec<usize>,
    /// Step 1: whether the user pasted a valid token. Token is mandatory now -
    /// we always validate, never skip.
    pub meta_connected: bool,
    /// Length of the validated token in characters. Captured before
    /// `meta_input` is cleared so the collapsed summary can show "••••• (N chars)".
    pub meta_token_len: usize,
    /// Step 2 entry buffer: "Name AdAccount" (space- or comma-separated).
    pub product_input: String,
    pub products: Vec<(String, String)>, // (name, ad_account)
    /// Step 3 entry buffer: "Prod1=10 Prod2=12".
    pub goals_input: String,
    /// Step 4 (LLM provider) substep: 0=provider pick, 1=API key paste,
    /// 2=model pick, 3=custom URL entry.
    pub llm_substep: u8,
    /// Step 4: selected provider id ("openai", "minimax", "custom", ...).
    pub llm_provider_id: String,
    /// Step 4: API key paste buffer (raw, before keychain write).
    pub llm_input: String,
    /// Step 4: length of captured API key for collapsed-summary bullets.
    pub llm_key_len: usize,
    /// Step 4: selected model slug (e.g. "gpt-5", "MiniMax-M3").
    pub llm_model: String,
    /// Step 4: custom base URL (only when provider id is "custom").
    pub llm_url: String,
    /// Step 4 picker: cursor row in the provider/model list.
    pub llm_highlight: usize,
    /// Step 4 picker: first visible row (scroll).
    pub llm_scroll: usize,
    /// Resolved provider entry for the selected id. Built as the user
    /// advances; persisted on save.
    pub llm_provider: Option<ModelProviderInfo>,
    pub error: Option<String>,
}

impl SetupState {
    pub fn new(default: PathBuf) -> Self {
        Self {
            step: 0,
            workspace_path: default.display().to_string(),
            meta_substep: 0,
            meta_input: String::new(),
            meta_discovered: Vec::new(),
            meta_selections: Vec::new(),
            meta_highlight: 0,
            meta_scroll: 0,
            meta_rename_input: String::new(),
            meta_selected: Vec::new(),
            meta_connected: false,
            meta_token_len: 0,
            product_input: String::new(),
            products: Vec::new(),
            goals_input: String::new(),
            llm_substep: 0,
            llm_provider_id: String::new(),
            llm_input: String::new(),
            llm_key_len: 0,
            llm_model: String::new(),
            llm_url: String::new(),
            llm_highlight: 0,
            llm_scroll: 0,
            llm_provider: None,
            error: None,
        }
    }

    /// Build a SetupState that mirrors an existing WorkspaceConfig so
    /// /setup re-runs as an "edit current settings" flow instead of a
    /// blank-slate wizard. The user can hit Enter through each step
    /// to keep the current values, or change one step and re-Enter.
    ///
    /// What we can restore from config:
    ///   - workspace path (data_root)
    ///   - products (name + ad_account)
    ///   - goals_input (re-serialized as "Prod1=10 Prod2=12")
    ///   - llm_provider_id, llm_model, llm_provider
    ///   - llm_key_len (read from keychain; 0 if not present)
    ///   - meta_connected (true if products were configured)
    ///
    /// What we cannot restore (user must redo if they want to change):
    ///   - meta_input (the Meta API token itself - not in memory or config)
    ///   - meta_discovered / meta_selections (require live API call)
    ///   - llm_input (the LLM API key - in keychain only)
    pub fn prefilled_from(cfg: &WorkspaceConfig, data_root: &Path) -> Self {
        let mut s = Self::new(data_root.to_path_buf());
        s.workspace_path = data_root.display().to_string();
        s.products = cfg
            .products
            .iter()
            .map(|p| (p.name.clone(), p.ad_account.clone()))
            .collect();
        // Serialize goals HashMap back to "Prod1=10 Prod2=12" format.
        s.goals_input = cfg
            .goals
            .iter()
            .map(|(name, n)| format!("{}={}", name, *n as u64))
            .collect::<Vec<_>>()
            .join(" ");
        // Meta: connected if products exist (meaning setup was completed
        // at some point). Token itself must be re-pasted.
        s.meta_connected = !cfg.products.is_empty();
        // LLM
        if let Some(provider) = &cfg.model_provider {
            s.llm_provider_id = provider.clone();
        }
        if let Some(model) = &cfg.model {
            s.llm_model = model.clone();
        }
        if let Some(info) = cfg.model_providers.get(s.llm_provider_id.as_str()) {
            s.llm_provider = Some(info.clone());
        }
        // Try the keychain so the collapsed summary shows bullet count.
        if !s.llm_provider_id.is_empty() {
            s.llm_key_len = moneyball_core::secrets::load_llm_key(&s.llm_provider_id)
                .map(|k| k.chars().count())
                .unwrap_or(0);
        }
        // If everything's configured, jump straight to the LLM step so
        // the user can fix a broken key without re-walking the wizard.
        // Otherwise start at the first unconfigured step.
        s.step = if !s.llm_provider_id.is_empty() && !s.llm_model.is_empty() {
            4
        } else if !s.products.is_empty() {
            3
        } else {
            0
        };
        s
    }
}

/// Built-in Fincity example products. Loaded when the user types `demo` in step 2.
const DEMO_PRODUCTS: &[(&str, &str)] = &[
    ("Namma Mane", "2087011578504572"),
    ("Valmark CityVille", "852565919728055"),
    ("Purva Sparkling Springs", "1043714050577651"),
    ("Primus by Fincity", "405885579167395"),
];

pub(crate) fn handle_setup_key(app: &mut App, mut state: SetupState, k: KeyEvent) {
    // Substep 1 of step 1 is a list-selection mode with its own keymap.
    if state.step == 1 && state.meta_substep == 1 {
        handle_select_keys(&mut state, k);
        app.view = View::Setup(state);
        return;
    }
    // Step 4 picker substeps (provider pick, model pick) have their own
    // arrow-key navigation. Other keys (Enter, Esc, Char) fall through
    // to the default handler so Enter advances to the next substep.
    if state.step == 4
        && (state.llm_substep == 0 || state.llm_substep == 2)
        && handle_llm_picker_keys(&mut state, k)
    {
        app.view = View::Setup(state);
        return;
    }
    // Substep 2 of step 1 is the rename buffer. Esc goes back to substep 1.
    if state.step == 1 && state.meta_substep == 2 && k.code == KeyCode::Esc {
        state.meta_substep = 1;
        state.meta_rename_input.clear();
        state.error = None;
        app.view = View::Setup(state);
        return;
    }
    // Char / Enter / Backspace fall through to the default handler below.
    match k.code {
        KeyCode::Esc => {
            // Esc: clear the active input if any, else leave the wizard
            // for the chat - the footer advertises "esc exit setup".
            let cleared = match state.step {
                0 if !state.workspace_path.is_empty() => {
                    state.workspace_path.clear();
                    true
                }
                1 => match state.meta_substep {
                    0 if !state.meta_input.is_empty() => {
                        state.meta_input.clear();
                        true
                    }
                    _ => false,
                },
                2 if !state.product_input.is_empty() => {
                    state.product_input.clear();
                    true
                }
                3 if !state.goals_input.is_empty() => {
                    state.goals_input.clear();
                    true
                }
                _ => false,
            };
            if cleared {
                state.error = None;
            } else {
                // Esc leaves the wizard for the chat (Claude Code modal
                // behavior) - saved steps are kept; /setup resumes with
                // current values prefilled.
                app.view = View::Brief;
                app.status = Some("setup closed - /setup reopens it".into());
                return;
            }
        }
        KeyCode::Enter => {
            advance_setup(app, &mut state);
        }
        KeyCode::Backspace => {
            backspace_setup(&mut state);
        }
        KeyCode::Char(c) => {
            insert_setup(&mut state, c);
        }
        _ => {}
    }
    // advance_save may have transitioned us out to View::Brief. Don't clobber that.
    if app.view != View::Brief {
        app.view = View::Setup(state);
    }
}

/// Keyboard handler for the multi-account selection list (step 1 substep 1).
fn handle_select_keys(s: &mut SetupState, k: KeyEvent) {
    let n = s.meta_discovered.len();
    if n == 0 {
        return;
    }
    // Visible rows must match the renderer's visible_rows constant below.
    const VISIBLE_ROWS: usize = 12;
    match k.code {
        KeyCode::Up => {
            if s.meta_highlight > 0 {
                s.meta_highlight -= 1;
                if s.meta_highlight < s.meta_scroll {
                    s.meta_scroll = s.meta_highlight;
                }
            }
        }
        KeyCode::Down => {
            if s.meta_highlight + 1 < n {
                s.meta_highlight += 1;
                if s.meta_highlight >= s.meta_scroll + VISIBLE_ROWS {
                    s.meta_scroll = s.meta_highlight + 1 - VISIBLE_ROWS;
                }
            }
        }
        KeyCode::PageUp => {
            s.meta_highlight = s.meta_highlight.saturating_sub(VISIBLE_ROWS);
            s.meta_scroll = s.meta_scroll.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::PageDown => {
            s.meta_highlight = (s.meta_highlight + VISIBLE_ROWS).min(n - 1);
            s.meta_scroll = (s.meta_highlight + 1).saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Home => {
            s.meta_highlight = 0;
            s.meta_scroll = 0;
        }
        KeyCode::End => {
            s.meta_highlight = n - 1;
            s.meta_scroll = n.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Char(' ') => {
            s.meta_selections[s.meta_highlight] = !s.meta_selections[s.meta_highlight];
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let any = s.meta_selections.iter().any(|&b| b);
            for sel in s.meta_selections.iter_mut() {
                *sel = !any;
            }
        }
        KeyCode::Enter => {
            let chosen: Vec<usize> = (0..n).filter(|&i| s.meta_selections[i]).collect();
            if chosen.is_empty() {
                s.error = Some("select at least one account (Space to toggle, 'a' for all)".into());
                return;
            }
            s.meta_selected = chosen;
            s.meta_substep = 2;
            s.error = None;
        }
        KeyCode::Esc => {
            // "back" - return to substep 0 (token paste) WITHOUT nuking the
            // user's progress. The discovered account list and the per-row
            // checkbox state survive so the user doesn't have to re-select
            // after re-validating. The token itself was already cleared
            // from `meta_input` after validation, so we restore a masked
            // placeholder (N bullets of `meta_token_len`) so the input
            // box doesn't look empty.
            s.meta_input = "\u{2022}".repeat(s.meta_token_len);
            s.meta_rename_input.clear();
            s.meta_substep = 0;
            s.error = None;
        }
        _ => {}
    }
}

/// Arrow-key navigation for step 4 picker substeps (provider pick +
/// model pick). Step 4 substep 1 (paste) and substep 3 (custom URL/model)
/// accept free text via `insert_setup`, so this only handles the picker
/// substeps. Returns `true` if the key was consumed (arrow nav), so
/// the caller can fall through to the default handler for Enter / Esc /
/// Backspace / Char which need to reach advance_setup, etc.
fn handle_llm_picker_keys(s: &mut SetupState, k: KeyEvent) -> bool {
    // Visible rows in step 4 picker lists. Keep in sync with the renderer.
    const VISIBLE_ROWS: usize = 6;
    let total = match s.llm_substep {
        0 => built_in_presets().len() + 1, // presets + custom
        2 => {
            let preset = if s.llm_provider_id == "custom" {
                ModelProviderInfo {
                    name: "custom".into(),
                    base_url: s.llm_url.clone(),
                    ..Default::default()
                }
            } else {
                s.llm_provider
                    .clone()
                    .unwrap_or_else(ModelProviderInfo::openai)
            };
            models_for(&preset).len()
        }
        _ => 0,
    };
    if total == 0 {
        return false;
    }
    match k.code {
        KeyCode::Up => {
            if s.llm_highlight > 0 {
                s.llm_highlight -= 1;
                if s.llm_highlight < s.llm_scroll {
                    s.llm_scroll = s.llm_highlight;
                }
            }
            true
        }
        KeyCode::Down => {
            if s.llm_highlight + 1 < total {
                s.llm_highlight += 1;
                if s.llm_highlight >= s.llm_scroll + VISIBLE_ROWS {
                    s.llm_scroll = s.llm_highlight + 1 - VISIBLE_ROWS;
                }
            }
            true
        }
        KeyCode::Home => {
            s.llm_highlight = 0;
            s.llm_scroll = 0;
            true
        }
        KeyCode::End => {
            s.llm_highlight = total.saturating_sub(1);
            s.llm_scroll = total.saturating_sub(VISIBLE_ROWS);
            true
        }
        // Other keys (Enter, Esc, Backspace, Char) are NOT consumed -
        // the caller routes them to advance_setup / Esc / backspace_setup
        // / insert_setup. Enter is the critical one: without it, the
        // picker is dead because advance_setup never gets called.
        _ => false,
    }
}

fn insert_setup(s: &mut SetupState, c: char) {
    match s.step {
        0 => {
            s.workspace_path.push(c);
        }
        1 => {
            meta_insert(s, c);
        }
        2 => {
            s.product_input.push(c);
        }
        3 => {
            s.goals_input.push(c);
        }
        4 => {
            // Step 4 substeps that accept free-text: paste-key (1) and
            // custom URL/model (3). Picker substeps (0, 2) ignore chars.
            match s.llm_substep {
                1 => s.llm_input.push(c),
                3 => {
                    // First fill URL, then model. The first Enter advances
                    // to the model phase; we toggle which buffer is active
                    // by checking whether URL is set.
                    if s.llm_url.is_empty() {
                        s.llm_url.push(c);
                    } else {
                        s.llm_model.push(c);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn backspace_setup(s: &mut SetupState) {
    match s.step {
        0 => {
            s.workspace_path.pop();
        }
        1 => {
            meta_backspace(s);
        }
        2 => {
            s.product_input.pop();
        }
        3 => {
            s.goals_input.pop();
        }
        4 => match s.llm_substep {
            1 => {
                s.llm_input.pop();
            }
            3 => {
                if s.llm_model.is_empty() {
                    s.llm_url.pop();
                } else {
                    s.llm_model.pop();
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn meta_insert(s: &mut SetupState, c: char) {
    match s.meta_substep {
        0 => {
            s.meta_input.push(c);
        }
        // substep 1 is keyboard-driven (Up/Down/Space/'a'/Enter); ignore chars.
        1 => {}
        2 => {
            s.meta_rename_input.push(c);
        }
        _ => {}
    }
}

fn meta_backspace(s: &mut SetupState) {
    match s.meta_substep {
        0 => {
            s.meta_input.pop();
        }
        // substep 1 ignored.
        1 => {}
        2 => {
            s.meta_rename_input.pop();
        }
        _ => {}
    }
}

fn advance_setup(app: &mut App, s: &mut SetupState) {
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

fn advance_meta(_app: &mut App, s: &mut SetupState) {
    match s.meta_substep {
        // Substep 0: paste token. Mandatory.
        0 => {
            let raw = s.meta_input.trim();
            if raw.is_empty() {
                s.error =
                    Some("a Meta Marketing API access token is required (paste it above)".into());
                return;
            }
            // Old wizards accepted the literal word "skip" here. Catch it
            // before it goes to Meta as a (masked) token and fails cryptically.
            if raw.eq_ignore_ascii_case("skip") {
                s.error = Some("'skip' is no longer supported - moneyball now requires a Meta token to discover your ad accounts".into());
                return;
            }
            // Validate token + list ad accounts.
            if let Err(e) = validate_token(raw) {
                s.error = Some(format!("token rejected: {}", e));
                return;
            }
            match list_ad_accounts(raw) {
                Ok(accounts) => {
                    if accounts.is_empty() {
                        s.error = Some("token is valid but no ad accounts found (need ads_read + an ad account assigned to you)".into());
                        return;
                    }
                    // Persist token to keychain immediately; we'll move it out of memory after.
                    if let Err(e) = moneyball_core::secrets::store_meta_token(raw) {
                        s.error = Some(format!(
                            "token accepted but keychain write failed: {}. \
On macOS, allow moneyball-cli in Keychain Access (or run /setup again after granting).",
                            e
                        ));
                        return;
                    }
                    // Round-trip verify: macOS Keychain ACLs (or an ad-hoc
                    // binary signature) can make set_password report success
                    // while the item never lands in the user keychain. Reject
                    // early so the user knows to retry.
                    if moneyball_core::secrets::load_meta_token().as_deref() != Some(raw) {
                        s.error = Some(
                            "keychain write did not persist (likely macOS denied access). \
On macOS, re-run /setup and approve the Keychain prompt, or sign the moneyball binary."
                                .into(),
                        );
                        let _ = moneyball_core::secrets::clear_meta_token();
                        return;
                    }
                    // Capture token length for the collapsed summary ("••••• (N chars)")
                    // BEFORE clearing the buffer.
                    s.meta_token_len = s.meta_input.chars().count();
                    s.meta_discovered = accounts;
                    s.meta_selections = vec![false; s.meta_discovered.len()];
                    s.meta_highlight = 0;
                    s.meta_scroll = 0;
                    s.meta_input.clear();
                    s.meta_substep = 1;
                }
                Err(e) => {
                    s.error = Some(format!("couldn't list ad accounts: {}", e));
                }
            }
        }
        // Substep 1: multi-select list. Enter handler lives in handle_select_keys.
        // (advance_setup is called for Enter; substep 1's Enter is handled there.)
        1 => {
            // Shouldn't usually hit this path (Enter is routed via handle_select_keys).
            // Fall through: confirm whatever is currently selected.
            let chosen: Vec<usize> = (0..s.meta_discovered.len())
                .filter(|&i| s.meta_selections[i])
                .collect();
            if chosen.is_empty() {
                s.error = Some("select at least one account (Space to toggle, 'a' for all)".into());
                return;
            }
            s.meta_selected = chosen;
            s.meta_substep = 2;
        }
        // Substep 2: rename overrides (or blank = use account names).
        2 => {
            let raw = s.meta_rename_input.trim();
            // Build overrides from input.
            let overrides = parse_renames(raw);
            if let Err(e) = overrides {
                s.error = Some(e);
                return;
            }
            let overrides = overrides.unwrap_or_default();
            // Build final products list.
            let mut new_products: Vec<(String, String)> = Vec::new();
            for (i, &idx) in s.meta_selected.iter().enumerate() {
                let acct = &s.meta_discovered[idx];
                // Default to the Meta account's display name; let the user
                // override via "1=Name 2=OtherName" syntax in the rename input.
                let name = overrides
                    .get(&(idx + 1))
                    .cloned()
                    .unwrap_or_else(|| acct.name.clone());
                let id = moneyball_core::meta::account_id_for_storage(&acct.id);
                if new_products.iter().any(|(n, _)| n == &name) {
                    s.error = Some(format!("duplicate product name '{}'", name));
                    return;
                }
                new_products.push((name, id));
                let _ = i;
            }
            // If user explicitly typed 'all' and used demo, skip auto-fills.
            s.products = new_products;
            s.meta_connected = true;
            s.error = None;
            s.meta_rename_input.clear();
            s.step = 3; // skip the manual "add products" step; go to goals.
        }
        _ => {}
    }
}

fn parse_renames(
    raw: &str,
) -> std::result::Result<std::collections::HashMap<usize, String>, String> {
    let mut out = std::collections::HashMap::new();
    for part in raw.split_whitespace() {
        let (idx_s, name) = part
            .split_once('=')
            .ok_or_else(|| format!("bad rename '{}': expected N=Name", part))?;
        let idx: usize = idx_s
            .parse()
            .map_err(|_| format!("bad index '{}'", idx_s))?;
        if idx < 1 {
            return Err(format!("index must be >= 1 (got {})", idx));
        }
        if name.is_empty() {
            return Err(format!("empty name at index {}", idx));
        }
        out.insert(idx, name.to_string());
    }
    Ok(out)
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

fn advance_save(app: &mut App, s: &mut SetupState) {
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
    // target_rs_per_q is intentionally NOT asked during setup - it's a
    // derived/observed metric per product, not a hardcoded universal value.
    // Stored as None; the advisor derives it from observed performance.
    let cfg = WorkspaceConfig {
        products,
        goals: goals_map,
        target_rs_per_q: None,
        crm: Default::default(),
        model_provider: s.llm_provider_id.clone().into(),
        model: s.llm_model.clone().into(),
        model_providers: llm_providers_map(s),
    };
    if let Err(e) = cfg.save(&app.cfg.data_root) {
        s.error = Some(format!("save failed: {}", e));
        return;
    }
    // If user skipped Meta, scrub any stale token from keychain.
    if !s.meta_connected {
        let _ = moneyball_core::secrets::clear_meta_token();
    }
    app.cfg.workspace = Some(cfg);
    // The startup chat log said "no workspace yet - run /setup"; that's now
    // stale. Tell the user setup landed and what to do next.
    app.chat.push(chat::Cell::System(chat::cells::System(
        "workspace configured. try /brief, /funnel <product>, /ask or anything you want.".into(),
    )));
    app.view = View::Brief;
    app.load_brief();
}

/// Build the `model_providers` HashMap to persist. Includes the active
/// provider entry plus any built-in presets that the user might want
/// later when switching providers from /settings.
fn llm_providers_map(s: &SetupState) -> std::collections::HashMap<String, ModelProviderInfo> {
    let mut map = std::collections::HashMap::new();
    for (id, p) in built_in_presets() {
        map.insert(id.to_string(), p);
    }
    if let Some(active) = &s.llm_provider {
        map.insert(s.llm_provider_id.clone(), active.clone());
    }
    map
}

/// Step 4 (LLM) advance logic. Dispatches on `llm_substep`:
///   0 -> provider pick (Enter on highlighted row)
///   1 -> API key paste -> writes to keychain, advances to model pick
///   2 -> model pick -> persists + transitions to advance_save
///   3 -> custom URL/model entry -> save
fn advance_llm(app: &mut App, s: &mut SetupState) {
    match s.llm_substep {
        0 => {
            // Pick the highlighted preset or "custom".
            let presets = built_in_presets();
            let total = presets.len() + 1;
            if s.llm_highlight >= total {
                s.error = Some("invalid provider selection".into());
                return;
            }
            if s.llm_highlight < presets.len() {
                let (id, provider) = &presets[s.llm_highlight];
                s.llm_provider_id = (*id).to_string();
                s.llm_provider = Some(provider.clone());
            } else {
                s.llm_provider_id = "custom".into();
                s.llm_provider = Some(ModelProviderInfo {
                    name: "custom".into(),
                    base_url: String::new(),
                    ..Default::default()
                });
            }
            s.llm_substep = 1;
            s.error = None;
        }
        1 => {
            // Validate that a key was pasted; write to keychain.
            let key = s.llm_input.trim();
            if key.is_empty() {
                s.error = Some("API key is required to continue".into());
                return;
            }
            if let Err(e) = moneyball_core::secrets::store_llm_key(&s.llm_provider_id, key) {
                s.error = Some(format!(
                    "keychain write failed: {}. \
On macOS, allow moneyball-cli in Keychain Access (or run /setup again after granting).",
                    e
                ));
                return;
            }
            // Round-trip verify: macOS Keychain ACLs (or an ad-hoc binary
            // signature) can make set_password report success while the
            // item never lands in the user keychain. Reject early so the
            // user knows to retry.
            if moneyball_core::secrets::load_llm_key(&s.llm_provider_id).as_deref() != Some(key) {
                s.error = Some(
                    "keychain write did not persist (likely macOS denied access). \
On macOS, re-run /setup and approve the Keychain prompt, or sign the moneyball binary."
                        .into(),
                );
                let _ = moneyball_core::secrets::clear_llm_key(&s.llm_provider_id);
                return;
            }
            s.llm_key_len = key.chars().count();
            s.llm_input.clear();
            // For "custom", ask for the base URL next (reuse substep 1 input).
            if s.llm_provider_id == "custom" {
                s.llm_substep = 3;
            } else {
                s.llm_substep = 2;
                s.llm_highlight = 0;
            }
            s.error = None;
        }
        2 => {
            // Pick the highlighted model.
            let preset = if s.llm_provider_id == "custom" {
                ModelProviderInfo {
                    name: "custom".into(),
                    base_url: s.llm_url.clone(),
                    ..Default::default()
                }
            } else {
                s.llm_provider
                    .clone()
                    .unwrap_or_else(ModelProviderInfo::openai)
            };
            let models = models_for(&preset);
            if s.llm_highlight >= models.len() {
                s.error = Some("invalid model selection".into());
                return;
            }
            s.llm_model = models[s.llm_highlight].to_string();
            // If user picked the "custom" sentinel, fall through to a free-text
            // URL substep instead of saving here.
            if s.llm_model == "custom" && s.llm_provider_id == "custom" {
                s.llm_model.clear();
                s.llm_substep = 3;
                return;
            }
            advance_save(app, s);
        }
        3 => {
            // Custom substep: collects base URL + (optionally) model slug.
            if s.llm_provider_id == "custom" && s.llm_url.trim().is_empty() {
                s.error = Some("enter the provider's base URL".into());
                return;
            }
            if s.llm_model.is_empty() {
                s.error = Some("enter the model slug".into());
                return;
            }
            // Refresh the provider entry with the URL the user typed.
            if let Some(p) = s.llm_provider.as_mut() {
                p.base_url = s.llm_url.trim().to_string();
            }
            advance_save(app, s);
        }
        _ => {}
    }
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
        let name = rest[..eq].trim();
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
        let after_eq = &rest[eq + 1..];
        let val_end = after_eq
            .find(|c: char| c.is_whitespace() || c == ',')
            .unwrap_or(after_eq.len());
        let val = after_eq[..val_end].trim();

        let v: f64 = val.parse().map_err(|_| {
            format!(
                "not a number: '{}' in '{}={}{}'",
                val,
                name,
                val,
                after_eq[val_end..].chars().take(20).collect::<String>()
            )
        })?;
        if v <= 0.0 || v > 1000.0 {
            return Err(format!("goal {} out of range (1-1000) for '{}'", v, name));
        }
        out.insert(name.to_string(), v);

        // Advance past the number (and any whitespace/comma immediately after).
        rest = after_eq[val_end..].trim_start();
    }

    // Fill in defaults for any missing products so partial input still saves.
    for (n, _) in products {
        out.entry(n.clone()).or_insert(10.0);
    }
    Ok(out)
}

// ---------- render ----------

mod render_steps;
pub(crate) use render_steps::render_setup;
