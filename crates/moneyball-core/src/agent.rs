//! Agent core - conversation history, the tool loop, and the front-end
//! seam. Binding design: ARCHITECTURE.md section 6b (codex-rs + pi-mono).
//! A front-end drives it through `Session` (driver.rs).
//!
//! History is the wire format: one `Item` enum is the in-memory
//! transcript, the prompt (full history every request), and the JSONL
//! persistence format. The loop sends history, executes tool calls,
//! appends outputs, and repeats until a response has no tool calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::ModelProviderInfo;
use crate::tools::{Tool, ToolCall};

mod driver;
mod exec;
pub mod prompt;
pub mod tool_text;
mod view;
pub use driver::{
    looks_like_context_overflow, model_configured, SaveFailed, Session, Turn, TurnRefused,
};
pub use view::{view_items, ToolStatus, ViewItem};

/// One transcript item. Serialized verbatim into session JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        args: Value,
    },
    ToolOutput {
        call_id: String,
        output: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// Events the agent worker sends to the UI thread. Items are emitted
/// one by one AS THEY COMPLETE (codex appends to the rollout as it
/// goes) - a crash or quit mid-turn loses at most the item in flight,
/// and the final events carry no payload to re-sync.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ev {
    AssistantDelta {
        text: String,
    },
    /// One assistant message finished (there may be several per turn
    /// when tool rounds intervene). UI-only; the matching ItemDone
    /// carries the same text for persistence.
    AssistantDone {
        text: String,
    },
    ToolBegin {
        call_id: String,
        name: String,
        args: Value,
    },
    ToolEnd {
        call_id: String,
        output: String,
        ok: bool,
    },
    /// One transcript item is final - record it (history + session
    /// JSONL) immediately.
    ItemDone {
        item: Item,
    },
    /// Turn ended normally. `usage`: what its last call read and
    /// everything its calls wrote.
    TurnComplete {
        ms: u64,
        provider: String,
        usage: Usage,
    },
    /// User interrupted (cancel flag). A <turn_aborted> marker item
    /// has already been emitted via ItemDone.
    TurnAborted,
    Failed {
        error: String,
    },
}

/// Executes one tool call (exec.rs runs them over snapshot data).
/// Errors return Err(message) and become ToolOutput{is_error} - the
/// model sees them and recovers; they never kill the turn.
pub trait ToolExec: Send {
    fn run(&self, name: &str, args: &Value) -> std::result::Result<String, String>;
}

/// pi's output caps: whichever is hit first wins, never split a line.
const MAX_OUTPUT_LINES: usize = 2000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES && s.lines().count() <= MAX_OUTPUT_LINES {
        return s.to_string();
    }
    let mut out = String::new();
    let mut kept = 0usize;
    for line in s.lines() {
        if kept >= MAX_OUTPUT_LINES || out.len() + line.len() + 1 > MAX_OUTPUT_BYTES {
            break;
        }
        out.push_str(line);
        out.push('\n');
        kept += 1;
    }
    out.push_str(&format!(
        "[truncated: kept first {} of {} lines]",
        kept,
        s.lines().count()
    ));
    out
}

/// Marker appended after an interrupt so the next turn's model knows
/// the previous one ended early (codex's turn_aborted contract).
pub const TURN_ABORTED_MARKER: &str = "<turn_aborted>The user interrupted the previous turn. \
Tools may have partially executed.</turn_aborted>";

