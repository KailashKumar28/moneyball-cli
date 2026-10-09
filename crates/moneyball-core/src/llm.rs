//! LLM HTTP client: three wire protocols via the `WireApi` enum:
//!
//! - `Responses`        -> OpenAI Responses API: POST `{base}/responses`
//!   body `{model, instructions, input}` -> `{output[0].content[0].text}`
//! - `ChatCompletions`  -> POST `{base}/chat/completions`
//!   body `{model, messages}` -> `{choices[0].message.content}`
//! - `Messages`         -> Anthropic Messages API: POST `{base}/messages`
//!   body `{model, system, messages, max_tokens}` -> `{content[0].text}`
//!
//! Two entry points, both streaming SSE on a worker thread:
//! `stream_blocking` (one-shot prompt -> text, used by crm connect
//! drafting) and `stream_turn` (full history + tools, the agent loop).
//!
//! Auth: provider.api_key() resolves the auth.json entry first, then env_key.
//! On 401/403 we return `Error::LlmAuth` with the provider id so the
//! wizard can re-prompt.

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::provider::{ModelProviderInfo, WireApi};
use crate::tools::{Tool, ToolCall};

mod wire;
use wire::*;

/// max_tokens for full agent turns on the Messages wire: an analysis
/// plus tool calls routinely outgrows the one-shot commentary budget,
/// and a length stop costs a whole model round trip (run_turn fails
/// the calls back). 8192 fits every Messages-wire model we target.
const AGENT_MAX_TOKENS: u32 = 8192;

/// Streaming completion over SSE. Blocking - call from a worker thread.
/// Invokes `on_delta` for every text fragment as it arrives and returns
/// the accumulated full text. Supports all three wire protocols:
///   Messages        -> `content_block_delta` events (`delta.text`)
///   ChatCompletions -> `choices[0].delta.content`, terminated by [DONE]
///   Responses       -> `response.output_text.delta` events (`delta`)
pub fn stream_blocking(
    provider_id: &str,
    provider: &ModelProviderInfo,
    model: &str,
    system: Option<&str>,
    user: &str,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String> {
    use std::io::{BufRead, BufReader};

    let api_key = provider
        .api_key(provider_id)
        .ok_or_else(|| Error::LlmAuth(format!("no API key for provider '{}'", provider_id)))?;
    let mut body = match provider.wire_api {
        WireApi::Responses => build_responses_body(model, system, user),
        WireApi::ChatCompletions => build_chat_body(model, system, user),
        WireApi::Messages => build_messages_body(model, system, user),
    };
    body["stream"] = Value::Bool(true);

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300)) // whole stream, not per-chunk
        .build()
        .map_err(|e| Error::Llm(format!("client: {}", e)))?;
    let resp = request(&client, provider, &api_key)
        .json(&body)
        .send()
        .map_err(|e| Error::Llm(format!("transport: {}", e)))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(status_error(provider_id, status, &text));
    }

    let mut full = String::new();
    let mut terminated = false;
    let reader = BufReader::new(resp);
    for line in reader.lines() {
        let line = line.map_err(|e| Error::Llm(format!("stream read: {}", e)))?;
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
            continue; // event:/id:/blank keep-alive lines
        };
        if payload == "[DONE]" {
            terminated = true;
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        match classify_event(provider.wire_api, &v) {
            SseSignal::Error(msg) => {
                return Err(Error::Llm(format!(
                    "provider '{}' stream error: {}",
                    provider_id, msg
                )))
            }
            SseSignal::Terminal | SseSignal::Stop(_) => terminated = true,
            SseSignal::None => {}
        }
        if let Some(d) = extract_stream_delta(provider.wire_api, &v) {
            if !d.is_empty() {
                full.push_str(&d);
                on_delta(&d);
            }
        }
    }
    // An empty stream with no terminal event is a failure dressed as
    // success - e.g. an HTTP 200 whose body is a plain JSON error with
    // no data: lines. Never return Ok("") for those.
    if full.is_empty() && !terminated {
        return Err(Error::Llm(format!(
            "provider '{}' stream ended without content or a terminal event",
            provider_id
        )));
    }
    Ok(full)
}

