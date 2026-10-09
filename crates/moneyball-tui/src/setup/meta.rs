//! Setup step 2, the Meta token: paste, validate, list the ad accounts it
//! can see, then pick products from them. `/setup meta` runs only the
//! paste: the new token replaces the stored one once it reads every
//! configured account, and products and settings stay as they are.
//! Split from setup/mod.rs (size cap).

use crate::*;

use moneyball_core::meta::account_id_for_storage;
use moneyball_core::{list_ad_accounts, validate_token, AdAccount};

use super::SetupState;

/// The box shows a saved token as bullets; they are never a token.
fn is_saved_placeholder(input: &str) -> bool {
    !input.is_empty() && input.chars().all(|c| c == '\u{2022}')
}

pub(super) fn meta_insert(s: &mut SetupState, c: char) {
    match s.meta_substep {
        // Bullets stand for the saved token: typing replaces them.
        0 => {
            if is_saved_placeholder(&s.meta_input) {
                s.meta_input.clear();
            }
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

pub(super) fn meta_backspace(s: &mut SetupState) {
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

pub(super) fn advance_meta(app: &mut App, s: &mut SetupState) {
    match s.meta_substep {
        // Substep 0: paste token. Mandatory.
        0 => {
            // Bullets left in the box are the saved token, kept as is.
            let saved = moneyball_core::secrets::load_meta_token();
            let raw = match &saved {
                Some(t) if is_saved_placeholder(&s.meta_input) => t.clone(),
                _ => s.meta_input.trim().to_string(),
            };
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
            if let Err(e) = validate_token(&raw) {
                s.error = Some(format!("token rejected: {}", e));
                return;
            }
            let accounts = match list_ad_accounts(&raw) {
                Ok(a) if a.is_empty() => {
                    s.error = Some("token is valid but no ad accounts found (need ads_read + an ad account assigned to you)".into());
                    return;
                }
                Ok(a) => a,
                Err(e) => {
                    s.error = Some(format!("couldn't list ad accounts: {}", e));
                    return;
                }
            };
            if s.token_only {
                return replace_token(app, s, &raw, &accounts);
            }
            if let Err(e) = store_verified(&raw) {
                s.error = Some(e);
                return;
            }
            // Capture token length for the collapsed summary ("••••• (N chars)")
            // BEFORE clearing the buffer.
            s.meta_token_len = raw.chars().count();
            s.meta_discovered = accounts;
            s.meta_selections = vec![false; s.meta_discovered.len()];
            s.meta_highlight = 0;
            s.meta_scroll = 0;
            s.meta_input.clear();
            s.meta_substep = 1;
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

/// `/setup meta`: replace the stored token only when the new one reads
/// every configured product's account - a verified replacement, never a
/// token that silently loses a client.
fn replace_token(app: &mut App, s: &mut SetupState, token: &str, accounts: &[AdAccount]) {
    let visible: Vec<String> = accounts
        .iter()
        .map(|a| account_id_for_storage(&a.id))
        .collect();
    let blind: Vec<String> = s
        .products
        .iter()
        .filter(|(_, acct)| !visible.contains(acct))
        .map(|(name, acct)| format!("{} (act_{})", name, acct))
        .collect();
    if !blind.is_empty() {
        s.error = Some(format!(
            "this token can't read {}: the saved one is kept. Paste a token that can.",
            blind.join(", ")
        ));
        return;
    }
    let same = moneyball_core::secrets::load_meta_token().as_deref() == Some(token);
    if !same {
        if let Err(e) = store_verified(token) {
            s.error = Some(e);
            return;
        }
    }
    s.meta_input.clear();
    app.view = View::Brief;
    app.chat.note(format!(
        "Meta token {}: it reads all {} products. /refresh pulls with it.",
        if same { "checked" } else { "replaced" },
        s.products.len()
    ));
}

/// Store the token and read it back: a keychain-era write could report
/// success and not land, and a token that did not land is no token.
fn store_verified(token: &str) -> Result<(), String> {
    moneyball_core::secrets::store_meta_token(token)
        .map_err(|e| format!("token accepted but could not be saved: {}", e))?;
    if moneyball_core::secrets::load_meta_token().as_deref() != Some(token) {
        let _ = moneyball_core::secrets::clear_meta_token();
        return Err("the token did not persist - check ~/.moneyball/auth.json is writable".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullets_are_the_saved_token_never_a_token() {
        assert!(is_saved_placeholder("\u{2022}\u{2022}\u{2022}"));
        assert!(!is_saved_placeholder(""));
        assert!(!is_saved_placeholder("EAAB\u{2022}"));
    }

    #[test]
    fn a_token_blind_to_a_configured_account_is_refused_before_saving() {
        let cfg = moneyball_core::AppConfig {
            data_root: std::env::temp_dir().join("mb-setup-meta-none"),
            date: None,
            workspace: None,
            agent: true,
        };
        let mut app = App::new_for_test(cfg);
        let mut s = SetupState::new(app.cfg.data_root.clone());
        s.token_only = true;
        s.products = vec![
            ("Primus".into(), "405885579167395".into()),
            ("Disha".into(), "2087011578504572".into()),
        ];
        let sees_one = vec![AdAccount {
            id: "act_2087011578504572".into(),
            name: "Disha".into(),
            account_status: Some(1),
        }];
        replace_token(&mut app, &mut s, "NEW-TOKEN", &sees_one);
        let err = s.error.clone().unwrap_or_default();
        assert!(
            err.contains("Primus (act_405885579167395)") && err.contains("saved one is kept"),
            "{}",
            err
        );
        assert!(
            app.view != View::Brief,
            "the wizard stays open on a refusal"
        );
    }
}
