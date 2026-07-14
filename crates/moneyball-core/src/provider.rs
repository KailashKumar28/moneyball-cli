//! LLM provider configuration. Code adapted from openai/codex's
//! `codex-rs/model-provider-info/src/lib.rs` but extended to support
//! multiple wire protocols (OpenAI Responses, OpenAI Chat Completions,
//! Anthropic Messages) since moneyball needs to talk to providers beyond
//! OpenAI - notably MiniMax, which is Anthropic-compatible.
//!
//! Schema follows codex:
//!   - `model_provider: String`         -> key into the providers map
//!   - `model: String`                  -> active model slug
//!   - `model_providers: HashMap<...>`  -> registry of known providers
//!
//! Each provider entry has the URL, an env_key (fallback for the API
//! key), the wire protocol, and optional custom headers/query params.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Wire protocol the provider speaks. Codex's current main locks this
/// to Responses only; moneyball needs the other two to talk to
/// Anthropic-compatible endpoints (MiniMax, Claude) and to legacy
/// OpenAI-compatible servers (Ollama, LM Studio).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    /// OpenAI Responses API at `${base_url}/responses`. Streaming SSE.
    #[default]
    Responses,
    /// OpenAI Chat Completions at `${base_url}/chat/completions`.
    ChatCompletions,
    /// Anthropic Messages API at `${base_url}/messages`. Headers:
    /// `x-api-key`, `anthropic-version: 2023-06-01`.
    Messages,
}

/// A provider entry. Stored in `WorkspaceConfig.model_providers` keyed
/// by a short id (e.g. "openai", "minimax", "ollama").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelProviderInfo {
    /// Friendly display name (e.g. "OpenAI", "MiniMax").
    #[serde(default)]
    pub name: String,

    /// Base URL for the provider's API. Required.
    pub base_url: String,

    /// Environment variable holding the API key. Used as a fallback if
    /// the keychain entry for this provider is empty / absent.
    #[serde(default)]
    pub env_key: Option<String>,

    /// Wire protocol. Defaults to Responses.
    #[serde(default)]
    pub wire_api: WireApi,

    /// Extra HTTP headers (literal value). Used e.g. for `anthropic-version`.
    #[serde(default)]
    pub http_headers: Option<HashMap<String, String>>,

    /// Extra HTTP headers where the value is an env var name. Resolved
    /// at request time; omitted if env is unset.
    #[serde(default)]
    pub env_http_headers: Option<HashMap<String, String>>,

    /// Optional query params appended to the base URL.
    #[serde(default)]
    pub query_params: Option<HashMap<String, String>>,
}