/// 401/403 keep the LlmAuth contract (the wizard re-prompts on it);
/// everything else is a plain Llm error. One place for all four
/// request paths - copy drift here is what lost the contract before.
fn status_error(provider_id: &str, status: reqwest::StatusCode, body: &str) -> Error {
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        Error::LlmAuth(format!(
            "provider '{}' rejected the key (HTTP {}): {}",
            provider_id,
            status,
            truncate(body, 200)
        ))
    } else {
        Error::Llm(format!(
            "provider '{}' returned HTTP {}: {}",
            provider_id,
            status,
            truncate(body, 200)
        ))
    }
}

/// Non-delta signals a stream can carry, per wire protocol. The stream
/// loops act on these so a mid-stream provider failure never ends as a
/// "successful" partial answer, and a max_tokens stop is never silent.
enum SseSignal {
    /// Provider reported an error mid-stream (Anthropic `event: error`,
    /// Responses `response.failed`, chat `{"error": ...}` frames).
    Error(String),
    /// The stream terminated properly (message_stop / response.completed;
    /// chat's `[DONE]` is handled by the line loop itself).
    Terminal,
    /// The model stopped for this reason (stop_reason / finish_reason).
    Stop(String),
    None,
}

fn classify_event(wire: WireApi, v: &Value) -> SseSignal {
    let err_msg = |v: &Value| {
        v.pointer("/error/message")
            .and_then(|m| m.as_str())
            .map(String::from)
            .unwrap_or_else(|| truncate(&v.to_string(), 200))
    };
    match wire {
        WireApi::Messages => match v.get("type").and_then(|t| t.as_str()) {
            Some("error") => SseSignal::Error(err_msg(v)),
            Some("message_stop") => SseSignal::Terminal,
            Some("message_delta") => match v.pointer("/delta/stop_reason").and_then(|s| s.as_str())
            {
                Some(r) => SseSignal::Stop(r.to_string()),
                None => SseSignal::None,
            },
            _ => SseSignal::None,
        },
        WireApi::ChatCompletions => {
            if v.get("error").is_some() {
                return SseSignal::Error(err_msg(v));
            }
            match v
                .pointer("/choices/0/finish_reason")
                .and_then(|s| s.as_str())
            {
                Some(r) => SseSignal::Stop(r.to_string()),
                None => SseSignal::None,
            }
        }
        WireApi::Responses => match v.get("type").and_then(|t| t.as_str()) {
            Some("response.failed") | Some("error") => SseSignal::Error(err_msg(v)),
            Some("response.completed") => SseSignal::Terminal,
            Some("response.incomplete") => SseSignal::Stop("max_tokens".into()),
            _ => SseSignal::None,
        },
    }
}

/// Did the model stop because it ran out of output budget?
fn is_length_stop(reason: &str) -> bool {
    matches!(reason, "max_tokens" | "length")
}

