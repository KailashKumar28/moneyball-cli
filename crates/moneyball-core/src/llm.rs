//! LLM HTTP client. One `Client` per process; supports three wire
//! protocols via the `WireApi` enum:
//!
//! - `Responses`        -> OpenAI Responses API: POST `{base}/responses`
//!   body `{model, instructions, input}` -> `{output[0].content[0].text}`
//! - `ChatCompletions`  -> POST `{base}/chat/completions`
//!   body `{model, messages}` -> `{choices[0].message.content}`
//! - `Messages`         -> Anthropic Messages API: POST `{base}/messages`
//!   body `{model, system, messages, max_tokens}` -> `{content[0].text}`
//!
//! Streaming can be added later. For now we expose `complete()` which
//! returns the full assistant text. This is enough to drive the first
//! LLM-driven `/brief`; we can swap to streaming once the chat TUI cell
//! wiring is in place.
//!
//! Auth: provider.api_key() resolves a keychain entry first, then env_key.
//! On 401/403 we return `Error::LlmAuth` with the provider id so the
//! wizard can re-prompt.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::provider::{ModelProviderInfo, WireApi};

/// Default request timeout. Long enough for a 7-day brief analysis but
/// short enough that the REPL doesn't hang forever on a stalled socket.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default max_tokens for Anthropic (it's required). 4096 is enough
/// for a brief commentary; bump if /ask starts producing longer replies.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// HTTP client wrapper. Cheap to clone (reqwest::Client is Arc-internal).
#[derive(Debug, Clone)]
pub struct Client {
    inner: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("reqwest client builder");
        Self { inner }
    }

    /// One-shot non-streaming completion. Returns the assistant text.
    /// `provider_id` is used to look up the keychain entry for the API key.
    pub async fn complete(
        &self,
        provider_id: &str,
        provider: &ModelProviderInfo,
        model: &str,
        system: Option<&str>,
        user: &str,
    ) -> Result<String> {
        let api_key = provider
            .api_key(provider_id)
            .ok_or_else(|| Error::LlmAuth(format!("no API key for provider '{}'", provider_id)))?;

        let url = endpoint_for(provider);
        let mut req = self
            .inner
            .post(&url)
            .header("Content-Type", "application/json");

        // Wire-specific auth header.
        req = match provider.wire_api {
            WireApi::Messages => req
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => req.header("Authorization", format!("Bearer {}", api_key)),
        };

        // Literal http_headers from the provider config.
        if let Some(hh) = &provider.http_headers {
            for (k, v) in hh {
                req = req.header(k, v);
            }
        }

        // Env-backed headers.
        if let Some(eh) = &provider.env_http_headers {
            for (k, var) in eh {
                if let Ok(v) = std::env::var(var) {
                    if !v.is_empty() {
                        req = req.header(k, v);
                    }
                }
            }
        }

        let body = match provider.wire_api {
            WireApi::Responses => build_responses_body(model, system, user),
            WireApi::ChatCompletions => build_chat_body(model, system, user),
            WireApi::Messages => build_messages_body(model, system, user),
        };
        let req = req.json(&body);

        let resp = req
            .send()
            .await
            .map_err(|e| Error::Llm(format!("transport: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::Llm(format!("body read: {}", e)))?;

        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(Error::LlmAuth(format!(
                "provider '{}' rejected the key (HTTP {}): {}",
                provider_id,
                status,
                truncate(&text, 200)
            )));
        }
        if !status.is_success() {
            return Err(Error::Llm(format!(
                "provider '{}' returned HTTP {}: {}",
                provider_id,
                status,
                truncate(&text, 200)
            )));
        }

        parse_assistant_text(provider.wire_api, &text).ok_or_else(|| {
            Error::Llm(format!(
                "could not parse assistant text from '{}' response: {}",
                wire_api_name(provider.wire_api),
                truncate(&text, 200)
            ))
        })
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...", &s[..n])
    }
}

/// Compute the full URL for a provider's request endpoint.
pub fn endpoint_for(provider: &ModelProviderInfo) -> String {
    let base = provider.base_url.trim_end_matches('/');
    match provider.wire_api {
        WireApi::Responses => format!("{}/responses", base),
        WireApi::ChatCompletions => format!("{}/chat/completions", base),
        WireApi::Messages => format!("{}/messages", base),
    }
}

// ---------- request body builders ----------

/// OpenAI Responses API. `instructions` is the system role; `input` is
/// a single user turn as a string.
fn build_responses_body(model: &str, system: Option<&str>, user: &str) -> Value {
    json!({
        "model": model,
        "instructions": system.unwrap_or(""),
        "input": user,
    })
}

/// OpenAI Chat Completions API. system -> system message.
fn build_chat_body(model: &str, system: Option<&str>, user: &str) -> Value {
    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(json!({ "role": "system", "content": s }));
    }
    messages.push(json!({ "role": "user", "content": user }));
    json!({
        "model": model,
        "messages": messages,
    })
}

/// Anthropic Messages API. system is top-level (not a message); max_tokens
/// is required.
fn build_messages_body(model: &str, system: Option<&str>, user: &str) -> Value {
    json!({
        "model": model,
        "system": system.unwrap_or(""),
        "max_tokens": DEFAULT_MAX_TOKENS,
        "messages": [{"role": "user", "content": user}],
    })
}

// ---------- response parsers ----------

