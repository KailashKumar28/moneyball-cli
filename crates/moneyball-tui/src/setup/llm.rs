//! Setup step 5, the model: pick a provider, paste its key, pick a model,
//! or for a custom provider type its base URL, then its model. Split from
//! setup/mod.rs (size cap).

use super::*;

/// Arrow-key navigation for step 4 picker substeps (provider pick +
/// model pick). Step 4 substep 1 (paste) and substep 3 (custom URL/model)
/// accept free text via `insert_setup`, so this only handles the picker
/// substeps. Returns `true` if the key was consumed (arrow nav), so
/// the caller can fall through to the default handler for Enter / Esc /
/// Backspace / Char which need to reach advance_setup, etc.
pub(super) fn handle_llm_picker_keys(s: &mut SetupState, k: KeyEvent) -> bool {
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

/// Build the `model_providers` HashMap to persist. Includes the active
/// provider entry plus any built-in presets that the user might want
/// later when switching providers from /settings.
pub(super) fn llm_providers_map(
    s: &SetupState,
) -> std::collections::HashMap<String, ModelProviderInfo> {
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
pub(super) fn advance_llm(app: &mut App, s: &mut SetupState) {
    match s.llm_substep {
        0 => {
            // Pick the highlighted preset or "custom".
            let presets = built_in_presets();
            let total = presets.len() + 1;
            if s.llm_highlight >= total {
                s.error = Some("invalid provider selection".into());
                return;
            }
            let id = presets
                .get(s.llm_highlight)
                .map_or("custom", |(id, _)| *id)
                .to_string();
            // Another provider's model and URL mean nothing here (they
            // used to stick: "custom . MiniMax-M3").
            if id != s.llm_provider_id {
                s.llm_model.clear();
                s.llm_url.clear();
            }
            s.llm_provider = Some(match presets.get(s.llm_highlight) {
                Some((_, provider)) => provider.clone(),
                None => ModelProviderInfo {
                    name: "custom".into(),
                    base_url: s.llm_url.clone(),
                    ..Default::default()
                },
            });
            s.llm_provider_id = id;
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
                    "could not save the key to ~/.moneyball/auth.json: {}",
                    e
                ));
                return;
            }
            // Read it back: a key that did not land is no key.
            if moneyball_core::secrets::load_llm_key(&s.llm_provider_id).as_deref() != Some(key) {
                s.error = Some(
                    "the key did not persist - check ~/.moneyball/auth.json is writable".into(),
                );
                let _ = moneyball_core::secrets::clear_llm_key(&s.llm_provider_id);
                return;
            }
            s.llm_key_len = key.chars().count();
            s.llm_input.clear();
            // For "custom", ask for the base URL next (reuse substep 1 input).
            if s.llm_provider_id == "custom" {
                s.llm_substep = 3;
                s.llm_url_entered = false;
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
            // Custom: the base URL first; enter moves on to the model.
            if !s.llm_url_entered {
                if s.llm_url.trim().is_empty() {
                    s.error = Some("enter the provider's base URL".into());
                    return;
                }
                s.llm_url_entered = true;
                s.error = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_provider_takes_its_url_then_its_model() {
        let cfg = moneyball_core::AppConfig {
            data_root: std::env::temp_dir().join("mb-setup-llm-none"),
            date: None,
            workspace: None,
            agent: true,
        };
        let mut app = App::new_for_test(cfg);
        let mut s = SetupState::new(app.cfg.data_root.clone());
        s.step = 4;
        // A model from the provider used before must not follow along.
        s.llm_provider_id = "minimax".into();
        s.llm_model = "MiniMax-M3".into();
        s.llm_highlight = built_in_presets().len(); // "custom", last
        advance_llm(&mut app, &mut s);
        assert_eq!(s.llm_provider_id, "custom");
        assert!(s.llm_model.is_empty(), "{}", s.llm_model);

        s.llm_substep = 3;
        for c in "http://localhost:8080/v1".chars() {
            insert_setup(&mut s, c);
        }
        assert_eq!(s.llm_url, "http://localhost:8080/v1");
        assert!(s.llm_model.is_empty());
        advance_llm(&mut app, &mut s);
        assert!(s.llm_url_entered && s.error.is_none(), "{:?}", s.error);
        for c in "qwen3".chars() {
            insert_setup(&mut s, c);
        }
        assert_eq!(
            (s.llm_url.as_str(), s.llm_model.as_str()),
            ("http://localhost:8080/v1", "qwen3")
        );
    }
}