/// Pull the text fragment out of one SSE JSON event, per wire protocol.
fn extract_stream_delta(wire: WireApi, v: &Value) -> Option<String> {
    match wire {
        WireApi::Messages => {
            if v.get("type")?.as_str()? != "content_block_delta" {
                return None;
            }
            v.pointer("/delta/text")?.as_str().map(String::from)
        }
        WireApi::ChatCompletions => v
            .pointer("/choices/0/delta/content")?
            .as_str()
            .map(String::from),
        WireApi::Responses => {
            if v.get("type")?.as_str()? != "response.output_text.delta" {
                return None;
            }
            v.get("delta")?.as_str().map(String::from)
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    crate::text::truncate_marked(s, n, "...")
}

// ---------- agent turn requests (full history + tools) ----------

use crate::agent::{Item, TurnResponse};
use std::sync::atomic::{AtomicBool, Ordering};

/// One agent-loop model request: sends the FULL history plus tool
/// definitions, streams text deltas, and accumulates tool calls
/// (ARCHITECTURE.md section 6b). Checks `cancel` between SSE events -
/// returning Err("interrupted") after dropping the stream is the
/// TCP-level cancel.
#[allow(clippy::too_many_arguments)]
pub fn stream_turn(
    provider_id: &str,
    provider: &ModelProviderInfo,
    model: &str,
    system: &str,
    history: &[Item],
    tools: &[Tool],
    cancel: &AtomicBool,
    on_delta: &mut dyn FnMut(&str),
) -> Result<TurnResponse> {
    use std::io::{BufRead, BufReader};

    let api_key = provider
        .api_key(provider_id)
        .ok_or_else(|| Error::LlmAuth(format!("no API key for provider '{}'", provider_id)))?;
    let mut body = match provider.wire_api {
        WireApi::Messages => json!({
            "model": model,
            "system": system,
            "max_tokens": AGENT_MAX_TOKENS,
            "messages": messages_history(history),
            "tools": tools.iter().map(|t| json!({
                "name": t.name, "description": t.description, "input_schema": t.parameters
            })).collect::<Vec<_>>(),
        }),
        WireApi::ChatCompletions => json!({
            "model": model,
            "messages": chat_history(system, history),
            "tools": tools.iter().map(|t| json!({
                "type": "function",
                "function": {"name": t.name, "description": t.description, "parameters": t.parameters}
            })).collect::<Vec<_>>(),
        }),
        WireApi::Responses => json!({
            "model": model,
            "instructions": system,
            "input": responses_history(history),
            "tools": tools.iter().map(|t| json!({
                "type": "function", "name": t.name,
                "description": t.description, "parameters": t.parameters
            })).collect::<Vec<_>>(),
        }),
    };
    body["stream"] = Value::Bool(true);

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| Error::Llm(format!("client: {}", e)))?;
    let resp = request(&client, provider, &api_key)
        .json(&body)
        .send()
        .map_err(|e| Error::Llm(format!("transport: {}", e)))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(status_error(provider_id, status, &text));
    }

    let mut out = TurnResponse::default();
    let mut acc = ToolCallAcc::default();
    let mut terminated = false;
    let reader = BufReader::new(resp);
    for line in reader.lines() {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Llm("interrupted".into()));
        }
        let line = line.map_err(|e| Error::Llm(format!("stream read: {}", e)))?;
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if payload == "[DONE]" {
            terminated = true;
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        match classify_event(provider.wire_api, &v) {
            SseSignal::Error(msg) => {
                return Err(Error::Llm(format!(
                    "provider '{}' stream error: {}",
                    provider_id, msg
                )))
            }
            SseSignal::Terminal => terminated = true,
            SseSignal::Stop(reason) => {
                terminated = true;
                if is_length_stop(&reason) {
                    out.truncated = true;
                }
            }
            SseSignal::None => {}
        }
        if let Some(d) = extract_stream_delta(provider.wire_api, &v) {
            if !d.is_empty() {
                out.text.push_str(&d);
                on_delta(&d);
            }
        }
        acc.feed(provider.wire_api, &v);
    }
    out.tool_calls = acc.finish();
    // Empty and terminal-less = a failure dressed as success (200 with
    // a non-SSE error body, dropped connection before any frame). A
    // silent dead turn violates the loop contract - fail loudly.
    if !terminated && out.text.is_empty() && out.tool_calls.is_empty() {
        return Err(Error::Llm(format!(
            "provider '{}' stream ended without content or a terminal event",
            provider_id
        )));
    }
    Ok(out)
}

/// Accumulates streamed tool-call fragments per wire protocol.
/// Messages: tool_use content_block_start + input_json_delta.
/// ChatCompletions: delta.tool_calls[] keyed by index.
/// Responses: complete function_call items on output_item.done.
#[derive(Default)]
struct ToolCallAcc {
    // index -> (id, name, accumulated JSON args string)
    parts: std::collections::BTreeMap<u64, (String, String, String)>,
    done: Vec<ToolCall>,
}

