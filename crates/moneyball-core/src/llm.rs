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
//! Two entry points, both streaming SSE on a worker thread:
//! `stream_blocking` (one-shot prompt -> text, used by crm connect
//! drafting) and `stream_turn` (full history + tools, the agent loop).
//!
//! Auth: provider.api_key() resolves a keychain entry first, then env_key.
//! On 401/403 we return `Error::LlmAuth` with the provider id so the
//! wizard can re-prompt.

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::provider::{ModelProviderInfo, WireApi};
use crate::tools::{Tool, ToolCall};

/// Default max_tokens for Anthropic (it's required). 4096 is enough
/// for a brief commentary; bump if /ask starts producing longer replies.
const DEFAULT_MAX_TOKENS: u32 = 4096;

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
    let url = endpoint_for(provider);

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
    let mut req = client.post(&url);
    for (k, v) in request_headers(provider, &api_key) {
        req = req.header(&k, &v);
    }

    let resp = req
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
            match v.pointer("/choices/0/finish_reason").and_then(|s| s.as_str()) {
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
    // Char-boundary walk: error bodies contain multibyte text (Rs signs,
    // lead names) and a raw byte slice would panic.
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == s.len() {
        s.to_string()
    } else {
        format!("{}...", &s[..end])
    }
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
    let mut req = client.post(endpoint_for(provider));
    for (k, v) in request_headers(provider, &api_key) {
        req = req.header(&k, &v);
    }
    let resp = req
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

/// Anthropic Messages history: consecutive same-role blocks coalesce
/// into one message (required - tool_use must live in an assistant
/// message's content array, tool_result in the following user message).
fn messages_history(history: &[Item]) -> Vec<Value> {
    let mut msgs: Vec<Value> = Vec::new();
    let push = |role: &str, block: Value, msgs: &mut Vec<Value>| {
        if let Some(last) = msgs.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some(role) {
                if let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                    arr.push(block);
                    return;
                }
            }
        }
        msgs.push(json!({"role": role, "content": [block]}));
    };
    for item in history {
        match item {
            Item::User { text } => push("user", json!({"type": "text", "text": text}), &mut msgs),
            Item::Assistant { text } => push(
                "assistant",
                json!({"type": "text", "text": text}),
                &mut msgs,
            ),
            Item::ToolCall {
                call_id,
                name,
                args,
            } => push(
                "assistant",
                json!({"type": "tool_use", "id": call_id, "name": name,
                       "input": if args.is_null() { json!({}) } else { args.clone() }}),
                &mut msgs,
            ),
            Item::ToolOutput {
                call_id,
                output,
                is_error,
            } => push(
                "user",
                json!({"type": "tool_result", "tool_use_id": call_id,
                       "content": output, "is_error": is_error}),
                &mut msgs,
            ),
        }
    }
    msgs
}

/// OpenAI Chat Completions history: tool calls ride on assistant
/// messages, results are role:"tool" messages.
fn chat_history(system: &str, history: &[Item]) -> Vec<Value> {
    let mut msgs = vec![json!({"role": "system", "content": system})];
    for item in history {
        match item {
            Item::User { text } => msgs.push(json!({"role": "user", "content": text})),
            Item::Assistant { text } => msgs.push(json!({"role": "assistant", "content": text})),
            Item::ToolCall {
                call_id,
                name,
                args,
            } => {
                let call = json!({"id": call_id, "type": "function",
                    "function": {"name": name, "arguments": args.to_string()}});
                let attach = msgs
                    .last_mut()
                    .filter(|m| m.get("tool_calls").is_some())
                    .and_then(|m| m.get_mut("tool_calls"))
                    .and_then(|t| t.as_array_mut());
                match attach {
                    Some(arr) => arr.push(call),
                    None => msgs
                        .push(json!({"role": "assistant", "content": null, "tool_calls": [call]})),
                }
            }
            Item::ToolOutput {
                call_id, output, ..
            } => msgs.push(json!({"role": "tool", "tool_call_id": call_id, "content": output})),
        }
    }
    msgs
}

/// OpenAI Responses history: typed input items.
fn responses_history(history: &[Item]) -> Vec<Value> {
    history
        .iter()
        .map(|item| match item {
            Item::User { text } => json!({"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": text}]}),
            Item::Assistant { text } => json!({"type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": text}]}),
            Item::ToolCall {
                call_id,
                name,
                args,
            } => json!({"type": "function_call", "call_id": call_id,
                "name": name, "arguments": args.to_string()}),
            Item::ToolOutput {
                call_id, output, ..
            } => json!({"type": "function_call_output", "call_id": call_id, "output": output}),
        })
        .collect()
}

/// All request headers for a provider call: content type, wire-specific
/// auth (x-api-key+anthropic-version vs Bearer), literal http_headers,
/// env-backed headers. Returned as pairs so the async and blocking reqwest
/// builders (distinct types) share one source of truth.
fn request_headers(provider: &ModelProviderInfo, api_key: &str) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = vec![("Content-Type".into(), "application/json".into())];
    match provider.wire_api {
        WireApi::Messages => {
            h.push(("x-api-key".into(), api_key.to_string()));
            h.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        _ => h.push(("Authorization".into(), format!("Bearer {}", api_key))),
    }
    if let Some(hh) = &provider.http_headers {
        for (k, v) in hh {
            h.push((k.clone(), v.clone()));
        }
    }
    if let Some(eh) = &provider.env_http_headers {
        for (k, var) in eh {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    h.push((k.clone(), v));
                }
            }
        }
    }
    h
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
        let v = build_messages_body("claude-sonnet-4-5", Some("be brief"), "hi");
        assert_eq!(v["model"], "claude-sonnet-4-5");
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
}
