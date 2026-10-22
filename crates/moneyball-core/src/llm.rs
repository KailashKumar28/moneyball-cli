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
use crate::tools::Tool;

mod sse;
mod wire;
use sse::*;
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
        usage_in(provider.wire_api, &v, &mut out.usage);
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
mod tests;
