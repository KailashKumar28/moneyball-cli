//! Starting an advisor turn from the TUI: what this front-end tells the
//! model about itself, and how it shows a turn that did not start. The
//! session, the prompts and the worker live in moneyball_core::agent.

use crate::commands::COMMANDS;
use crate::*;
use moneyball_core::agent::{prompt, Item, TurnRefused};

/// How the chat view renders the model's text (markdown.rs).
const OUTPUT_STYLE: &str =
    "Output style: plain terminal prose. **bold** and `code` render styled; \
     tables, links and nested lists do not - avoid them. When citing commands, \
     use product names verbatim including spaces (e.g. /funnel Namma Mane) - \
     never slugify them.";

/// This front-end in the model's eyes: its commands, its rendering
/// rule, and its words for the steps core's prompts and tools name.
fn front_end<'a>(commands: &'a [&'a str]) -> prompt::FrontEnd<'a> {
    prompt::FrontEnd {
        commands,
        output_style: OUTPUT_STYLE,
        refresh: "/refresh",
        setup: "/setup",
        brief: "/brief",
        mark_applied: "type /ledger {id} applied",
    }
}

fn commands() -> Vec<&'static str> {
    COMMANDS.iter().map(|(c, _)| *c).collect()
}

/// A persona prompt plus the live app state, as this front-end has it
/// loaded.
pub(crate) fn system_prompt(app: &App, persona: &str) -> String {
    let state = prompt::app_state_block(
        &app.cfg,
        app.snap_date.as_deref(),
        app.crm_note.as_deref(),
        &front_end(&commands()),
    );
    format!("{}\n\n{}", persona, state)
}

/// Record `turn` (the user's item, and any tool round a typed command
/// stands for) and run the agent on it.
pub(crate) fn call_agent(app: &mut App, system: &str, turn: Vec<Item>) {
    use crate::chat::cells;
    use crate::chat::Cell;
    // A /report holds the stream slot too (submit already holds input
    // while it is taken).
    if app.stream.is_some() {
        return still_responding(app);
    }
    // Agent turn (ARCHITECTURE.md 6b): the session records the user
    // item, then runs the tool loop on a worker thread. Ev's arrive via
    // the tick drain; the UI never blocks and Esc interrupts.
    let commands = commands();
    let front = front_end(&commands);
    let turn = match app.session.start_turn(&app.cfg, &front, system, turn) {
        Ok(turn) => turn,
        Err(TurnRefused::Busy) => return still_responding(app),
        Err(refused) => {
            let text = match refused {
                TurnRefused::UnknownProvider { provider } => format!(
                    "configured provider '{}' is not in the model_providers map - re-run /setup.",
                    provider
                ),
                _ => "no LLM configured - run /setup (step 4) to pick a provider + paste a key."
                    .into(),
            };
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text,
                streaming: false,
            }));
            return;
        }
    };
    if let Some(e) = turn.save_failed {
        app.status = Some(e.to_string());
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: String::new(),
        streaming: true,
    }));
    // Bridge the turn's Ev's into the app's StreamEvent channel.
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    std::thread::spawn(move || {
        for ev in turn.events {
            if tx.send(StreamEvent::Agent(ev)).is_err() {
                break; // UI dropped the receiver
            }
        }
    });
    app.stream = Some(rx);
    app.turn_started = std::time::Instant::now();
    // A tool left "running" by a worker that died must not label this turn.
    app.running_tool = None;
}

fn still_responding(app: &mut App) {
    app.chat
        .note("still responding - wait for it to finish (esc interrupts), then resend");
}

/// Free-form chat: plain text in the input bar is a question for the
/// advisor, which pulls its numbers through its tools (the LLM is the
/// default, as in codex and Claude Code).
pub(crate) fn run_freeform(app: &mut App, question: &str) {
    // No pre-stuffed context: the question goes in verbatim.
    let sys = system_prompt(app, prompt::AGENT_SYSTEM_PROMPT);
    let turn = vec![Item::User {
        text: question.to_string(),
    }];
    call_agent(app, &sys, turn);
}
