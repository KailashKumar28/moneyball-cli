//! The advisor session a front-end drives: the transcript, its session
//! file, the cancel flag and the one-turn-at-a-time guard (ARCHITECTURE.md
//! 6b). The front-end owns a `Session` on its own thread, starts turns on
//! it, and feeds it every event the turn's worker sends back.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use super::{exec, prompt, Ev, Item};
use crate::config::AppConfig;
use crate::provider::ModelProviderInfo;
use crate::session::SessionLog;

/// Why a turn did not start. Nothing was recorded; each front-end says
/// it in its own words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnRefused {
    #[error("no model is configured")]
    NoModel,
    #[error("the configured provider '{provider}' is not in model_providers")]
    UnknownProvider { provider: String },
    #[error("a turn is already running")]
    Busy,
}

/// An item is in the history but did not reach the session file. The
/// conversation goes on; the front-end shows this once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[error("session save failed: {0}")]
pub struct SaveFailed(pub String);

/// A started turn.
#[derive(Debug)]
pub struct Turn {
    /// The worker's events. Feed each one to `Session::apply`; if the
    /// channel closes with no final event, call `Session::stream_closed`.
    pub events: Receiver<Ev>,
    /// The turn's own items were not saved (the turn runs regardless).
    pub save_failed: Option<SaveFailed>,
}

/// One conversation: the transcript in wire format (the prompt of every
/// turn), its append-only session file, and the turn in flight.
pub struct Session {
    history: Vec<Item>,
    /// None = not persisted (the file could not be created, or tests).
    log: Option<SessionLog>,
    /// Shared with the turn's worker, which aborts between SSE events.
    cancel: Arc<AtomicBool>,
    turn_active: bool,
}

impl Session {
    /// A fresh conversation (`history` empty) or a resumed one
    /// (`SessionLog::open` gives both parts).
    pub fn new(log: Option<SessionLog>, history: Vec<Item>) -> Self {
        Self {
            history,
            log,
            cancel: Arc::new(AtomicBool::new(false)),
            turn_active: false,
        }
    }

    pub fn history(&self) -> &[Item] {
        &self.history
    }

    /// The session file's id; None when the session is not persisted.
    pub fn id(&self) -> Option<&str> {
        self.log.as_ref().map(|log| log.meta.id.as_str())
    }

    /// True from `start_turn` until the turn's final event is applied.
    pub fn turn_active(&self) -> bool {
        self.turn_active
    }

    /// Append one item to the history AND the session file. The item is
    /// in the history either way; Err says the file missed it.
    pub fn record(&mut self, item: Item) -> Result<(), SaveFailed> {
        let saved = match &self.log {
            Some(log) => log.append(&item).map_err(|e| SaveFailed(e.to_string())),
            None => Ok(()),
        };
        self.history.push(item);
        saved
    }

    /// The turn a typed command opens: what the user typed, then the tool
    /// call it stands for with that tool's output, so the model reads the
    /// numbers as a tool result and never calls the tool again for them.
    pub fn typed_tool_turn(
        &self,
        typed: &str,
        tool: &str,
        args: Value,
        output: String,
    ) -> Vec<Item> {
        let call_id = format!("typed-{}", self.history.len());
        vec![
            Item::User {
                text: typed.to_string(),
            },
            Item::ToolCall {
                call_id: call_id.clone(),
                name: tool.to_string(),
                args,
            },
            Item::ToolOutput {
                call_id,
                output,
                is_error: false,
            },
        ]
    }

    /// Record `turn` (the user's item, and any tool round a typed command
    /// stands for) and run the agent on the whole history, on a worker
    /// thread. `system` is a persona prompt plus `prompt::app_state_block`.
    /// A refusal records nothing and starts nothing.
    pub fn start_turn(
        &mut self,
        cfg: &AppConfig,
        system: &str,
        turn: Vec<Item>,
    ) -> Result<Turn, TurnRefused> {
        let (provider_id, provider, model) = model_for(cfg)?;
        if self.turn_active {
            return Err(TurnRefused::Busy);
        }
        let mut save_failed = None;
        for item in turn {
            save_failed = self.record(item).err().or(save_failed);
        }
        self.cancel.store(false, Ordering::SeqCst);
        let (tx, events) = std::sync::mpsc::channel();
        let system = format!("{}\n\n{}", system, prompt::RECORD_RULE);
        let history = self.history.clone();
        let (cfg, cancel) = (cfg.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            let exec = exec::SnapshotTools { cfg };
            let tools = exec::tools();
            super::run_turn(
                &provider_id,
                &provider,
                &model,
                &system,
                history,
                &tools,
                &exec,
                &cancel,
                &tx,
            );
        });
        self.turn_active = true;
        Ok(Turn {
            events,
            save_failed,
        })
    }

    /// Take in one event of the turn in flight: a finished item is
    /// recorded the moment it arrives (a crash mid-turn loses at most
    /// the item in flight), and a final event frees the session for the
    /// next turn.
    pub fn apply(&mut self, ev: &Ev) -> Result<(), SaveFailed> {
        match ev {
            Ev::ItemDone { item } => return self.record(item.clone()),
            Ev::TurnComplete { .. } | Ev::TurnAborted | Ev::Failed { .. } => {
                self.turn_active = false;
            }
            Ev::AssistantDelta { .. }
            | Ev::AssistantDone { .. }
            | Ev::ToolBegin { .. }
            | Ev::ToolEnd { .. } => {}
        }
        Ok(())
    }

    /// The event channel closed with no final event (the worker died):
    /// free the session so the next turn can start.
    pub fn stream_closed(&mut self) {
        self.turn_active = false;
        self.cancel.store(false, Ordering::SeqCst);
    }

    /// Ask the turn in flight to stop; false when there is none. The
    /// worker ends it with `Ev::TurnAborted` once it notices.
    pub fn interrupt(&self) -> bool {
        if self.turn_active {
            self.cancel.store(true, Ordering::SeqCst);
        }
        self.turn_active
    }
}

/// The workspace's chosen provider and model.
fn model_for(cfg: &AppConfig) -> Result<(String, ModelProviderInfo, String), TurnRefused> {
    let w = cfg.workspace.as_ref().ok_or(TurnRefused::NoModel)?;
    let provider_id = w.model_provider.clone().unwrap_or_default();
    let model = w.model.clone().unwrap_or_default();
    if provider_id.is_empty() || model.is_empty() {
        return Err(TurnRefused::NoModel);
    }
    match w.model_providers.get(&provider_id) {
        Some(provider) => Ok((provider_id, provider.clone(), model)),
        None => Err(TurnRefused::UnknownProvider {
            provider: provider_id,
        }),
    }
}

/// Provider error text (`Ev::Failed`) that smells like a context-window
/// overflow: every later turn of this session would fail the same way,
/// so the front-end should offer a fresh one. Wording varies per
/// provider; these substrings cover Anthropic, OpenAI-compatible, and
/// MiniMax phrasings.
pub fn looks_like_context_overflow(error: &str) -> bool {
    let e = error.to_lowercase();
    [
        "context length",
        "context window",
        "prompt is too long",
        "maximum context",
        "token limit",
    ]
    .iter()
    .any(|p| e.contains(p))
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
