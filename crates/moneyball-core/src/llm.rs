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
use crate::tools::{Completion, Tool, ToolCall, tools_payload};

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

    /// Tool-calling completion. The provider may either return text
    /// (final answer) or one-or-more tool calls. The agent loop in
    /// moneyball-tui feeds the tool results back as another turn via
    /// `complete_with_tools(..., prior_tool_results)`.
    ///
    /// `tools` is the full registry available to this turn. Pass an
    /// empty slice to disable tool calling (the LLM still answers as
    /// a normal chat completion).
    pub async fn complete_with_tools(
        &self,
        provider_id: &str,
        provider: &ModelProviderInfo,
        model: &str,
        system: Option<&str>,
        user: &str,
        tools: &[Tool],
    ) -> Result<Completion> {
        let api_key = provider
            .api_key(provider_id)
            .ok_or_else(|| Error::LlmAuth(format!("no API key for provider '{}'", provider_id)))?;

        let url = endpoint_for(provider);
        let mut req = self
            .inner
            .post(&url)
            .header("Content-Type", "application/json");

        req = match provider.wire_api {
            WireApi::Messages => req
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => req.header("Authorization", format!("Bearer {}", api_key)),
        };

        if let Some(hh) = &provider.http_headers {
            for (k, v) in hh {
                req = req.header(k, v);
            }
        }
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
            WireApi::Responses => build_responses_body_with_tools(model, system, user, tools),
            WireApi::ChatCompletions => build_chat_body_with_tools(model, system, user, tools),
            WireApi::Messages => build_messages_body_with_tools(model, system, user, tools),
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

        parse_completion(provider.wire_api, &text).ok_or_else(|| {
            Error::Llm(format!(
                "could not parse completion from '{}' response: {}",
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

// ---------- tool-calling body builders ----------

/// OpenAI Responses with tools. `tools` may be empty (omit the field).
fn build_responses_body_with_tools(
    model: &str,
    system: Option<&str>,
    user: &str,
    tools: &[Tool],
) -> Value {
    let mut body = json!({
        "model": model,
        "instructions": system.unwrap_or(""),
        "input": user,
    });
    if !tools.is_empty() {
        body["tools"] = tools_payload(WireApi::Responses, tools);
    }
    body
}

/// OpenAI Chat Completions with tools.
fn build_chat_body_with_tools(
    model: &str,
    system: Option<&str>,
    user: &str,
    tools: &[Tool],
) -> Value {
    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(json!({ "role": "system", "content": s }));
    }
    messages.push(json!({ "role": "user", "content": user }));
    let mut body = json!({
        "model": model,
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = tools_payload(WireApi::ChatCompletions, tools);
    }
    body
}

/// Anthropic Messages with tools. `tools` is always at the top level.
fn build_messages_body_with_tools(
    model: &str,
    system: Option<&str>,
    user: &str,
    tools: &[Tool],
) -> Value {
    let mut body = json!({
        "model": model,
        "system": system.unwrap_or(""),
        "max_tokens": DEFAULT_MAX_TOKENS,
        "messages": [{"role": "user", "content": user}],
    });
    if !tools.is_empty() {
        body["tools"] = tools_payload(WireApi::Messages, tools);
    }
    body
}

// ---------- tool-calling response parser ----------

/// OpenAI Chat Completions tool_call shape.
#[derive(Debug, Deserialize)]
struct ChatToolCall {
    #[serde(default)]
    id: String,
    function: ChatToolCallFn,
}
#[derive(Debug, Deserialize)]
struct ChatToolCallFn {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

/// Anthropic Messages tool_use block shape.
#[derive(Debug, Deserialize)]
struct AnthropicToolUse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    /// Anthropic sends `input` as a JSON object, not a string.
    #[serde(default)]
    input: Value,
}

/// Parse a completion response. Prefers tool_calls over text if both
/// are present (the LLM is expected to either respond or call, not
/// both at once - but if it does, the tool calls win).
pub fn parse_completion(wire: WireApi, body: &str) -> Option<Completion> {
    let v: Value = serde_json::from_str(body).ok()?;
    match wire {
        WireApi::Responses => parse_responses_completion(&v),
        WireApi::ChatCompletions => parse_chat_completion(&v),
        WireApi::Messages => parse_anthropic_completion(&v),
    }
}

fn parse_responses_completion(v: &Value) -> Option<Completion> {
    // Newer Responses API: output[] has both `message` (text) and
    // `function_call` items at the same level. Walk output[] once.
    #[derive(Deserialize)]
    struct Out {
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        arguments: Value,
        #[serde(default, alias = "call_id", alias = "id")]
        id: String,
        #[serde(default)]
        content: Vec<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        text: String,
    }
    let outputs: Vec<Out> = serde_json::from_value(v.get("output")?.clone()).ok()?;
    let mut calls = Vec::new();
    let mut text = String::new();
    for o in outputs {
        if o.kind == "function_call" {
            let args = if o.arguments.is_string() {
                serde_json::from_str(&o.arguments.to_string()).unwrap_or(Value::Null)
            } else {
                o.arguments
            };
            calls.push(ToolCall { id: o.id, name: o.name, arguments: args });
        } else {
            // text or message
            for c in o.content {
                if (c.kind == "output_text" || c.kind.is_empty()) && !c.text.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&c.text);
                }
            }
        }
    }
    if !calls.is_empty() {
        Some(Completion::ToolCalls(calls))
    } else if !text.is_empty() {
        Some(Completion::Text(text))
    } else {
        None
    }
}

fn parse_chat_completion(v: &Value) -> Option<Completion> {
    let choice = v.get("choices")?.as_array()?.first()?;
    let msg = choice.get("message")?;
    let text = msg
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let mut calls = Vec::new();
    if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tcs {
            if let Ok(parsed) = serde_json::from_value::<ChatToolCall>(tc.clone()) {
                let args: Value = serde_json::from_str(&parsed.function.arguments)
                    .unwrap_or(Value::Null);
                calls.push(ToolCall {
                    id: parsed.id,
                    name: parsed.function.name,
                    arguments: args,
                });
            }
        }
    }
    if !calls.is_empty() {
        Some(Completion::ToolCalls(calls))
    } else if !text.is_empty() {
        Some(Completion::Text(text))
    } else {
        None
    }
}

fn parse_anthropic_completion(v: &Value) -> Option<Completion> {
    let content = v.get("content")?.as_array()?;
    let mut calls = Vec::new();
    let mut text = String::new();
    for block in content {
        let kind = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind == "tool_use" {
            if let Ok(parsed) = serde_json::from_value::<AnthropicToolUse>(block.clone()) {
                calls.push(ToolCall { id: parsed.id, name: parsed.name, arguments: parsed.input });
            }
        } else if kind == "text" || kind.is_empty() {
            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
    }
    if !calls.is_empty() {
        Some(Completion::ToolCalls(calls))
    } else if !text.is_empty() {
        Some(Completion::Text(text))
    } else {
        None
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

    // ---------- tool-calling body builders ----------

    #[test]
    fn responses_body_with_tools_includes_tools_array() {
        let t = crate::tools::brief_tool();
        let v = build_responses_body_with_tools("gpt-5", Some("sys"), "hi", &[t]);
        assert_eq!(v["model"], "gpt-5");
        assert!(v["tools"].is_array());
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "brief");
    }

    #[test]
    fn chat_body_with_tools_omits_field_when_empty() {
        let v = build_chat_body_with_tools("gpt-4", None, "hi", &[]);
        assert!(v.get("tools").is_none());
    }

    #[test]
    fn anthropic_body_with_tools_uses_input_schema() {
        let t = crate::tools::diagnose_tool();
        let v = build_messages_body_with_tools("claude-sonnet-4-5", Some("sys"), "hi", &[t]);
        assert!(v["tools"].is_array());
        assert!(v["tools"][0].get("input_schema").is_some());
        assert!(v["tools"][0]["name"].as_str().unwrap() == "diagnose");
    }

    // ---------- tool-calling response parsers ----------

    #[test]
    fn parse_responses_text_only() {
        let body = r#"{
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello"}]
            }]
        }"#;
        match parse_completion(WireApi::Responses, body).unwrap() {
            Completion::Text(t) => assert_eq!(t, "Hello"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn parse_responses_function_call() {
        let body = r#"{
            "output": [{
                "type": "function_call",
                "name": "brief",
                "arguments": "{}",
                "call_id": "call_1"
            }]
        }"#;
        match parse_completion(WireApi::Responses, body).unwrap() {
            Completion::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "brief");
                assert_eq!(calls[0].id, "call_1");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    #[test]
    fn parse_responses_function_call_object_arguments() {
        // Newer Responses API may send arguments as an object, not a string.
        let body = r#"{
            "output": [{
                "type": "function_call",
                "name": "diagnose",
                "arguments": {"product": "Namma Mane"},
                "call_id": "call_2"
            }]
        }"#;
        match parse_completion(WireApi::Responses, body).unwrap() {
            Completion::ToolCalls(calls) => {
                assert_eq!(calls[0].name, "diagnose");
                assert_eq!(calls[0].arguments["product"], "Namma Mane");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    #[test]
    fn parse_chat_completions_tool_calls() {
        let body = r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "tc_1",
                        "type": "function",
                        "function": {"name": "brief", "arguments": "{}"}
                    }]
                }
            }]
        }"#;
        match parse_completion(WireApi::ChatCompletions, body).unwrap() {
            Completion::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "tc_1");
                assert_eq!(calls[0].name, "brief");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    #[test]
    fn parse_anthropic_tool_use() {
        let body = r#"{
            "content": [
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "id": "toolu_1", "name": "brief", "input": {}}
            ]
        }"#;
        match parse_completion(WireApi::Messages, body).unwrap() {
            Completion::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "toolu_1");
                assert_eq!(calls[0].name, "brief");
            }
            other => panic!("expected ToolCalls (since tool_use present), got {:?}", other),
        }
    }

    #[test]
    fn parse_anthropic_text_only() {
        let body = r#"{
            "content": [{"type": "text", "text": "All healthy."}]
        }"#;
        match parse_completion(WireApi::Messages, body).unwrap() {
            Completion::Text(t) => assert_eq!(t, "All healthy."),
            other => panic!("expected Text, got {:?}", other),
        }
    }
}