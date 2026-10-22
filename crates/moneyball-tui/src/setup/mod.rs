//! First-run setup wizard: the state here; keys.rs routes keys,
//! steps.rs advances each step and saves, meta.rs / llm.rs own the token
//! and model steps, render_*.rs draw them.

use crate::*;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use llm::{advance_llm, handle_llm_picker_keys, llm_providers_map};
use meta::{advance_meta, meta_backspace, meta_insert};
use moneyball_core::provider::{built_in_presets, models_for, ModelProviderInfo};
use moneyball_core::{AdAccount, WorkspaceConfig};

mod keys;
mod llm;
mod meta;
mod render_llm;
mod render_meta;
mod steps;

pub(crate) use keys::{handle_setup_key, insert_setup};
use std::path::Path;
use steps::{advance_save, advance_setup};

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
    /// `/setup meta`: only replace the Meta token, keeping products and
    /// settings; the new token must read every configured account.
    pub token_only: bool,
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
    /// Step 4: API key paste buffer (raw, before the auth.json write).
    pub llm_input: String,
    /// Step 4: length of captured API key for collapsed-summary bullets.
    pub llm_key_len: usize,
    /// Step 4: selected model slug (e.g. "gpt-5", "MiniMax-M3").
    pub llm_model: String,
    /// Step 4: custom base URL (only when provider id is "custom").
    pub llm_url: String,
    /// Custom provider: the URL is done and keys now type the model.
    pub llm_url_entered: bool,
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
            token_only: false,
            product_input: String::new(),
            products: Vec::new(),
            goals_input: String::new(),
            llm_substep: 0,
            llm_provider_id: String::new(),
            llm_input: String::new(),
            llm_key_len: 0,
            llm_model: String::new(),
            llm_url: String::new(),
            llm_url_entered: false,
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
    ///   - llm_key_len (read from auth.json; 0 if not present)
    ///   - meta_connected (true if products were configured)
    ///
    /// What we cannot restore (user must redo if they want to change):
    ///   - meta_input (the Meta API token itself - not in memory or config)
    ///   - meta_discovered / meta_selections (require live API call)
    ///   - llm_input (the LLM API key - in auth.json only)
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
        // A saved token shows as its length in bullets, never "(0 chars)".
        s.meta_token_len = moneyball_core::secrets::load_meta_token()
            .map(|t| t.chars().count())
            .unwrap_or(0);
        // LLM
        if let Some(provider) = &cfg.model_provider {
            s.llm_provider_id = provider.clone();
        }
        if let Some(model) = &cfg.model {
            s.llm_model = model.clone();
        }
        if let Some(info) = cfg.model_providers.get(s.llm_provider_id.as_str()) {
            s.llm_provider = Some(info.clone());
            // A saved custom provider re-runs with its full endpoint in
            // the box: the endpoint is what says its protocol.
            if s.llm_provider_id == "custom" {
                s.llm_url = moneyball_core::llm::endpoint_for(info);
            }
        }
        // Read auth.json so the collapsed summary shows bullet count.
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

// ---------- render ----------

mod render_steps;
pub(crate) use render_steps::render_setup;
