//! The wire formats, pure JSON: an agent history as each protocol's
//! messages, and the one-shot request bodies. Split from llm.rs (size
//! cap); llm.rs sends them.

use serde_json::{json, Value};

use crate::agent::Item;

/// Default max_tokens for Anthropic (it's required). 4096 is enough
/// for a brief commentary; bump if /ask starts producing longer replies.
pub(super) const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic Messages history: consecutive same-role blocks coalesce
/// into one message (required - tool_use must live in an assistant
/// message's content array, tool_result in the following user message).
pub(super) fn messages_history(history: &[Item]) -> Vec<Value> {
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
pub(super) fn chat_history(system: &str, history: &[Item]) -> Vec<Value> {
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
pub(super) fn responses_history(history: &[Item]) -> Vec<Value> {
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

// ---------- request body builders ----------

/// OpenAI Responses API. `instructions` is the system role; `input` is
/// a single user turn as a string.
pub(super) fn build_responses_body(model: &str, system: Option<&str>, user: &str) -> Value {
    json!({
        "model": model,
        "instructions": system.unwrap_or(""),
        "input": user,
    })
}

/// OpenAI Chat Completions API. system -> system message.
pub(super) fn build_chat_body(model: &str, system: Option<&str>, user: &str) -> Value {
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
pub(super) fn build_messages_body(model: &str, system: Option<&str>, user: &str) -> Value {
    json!({
        "model": model,
        "system": system.unwrap_or(""),
        "max_tokens": DEFAULT_MAX_TOKENS,
        "messages": [{"role": "user", "content": user}],
    })
}