/// Prompt-build healing (codex invariant 1): every ToolCall must have a
/// ToolOutput before the next request. Synthesizes "aborted" outputs.
pub fn heal_history(history: &mut Vec<Item>) {
    let answered: std::collections::HashSet<String> = history
        .iter()
        .filter_map(|i| match i {
            Item::ToolOutput { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect();
    let missing: Vec<(usize, String)> = history
        .iter()
        .enumerate()
        .filter_map(|(idx, i)| match i {
            Item::ToolCall { call_id, .. } if !answered.contains(call_id) => {
                Some((idx, call_id.clone()))
            }
            _ => None,
        })
        .collect();
    // Insert right after each dangling call, back to front to keep indices valid.
    for (idx, call_id) in missing.into_iter().rev() {
        history.insert(
            idx + 1,
            Item::ToolOutput {
                call_id,
                output: "aborted before completion".into(),
                is_error: true,
            },
        );
    }
}

/// One model request: full history in, streamed deltas out via the
/// callback, text + tool calls back. This is the seam that makes the
/// loop testable without a network (pi injects streamFn the same way);
/// production passes a closure over `llm::stream_turn`.
pub type StreamFn<'a> =
    &'a dyn Fn(&[Item], &mut dyn FnMut(&str)) -> crate::error::Result<TurnResponse>;

/// Run one full turn on the calling (worker) thread: repeat model
/// request -> execute tool calls -> append outputs, until a response
/// has no tool calls (pi's loop, no iteration cap). Sends Ev's as it
/// goes and finishes with TurnComplete / TurnAborted / Failed.
#[allow(clippy::too_many_arguments)]
pub fn run_turn(
    provider_id: &str,
    provider: &ModelProviderInfo,
    model: &str,
    system: &str,
    history: Vec<Item>,
    tools: &[Tool],
    exec: &dyn ToolExec,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Ev>,
) {
    let stream: StreamFn = &|history, on_delta| {
        crate::llm::stream_turn(
            provider_id,
            provider,
            model,
            system,
            history,
            tools,
            cancel,
            on_delta,
        )
    };
    run_turn_with(stream, provider_id, history, exec, cancel, tx)
}

/// The loop body behind the stream seam. Tests drive it with a
/// scripted StreamFn and a fake ToolExec.
fn run_turn_with(
    stream: StreamFn,
    provider_id: &str,
    mut history: Vec<Item>,
    exec: &dyn ToolExec,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Ev>,
) {
    let started = std::time::Instant::now();
    // New items are appended to the local prompt history AND emitted as
    // Ev::ItemDone the moment they are final (codex's rollout rule) -
    // the UI records each immediately, so a crash mid-turn loses at
    // most the item in flight.
    let emit = |history: &mut Vec<Item>, item: Item| {
        history.push(item.clone());
        let _ = tx.send(Ev::ItemDone { item });
    };
    // Request-building healing only (never emitted): dangling ToolCalls
    // from a prior aborted turn get synthesized outputs so every wire
    // request satisfies invariant 1. Within a turn no new dangling call
    // can appear - each ToolCall is answered inline or the loop
    // returns - so once is enough.
    heal_history(&mut history);
    let mut usage = Usage::default();
    loop {
        let resp = stream(&history, &mut |d| {
            let _ = tx.send(Ev::AssistantDelta {
                text: d.to_string(),
            });
        });
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                if cancel.load(Ordering::SeqCst) {
                    emit(
                        &mut history,
                        Item::User {
                            text: TURN_ABORTED_MARKER.into(),
                        },
                    );
                    let _ = tx.send(Ev::TurnAborted);
                } else {
                    let _ = tx.send(Ev::Failed {
                        error: e.to_string(),
                    });
                }
                return;
            }
        };
        // Each call re-reads the whole history: the last one's input is
        // the context's size, not their sum.
        usage = Usage {
            input: resp.usage.input,
            output: usage.output + resp.usage.output,
        };
        let mut text = resp.text;
        if resp.truncated && resp.tool_calls.is_empty() {
            // Length-stopped plain answer: never present it as complete.
            let note = "\n[response cut off: the model hit its max_tokens limit]";
            let _ = tx.send(Ev::AssistantDelta {
                text: note.to_string(),
            });
            text.push_str(note);
        }
        if !text.is_empty() {
            emit(&mut history, Item::Assistant { text: text.clone() });
            let _ = tx.send(Ev::AssistantDone { text });
        }
        if resp.truncated && !resp.tool_calls.is_empty() {
            // pi's contract: a length stop fails ALL tool calls - the
            // args may be cut mid-JSON (parse_args would run the tool
            // with silently wrong args). Feed errors back and let the
            // model retry.
            for call in resp.tool_calls {
                emit(
                    &mut history,
                    Item::ToolCall {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        args: call.arguments.clone(),
                    },
                );
                let output = "not executed: the response hit the max_tokens limit \
                              mid-call, so the arguments may be truncated - re-issue \
                              the call"
                    .to_string();
                emit(
                    &mut history,
                    Item::ToolOutput {
                        call_id: call.id.clone(),
                        output: output.clone(),
                        is_error: true,
                    },
                );
                let _ = tx.send(Ev::ToolBegin {
                    call_id: call.id.clone(),
                    name: call.name,
                    args: call.arguments,
                });
                let _ = tx.send(Ev::ToolEnd {
                    call_id: call.id,
                    output: tool_text::for_people(&output),
                    ok: false,
                });
            }
            continue;
        }
        if resp.tool_calls.is_empty() {
            let _ = tx.send(Ev::TurnComplete {
                ms: started.elapsed().as_millis() as u64,
                provider: provider_id.to_string(),
                usage,
            });
            return;
        }
        for call in resp.tool_calls {
            emit(
                &mut history,
                Item::ToolCall {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args: call.arguments.clone(),
                },
            );
            if cancel.load(Ordering::SeqCst) {
                break; // next turn's heal_history synthesizes the aborted output
            }
            let _ = tx.send(Ev::ToolBegin {
                call_id: call.id.clone(),
                name: call.name.clone(),
                args: call.arguments.clone(),
            });
            let (output, ok) = match exec.run(&call.name, &call.arguments) {
                Ok(o) => (truncate_output(&o), true),
                Err(e) => (e, false),
            };
            emit(
                &mut history,
                Item::ToolOutput {
                    call_id: call.id.clone(),
                    output: output.clone(),
                    is_error: !ok,
                },
            );
            // The history keeps what the model read; the event, what a person sees.
            let _ = tx.send(Ev::ToolEnd {
                call_id: call.id,
                output: tool_text::for_people(&output),
                ok,
            });
        }
        if cancel.load(Ordering::SeqCst) {
            emit(
                &mut history,
                Item::User {
                    text: TURN_ABORTED_MARKER.into(),
                },
            );
            let _ = tx.send(Ev::TurnAborted);
            return;
        }
    }
}

/// Tokens a model call read and wrote, as the provider reported them
/// (zero when it reports none). Input counts the whole prompt, cached
/// parts included: it is how full the context is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

/// What one model request returned: streamed text plus any tool calls.
#[derive(Debug, Default)]
pub struct TurnResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    /// The model hit its max_tokens budget (stop_reason max_tokens /
    /// finish_reason length): text is incomplete and any tool-call args
    /// may be cut mid-JSON - they must not be executed as-is.
    pub truncated: bool,
    pub usage: Usage,
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
