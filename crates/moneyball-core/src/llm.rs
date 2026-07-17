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
use crate::tools::{tools_payload, Completion, Tool, ToolCall};

/// Default request timeout. Long enough for a 7-day brief analysis but
/// short enough that the REPL doesn't hang forever on a stalled socket.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default max_tokens for Anthropic (it's required). 4096 is enough
/// for a brief commentary; bump if /ask starts producing longer replies.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// max_tokens for full agent turns on the Messages wire: an analysis
/// plus tool calls routinely outgrows the one-shot commentary budget,
/// and a length stop costs a whole model round trip (run_turn fails
/// the calls back). 8192 fits every Messages-wire model we target.
const AGENT_MAX_TOKENS: u32 = 8192;

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
        let mut req = self.inner.post(&url);
        for (k, v) in request_headers(provider, &api_key) {
            req = req.header(&k, &v);
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

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
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
        let mut req = self.inner.post(&url);
        for (k, v) in request_headers(provider, &api_key) {
            req = req.header(&k, &v);
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

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
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
            calls.push(ToolCall {
                id: o.id,
                name: o.name,
                arguments: args,
            });
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
                let args: Value =
                    serde_json::from_str(&parsed.function.arguments).unwrap_or(Value::Null);
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
                calls.push(ToolCall {
                    id: parsed.id,
                    name: parsed.name,
                    arguments: parsed.input,
                });
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
        let t = crate::tools::funnel_tool();
        let v = build_messages_body_with_tools("claude-sonnet-4-5", Some("sys"), "hi", &[t]);
        assert!(v["tools"].is_array());
        assert!(v["tools"][0].get("input_schema").is_some());
        assert!(v["tools"][0]["name"].as_str().unwrap() == "funnel");
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
            other => panic!(
                "expected ToolCalls (since tool_use present), got {:?}",
                other
            ),
        }
    }

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
    fn stream_delta_chat_wire() {
        let v: Value =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"lo "},"index":0}]}"#).unwrap();
        assert_eq!(
            extract_stream_delta(WireApi::ChatCompletions, &v).as_deref(),
            Some("lo ")
        );
        // Final chunk has empty delta - no text.
        let v2: Value =
            serde_json::from_str(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#).unwrap();
        assert!(extract_stream_delta(WireApi::ChatCompletions, &v2).is_none());
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
    fn classify_error_terminal_and_stop_per_wire() {
        let cases: &[(WireApi, &str, &str)] = &[
            (
                WireApi::Messages,
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
                "error",
            ),
            (WireApi::Messages, r#"{"type":"message_stop"}"#, "terminal"),
            (
                WireApi::Messages,
                r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{}}"#,
                "length",
            ),
            (
                WireApi::ChatCompletions,
                r#"{"error":{"message":"bad model"}}"#,
                "error",
            ),
            (
                WireApi::ChatCompletions,
                r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
                "length",
            ),
            (
                WireApi::ChatCompletions,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "stop",
            ),
            (
                WireApi::Responses,
                r#"{"type":"response.failed","response":{},"error":{"message":"boom"}}"#,
                "error",
            ),
            (WireApi::Responses, r#"{"type":"response.completed"}"#, "terminal"),
        ];
        for (wire, payload, want) in cases {
            let v: Value = serde_json::from_str(payload).unwrap();
            let got = match classify_event(*wire, &v) {
                SseSignal::Error(_) => "error".to_string(),
                SseSignal::Terminal => "terminal".to_string(),
                SseSignal::Stop(r) if is_length_stop(&r) => "length".to_string(),
                SseSignal::Stop(r) => r,
                SseSignal::None => "none".to_string(),
            };
            assert_eq!(&got, want, "payload: {}", payload);
        }
        // Plain deltas are not signals.
        let v: Value = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(
            classify_event(WireApi::Messages, &v),
            SseSignal::None
        ));
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
    fn acc_assembles_chat_wire_tool_call_by_index() {
        let mut acc = ToolCallAcc::default();
        acc.feed(
            WireApi::ChatCompletions,
            &json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"x","function":{"name":"funnel","arguments":"{\"p\""}}]}}]}),
        );
        acc.feed(
            WireApi::ChatCompletions,
            &json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":":\"P\"}"}}]}}]}),
        );
        let calls = acc.finish();
        assert_eq!(calls[0].name, "funnel");
        assert_eq!(calls[0].arguments["p"], "P");
    }

    #[test]
    fn bad_args_become_empty_object_not_panic() {
        assert_eq!(parse_args("{broken"), json!({}));
        assert_eq!(parse_args(""), json!({}));
    }
}