impl ToolCallAcc {
    fn feed(&mut self, wire: WireApi, v: &Value) {
        match wire {
            WireApi::Messages => match v.get("type").and_then(|t| t.as_str()) {
                Some("content_block_start")
                    if v.pointer("/content_block/type").and_then(|t| t.as_str())
                        == Some("tool_use") =>
                {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let id = str_at(v, "/content_block/id");
                    let name = str_at(v, "/content_block/name");
                    self.parts.insert(idx, (id, name, String::new()));
                }
                Some("content_block_delta")
                    if v.pointer("/delta/type").and_then(|t| t.as_str())
                        == Some("input_json_delta") =>
                {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    if let Some(p) = self.parts.get_mut(&idx) {
                        p.2.push_str(&str_at(v, "/delta/partial_json"));
                    }
                }
                _ => {}
            },
            WireApi::ChatCompletions => {
                let Some(calls) = v
                    .pointer("/choices/0/delta/tool_calls")
                    .and_then(|c| c.as_array())
                else {
                    return;
                };
                for c in calls {
                    let idx = c.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let e = self.parts.entry(idx).or_default();
                    if let Some(id) = c.get("id").and_then(|s| s.as_str()) {
                        e.0 = id.into();
                    }
                    if let Some(n) = c.pointer("/function/name").and_then(|s| s.as_str()) {
                        e.1 = n.into();
                    }
                    if let Some(a) = c.pointer("/function/arguments").and_then(|s| s.as_str()) {
                        e.2.push_str(a);
                    }
                }
            }
            WireApi::Responses => {
                if v.get("type").and_then(|t| t.as_str()) == Some("response.output_item.done")
                    && v.pointer("/item/type").and_then(|t| t.as_str()) == Some("function_call")
                {
                    self.done.push(ToolCall {
                        id: str_at(v, "/item/call_id"),
                        name: str_at(v, "/item/name"),
                        arguments: parse_args(&str_at(v, "/item/arguments")),
                    });
                }
            }
        }
    }
    fn finish(mut self) -> Vec<ToolCall> {
        for (_, (id, name, args)) in self.parts {
            self.done.push(ToolCall {
                id,
                name,
                arguments: parse_args(&args),
            });
        }
        self.done
    }
}

