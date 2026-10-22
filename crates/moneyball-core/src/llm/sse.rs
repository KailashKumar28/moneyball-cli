//! What a streamed event says, per wire protocol: its text delta, an
//! error, a stop and why, and the tool calls it carries in pieces. Pure
//! JSON; llm.rs reads the stream. Split from llm.rs (size cap).

use serde_json::{json, Value};

use crate::provider::WireApi;
use crate::tools::ToolCall;

/// Non-delta signals a stream can carry, per wire protocol. The stream
/// loops act on these so a mid-stream provider failure never ends as a
/// "successful" partial answer, and a max_tokens stop is never silent.
pub(super) enum SseSignal {
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

pub(super) fn classify_event(wire: WireApi, v: &Value) -> SseSignal {
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
pub(super) fn is_length_stop(reason: &str) -> bool {
    matches!(reason, "max_tokens" | "length")
}

/// Pull the text fragment out of one SSE JSON event, per wire protocol.
pub(super) fn extract_stream_delta(wire: WireApi, v: &Value) -> Option<String> {
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

pub(super) fn truncate(s: &str, n: usize) -> String {
    crate::text::truncate_marked(s, n, "...")
}

/// Accumulates streamed tool-call fragments per wire protocol.
/// Messages: tool_use content_block_start + input_json_delta.
/// ChatCompletions: delta.tool_calls[] keyed by index.
/// Responses: complete function_call items on output_item.done.
#[derive(Default)]
pub(super) struct ToolCallAcc {
    // index -> (id, name, accumulated JSON args string)
    parts: std::collections::BTreeMap<u64, (String, String, String)>,
    done: Vec<ToolCall>,
}

impl ToolCallAcc {
    pub(super) fn feed(&mut self, wire: WireApi, v: &Value) {
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
    pub(super) fn finish(mut self) -> Vec<ToolCall> {
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
pub(super) fn parse_args(s: &str) -> Value {
    if s.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(s).unwrap_or_else(|_| json!({}))
}