impl ModelProviderInfo {
    /// Resolve the API key for this provider. Tries the keychain first
    /// (`llm:<provider_id>`), falls back to the `env_key` env var.
    /// Returns `None` if neither source has a non-empty value.
    pub fn api_key(&self, provider_id: &str) -> Option<String> {
        if let Some(k) = crate::secrets::load_llm_key(provider_id) {
            if !k.trim().is_empty() {
                return Some(k);
            }
        }
        if let Some(var) = &self.env_key {
            if let Ok(v) = std::env::var(var) {
                if !v.trim().is_empty() {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Built-in preset for OpenAI (Responses, `https://api.openai.com/v1`,
    /// env_key `OPENAI_API_KEY`).
    pub fn openai() -> Self {
        Self {
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            env_key: Some("OPENAI_API_KEY".into()),
            wire_api: WireApi::Responses,
            http_headers: None,
            env_http_headers: None,
            query_params: None,
        }
    }

    /// Built-in preset for Ollama (ChatCompletions, localhost:11434).
    pub fn ollama() -> Self {
        Self {
            name: "Ollama (local)".into(),
            base_url: "http://localhost:11434/v1".into(),
            env_key: None,
            wire_api: WireApi::ChatCompletions,
            http_headers: None,
            env_http_headers: None,
            query_params: None,
        }
    }

    /// Built-in preset for MiniMax (Messages API, Anthropic-compatible).
    /// URL and model list to be confirmed with the user.
    pub fn minimax() -> Self {
        Self {
            name: "MiniMax".into(),
            base_url: "https://api.minimax.io/v1".into(),
            env_key: Some("MINIMAX_API_KEY".into()),
            wire_api: WireApi::Messages,
            http_headers: Some(HashMap::from([(
                "anthropic-version".into(),
                "2023-06-01".into(),
            )])),
            env_http_headers: None,
            query_params: None,
        }
    }

    /// Built-in preset for direct Anthropic.
    pub fn anthropic() -> Self {
        Self {
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            env_key: Some("ANTHROPIC_API_KEY".into()),
            wire_api: WireApi::Messages,
            http_headers: Some(HashMap::from([(
                "anthropic-version".into(),
                "2023-06-01".into(),
            )])),
            env_http_headers: None,
            query_params: None,
        }
    }
}

/// Curated model list per provider. Used by the wizard's model picker.
pub fn models_for(preset: &ModelProviderInfo) -> &'static [&'static str] {
    match (preset.name.as_str(), preset.wire_api) {
        ("OpenAI", _) => &[
            "gpt-5",
            "gpt-5-mini",
            "gpt-4.1",
            "gpt-4.1-mini",
            "o3",
            "o4-mini",
        ],
        ("Anthropic", _) => &[
            "claude-opus-4-5",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
        ],
        ("MiniMax", _) => &["MiniMax-M3", "MiniMax-M2"],
        _ => &["custom"],
    }
}

/// Built-in presets the wizard offers before the "custom" branch.
pub fn built_in_presets() -> Vec<(&'static str, ModelProviderInfo)> {
    vec![
        ("openai", ModelProviderInfo::openai()),
        ("anthropic", ModelProviderInfo::anthropic()),
        ("minimax", ModelProviderInfo::minimax()),
        ("ollama", ModelProviderInfo::ollama()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_api_serde_round_trip() {
        for (json, expected) in [
            (r#""responses""#, WireApi::Responses),
            (r#""chat_completions""#, WireApi::ChatCompletions),
            (r#""messages""#, WireApi::Messages),
        ] {
            let parsed: WireApi = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, expected);
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(back, json);
        }
    }

    #[test]
    fn openai_preset_uses_responses() {
        let p = ModelProviderInfo::openai();
        assert_eq!(p.wire_api, WireApi::Responses);
        assert_eq!(p.base_url, "https://api.openai.com/v1");
        assert_eq!(p.env_key.as_deref(), Some("OPENAI_API_KEY"));
    }

    #[test]
    fn minimax_preset_uses_messages() {
        let p = ModelProviderInfo::minimax();
        assert_eq!(p.wire_api, WireApi::Messages);
        let v = p.http_headers.as_ref().unwrap();
        assert_eq!(v.get("anthropic-version").unwrap(), "2023-06-01");
    }

    #[test]
    fn provider_info_round_trips_via_json() {
        let mut p = ModelProviderInfo::anthropic();
        p.query_params = Some(HashMap::from([("beta".into(), "true".into())]));
        let s = serde_json::to_string(&p).unwrap();
        let back: ModelProviderInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(back.base_url, p.base_url);
        assert_eq!(back.wire_api, p.wire_api);
        assert_eq!(
            back.query_params.unwrap().get("beta").unwrap(),
            "true"
        );
    }

    #[test]
    fn api_key_falls_back_to_env_when_keychain_empty() {
        let mut p = ModelProviderInfo::openai();
        p.env_key = Some("MONEYBALL_TEST_NONEXISTENT_VAR_XYZ".into());
        // No keychain entry was stored for "openai", so it must fall back
        // to the env var - which is unset, so the result is None.
        assert!(p.api_key("openai").is_none());
    }

    #[test]
    fn api_key_uses_env_when_set() {
        std::env::set_var("MONEYBALL_TEST_PROVIDER_KEY", "sk-test-abc");
        let mut p = ModelProviderInfo::openai();
        p.env_key = Some("MONEYBALL_TEST_PROVIDER_KEY".into());
        let k = p.api_key("openai");
        std::env::remove_var("MONEYBALL_TEST_PROVIDER_KEY");
        assert_eq!(k.as_deref(), Some("sk-test-abc"));
    }
}