#[derive(Debug, Deserialize)]
struct ResponsesResp {
    output: Vec<ResponsesOutput>,
}
#[derive(Debug, Deserialize)]
struct ResponsesOutput {
    #[serde(default)]
    content: Vec<ResponsesContent>,
}
#[derive(Debug, Deserialize)]
struct ResponsesContent {
    #[serde(rename = "type")]
    #[serde(default)]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Debug, Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
}
#[derive(Debug, Deserialize)]
struct ChatResp {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct MessagesContent {
    #[serde(rename = "type")]
    #[serde(default)]
    kind: String,
    #[serde(default)]
    text: String,
}
#[derive(Debug, Deserialize)]
struct MessagesResp {
    #[serde(default)]
    content: Vec<MessagesContent>,
}

/// Extract assistant text from a parsed JSON body. Returns None if the
/// payload doesn't match the expected shape.
pub fn parse_assistant_text(wire: WireApi, body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    match wire {
        WireApi::Responses => {
            let r: ResponsesResp = serde_json::from_value(v).ok()?;
            r.output
                .into_iter()
                .flat_map(|o| o.content)
                .find(|c| c.kind == "output_text" || c.kind.is_empty())
                .map(|c| c.text)
        }
        WireApi::ChatCompletions => {
            let r: ChatResp = serde_json::from_value(v).ok()?;
            r.choices.into_iter().next().map(|c| c.message.content)
        }
        WireApi::Messages => {
            let r: MessagesResp = serde_json::from_value(v).ok()?;
            r.content
                .into_iter()
                .find(|c| c.kind == "text" || c.kind.is_empty())
                .map(|c| c.text)
        }
    }
}

// ---------- error variants ----------
// We add the new Llm / LlmAuth variants to moneyball-core::error.
// They're declared in error.rs; we just declare the call sites here.

fn wire_api_name(w: WireApi) -> &'static str {
    match w {
        WireApi::Responses => "Responses",
        WireApi::ChatCompletions => "ChatCompletions",
        WireApi::Messages => "Messages",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ModelProviderInfo;

    #[test]
    fn endpoint_for_each_wire() {
        let p = ModelProviderInfo {
            base_url: "https://api.example.com/v1/".into(),
            ..Default::default()
        };
        let mut p_responses = p.clone();
        p_responses.wire_api = WireApi::Responses;
        assert_eq!(
            endpoint_for(&p_responses),
            "https://api.example.com/v1/responses"
        );
        let mut p_chat = p.clone();
        p_chat.wire_api = WireApi::ChatCompletions;
        assert_eq!(
            endpoint_for(&p_chat),
            "https://api.example.com/v1/chat/completions"
        );
        let mut p_messages = p;
        p_messages.wire_api = WireApi::Messages;
        assert_eq!(
            endpoint_for(&p_messages),
            "https://api.example.com/v1/messages"
        );
    }

    #[test]
    fn responses_body_uses_instructions_and_input() {
        let v = build_responses_body("gpt-5", Some("you are a moneyball advisor"), "show me namma");
        assert_eq!(v["model"], "gpt-5");
        assert_eq!(v["instructions"], "you are a moneyball advisor");
        assert_eq!(v["input"], "show me namma");
    }

    #[test]
    fn chat_body_separates_system_message() {
        let v = build_chat_body("gpt-oss", Some("be terse"), "hello");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "be terse");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "hello");
    }

    #[test]
    fn chat_body_no_system() {
        let v = build_chat_body("gpt-oss", None, "hello");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn messages_body_top_level_system_and_max_tokens() {
        let v = build_messages_body("claude-sonnet-4-5", Some("be brief"), "hi");
        assert_eq!(v["model"], "claude-sonnet-4-5");
        assert_eq!(v["system"], "be brief");
        assert_eq!(v["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(v["messages"][0]["role"], "user");
    }

    #[test]
    fn parse_responses_output_text() {
        let body = r#"{
            "id": "resp_1",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello world"}]
            }]
        }"#;
        assert_eq!(
            parse_assistant_text(WireApi::Responses, body).as_deref(),
            Some("Hello world")
        );
    }

    #[test]
    fn parse_chat_completions_first_choice() {
        let body = r#"{
            "choices": [
                {"message": {"role": "assistant", "content": "Hi there"}}
            ]
        }"#;
        assert_eq!(
            parse_assistant_text(WireApi::ChatCompletions, body).as_deref(),
            Some("Hi there")
        );
    }

    #[test]
    fn parse_messages_text_block() {
        let body = r#"{
            "content": [{"type": "text", "text": "Greetings"}]
        }"#;
        assert_eq!(
            parse_assistant_text(WireApi::Messages, body).as_deref(),
            Some("Greetings")
        );
    }

    #[test]
    fn parse_empty_body_returns_none() {
        assert!(parse_assistant_text(WireApi::Responses, "").is_none());
        assert!(parse_assistant_text(WireApi::ChatCompletions, "not json").is_none());
        assert!(parse_assistant_text(WireApi::Messages, "{}").is_none());
    }

    #[test]
    fn parse_unknown_wire_returns_none() {
        // Sanity: malformed body for each wire.
        assert!(parse_assistant_text(WireApi::Responses, "{}").is_none());
        assert!(parse_assistant_text(WireApi::ChatCompletions, "{}").is_none());
    }

    #[test]
    fn client_default_builds() {
        // Smoke test: just constructing the client must not panic.
        let _ = Client::new();
        let _ = Client::default();
    }
}