fn str_at(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Empty/invalid accumulated args become `{}` - the tool handler
/// reports bad args back to the model, never a parse crash here.
fn parse_args(s: &str) -> Value {
    if s.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(s).unwrap_or_else(|_| json!({}))
}

/// One provider call: its endpoint with the provider's query params
/// (Azure's api-version), content type, the wire's auth (x-api-key and
/// anthropic-version, or Bearer), literal http_headers, env-backed ones.
fn request(
    client: &reqwest::blocking::Client,
    provider: &ModelProviderInfo,
    api_key: &str,
) -> reqwest::blocking::RequestBuilder {
    let mut req = client
        .post(endpoint_for(provider))
        .header("Content-Type", "application/json");
    if let Some(q) = &provider.query_params {
        req = req.query(q);
    }
    req = match provider.wire_api {
        WireApi::Messages => req
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        _ => req.bearer_auth(api_key),
    };
    for (k, v) in provider.http_headers.iter().flatten() {
        req = req.header(k, v);
    }
    for (k, var) in provider.env_http_headers.iter().flatten() {
        if let Some(v) = std::env::var(var).ok().filter(|v| !v.is_empty()) {
            req = req.header(k, v);
        }
    }
    req
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_body_uses_instructions_and_input() {
        let v = build_responses_body(
            "gpt-5",
            Some("you are a moneyball advisor"),
            "show me namma",
        );
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
        let v = build_messages_body("claude-sonnet-5", Some("be brief"), "hi");
        assert_eq!(v["model"], "claude-sonnet-5");
        assert_eq!(v["system"], "be brief");
        assert_eq!(v["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(v["messages"][0]["role"], "user");
    }

    // ---------- tool-calling body builders ----------

    // ---------- tool-calling response parsers ----------

    // ---------- streaming delta extraction ----------

    #[test]
    fn stream_delta_messages_wire() {
        let v: Value = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
        )
        .unwrap();
        assert_eq!(
            extract_stream_delta(WireApi::Messages, &v).as_deref(),
            Some("Hel")
        );
        // Non-delta events yield nothing.
        let v2: Value = serde_json::from_str(r#"{"type":"message_start"}"#).unwrap();
        assert!(extract_stream_delta(WireApi::Messages, &v2).is_none());
    }

    #[test]
    fn stream_delta_responses_wire() {
        let v: Value =
            serde_json::from_str(r#"{"type":"response.output_text.delta","delta":"world"}"#)
                .unwrap();
        assert_eq!(
            extract_stream_delta(WireApi::Responses, &v).as_deref(),
            Some("world")
        );
        let v2: Value = serde_json::from_str(r#"{"type":"response.completed"}"#).unwrap();
        assert!(extract_stream_delta(WireApi::Responses, &v2).is_none());
    }

    #[test]
    fn status_error_keeps_llm_auth_contract() {
        assert!(matches!(
            status_error("p", reqwest::StatusCode::UNAUTHORIZED, "no"),
            Error::LlmAuth(_)
        ));
        assert!(matches!(
            status_error("p", reqwest::StatusCode::FORBIDDEN, "no"),
            Error::LlmAuth(_)
        ));
        assert!(matches!(
            status_error("p", reqwest::StatusCode::INTERNAL_SERVER_ERROR, "boom"),
            Error::Llm(_)
        ));
    }
}

#[cfg(test)]
mod turn_tests {
    use super::*;

    fn hist() -> Vec<Item> {
        vec![
            Item::User { text: "q".into() },
            Item::ToolCall {
                call_id: "c1".into(),
                name: "funnel".into(),
                args: json!({"product": "P"}),
            },
            Item::ToolOutput {
                call_id: "c1".into(),
                output: "table".into(),
                is_error: false,
            },
            Item::Assistant { text: "ans".into() },
        ]
    }

    #[test]
    fn messages_history_coalesces_roles_and_maps_tool_blocks() {
        let m = messages_history(&hist());
        // user(q) | assistant(tool_use) | user(tool_result) | assistant(text)
        assert_eq!(m.len(), 4);
        assert_eq!(m[1]["content"][0]["type"], "tool_use");
        assert_eq!(m[1]["content"][0]["input"]["product"], "P");
        assert_eq!(m[2]["content"][0]["type"], "tool_result");
        assert_eq!(m[2]["content"][0]["tool_use_id"], "c1");
    }

    #[test]
    fn chat_history_uses_tool_role_and_stringified_args() {
        let m = chat_history("sys", &hist());
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "funnel");
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[3]["tool_call_id"], "c1");
    }

    #[test]
    fn responses_history_emits_typed_items() {
        let m = responses_history(&hist());
        assert_eq!(m[1]["type"], "function_call");
        assert_eq!(m[2]["type"], "function_call_output");
    }

    #[test]
    fn acc_assembles_messages_wire_tool_call_from_deltas() {
        let mut acc = ToolCallAcc::default();
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_start","index":1,
                "content_block":{"type":"tool_use","id":"c9","name":"brief"}}),
        );
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_delta","index":1,
                "delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}),
        );
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_delta","index":1,
                "delta":{"type":"input_json_delta","partial_json":"1}"}}),
        );
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c9");
        assert_eq!(calls[0].arguments["a"], 1);
    }

    #[test]
    fn bad_args_become_empty_object_not_panic() {
        assert_eq!(parse_args("{broken"), json!({}));
        assert_eq!(parse_args(""), json!({}));
    }

    #[test]
    fn a_call_carries_the_providers_query_params_and_auth() {
        let p = ModelProviderInfo {
            base_url: "https://x.openai.azure.com/openai".into(),
            wire_api: WireApi::ChatCompletions,
            query_params: Some(std::collections::HashMap::from([(
                "api-version".into(),
                "2025-01-01".into(),
            )])),
            ..Default::default()
        };
        let req = request(&reqwest::blocking::Client::new(), &p, "k")
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://x.openai.azure.com/openai/chat/completions?api-version=2025-01-01"
        );
        assert_eq!(req.headers()["authorization"], "Bearer k");
    }
}
