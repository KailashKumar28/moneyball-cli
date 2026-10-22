//! Starting an advisor turn: the system prompts, the live app state
//! every prompt carries, the turn a typed command opens, and the worker
//! that runs the agent loop. Split from commands.rs (size cap).

use crate::commands::COMMANDS;
use crate::*;

/// System prompt for the free-form chat agent. Same persona, but the
/// user is asking a free-form question rather than triggering /brief.
const AGENT_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You have three tools that read the on-disk snapshot: brief (7-day per-product summary plus feasibility math), funnel (7-day table for one product, per adset, campaign or ad) and diagnose (for one product: where its CRM leads die, tired ads, the last 7 days against the days before, what changed in its ad account). Call them whenever the user asks about portfolio numbers - never answer from memory or invent figures. Before recommending a cut, pause or kill, call diagnose for that product.\n\
If a tool reports missing data (no snapshot, no CRM), relay its suggested fix to the user instead of guessing.\n\
Keep the answer focused and concrete. Cite the numbers you use. 3-6 sentences unless the user explicitly asks for more.";

pub(crate) const FUNNEL_SYSTEM_PROMPT: &str = "You are moneyball, a Meta-ads portfolio advisor. \
You are given a per-entity funnel table (spend, Meta leads m, CRM leads l, qualified q, \
visits v, Rs/qualified, kill flags). Give a per-entity read: SCALE (efficient + sufficient \
volume), KILL (kill flag true and not immature), or WAIT (immature, learning, or not enough \
spend). Cite the numbers that justify each call. Never recommend killing an immature or \
learning entity. kill = spend passed the kill table with <=2 qualified; immature = leads \
still inside the 72h maturation lag. Before any KILL, call the diagnose tool for this product \
and say what it shows. 3-8 sentences, no preamble.";

/// Live app state injected into every LLM system prompt so the model
/// never hallucinates about setup/data (codex + claude code both ground
/// their agents in real environment state for exactly this reason).
pub(crate) fn app_state_block(app: &App) -> String {
    let mut s = String::from("== live app state (authoritative - never contradict this) ==\n");
    match app.cfg.workspace.as_ref() {
        Some(w) => {
            s.push_str(&format!(
                "workspace: {} (setup is COMPLETE - do not suggest /setup for missing data)\n",
                app.cfg.data_root.display()
            ));
            let prods: Vec<String> = w
                .products
                .iter()
                .map(|p| format!("{} (goal {}/day)", p.name, w.goal_for(&p.name)))
                .collect();
            s.push_str(&format!(
                "products ({}): {}\n",
                prods.len(),
                prods.join(", ")
            ));
            s.push_str(
                "When you name a product or suggest a command, use these product \
names EXACTLY as listed - suggested commands must run verbatim when typed.\n",
            );
        }
        None => s.push_str("workspace: NOT configured - /setup is the fix\n"),
    }
    match &app.snap_date {
        Some(d) => {
            s.push_str(&format!("snapshot: {} loaded\n", d));
            if let Some(note) = &app.crm_note {
                s.push_str(&format!("{}\n", note));
            }
        }
        None => s.push_str(
            "snapshot: none yet. /brief self-heals by running /refresh when a token \
             is configured; otherwise an external pipeline can write \
             <workspace>/.moneyball/history/snap/<YYYY-MM-DD>/*.json. \
             /setup does NOT create snapshots - never suggest it for missing data.\n",
        ),
    }
    let cmds: Vec<&str> = COMMANDS.iter().map(|(c, _)| *c).collect();
    s.push_str(&format!("slash commands: {}\n", cmds.join(" ")));
    s.push_str(
        "Output style: plain terminal prose. **bold** and `code` render styled; \
         tables, links and nested lists do not - avoid them. When citing commands, \
         use product names verbatim including spaces (e.g. /funnel Namma Mane) - \
         never slugify them.\n",
    );
    s
}

/// The turn a typed command opens: what the user typed, then the tool
/// call it stands for with that tool's output, so the model reads the
/// numbers as a tool result and never calls the tool again for them.
pub(crate) fn typed_tool_turn(
    app: &App,
    typed: &str,
    tool: &str,
    args: serde_json::Value,
    output: String,
) -> Vec<moneyball_core::agent::Item> {
    use moneyball_core::agent::Item;
    let call_id = format!("typed-{}", app.history.len());
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
/// stands for) and run the agent on it.
pub(crate) fn call_agent(app: &mut App, system: &str, turn: Vec<moneyball_core::agent::Item>) {
    use crate::chat::cells;
    use crate::chat::Cell;
    let (provider_id, model_providers, model) = match app.cfg.workspace.as_ref() {
        Some(w) => (
            w.model_provider.clone().unwrap_or_default(),
            w.model_providers.clone(),
            w.model.clone().unwrap_or_default(),
        ),
        None => (String::new(), Default::default(), String::new()),
    };
    if provider_id.is_empty() || model.is_empty() {
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: "no LLM configured - run /setup (step 4) to pick a provider + paste a key."
                .into(),
            streaming: false,
        }));
        return;
    }
    let provider_info = match model_providers.get(&provider_id) {
        Some(p) => p,
        None => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!(
                    "configured provider '{}' is not in the model_providers map - re-run /setup.",
                    provider_id
                ),
                streaming: false,
            }));
            return;
        }
    };
    // Agent turn (ARCHITECTURE.md 6b): record the user item, then run
    // the tool loop on a worker thread. Ev's arrive via the tick drain;
    // the UI never blocks and Esc cancels via the shared flag.
    if app.stream.is_some() {
        app.chat
            .note("still responding - wait for it to finish (esc interrupts), then resend");
        return;
    }
    for item in turn {
        app.record(item);
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: String::new(),
        streaming: true,
    }));
    app.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let pid = provider_id.clone();
    let pinfo = provider_info.clone();
    let model = model.clone();
    let sys = system.to_string();
    let history = app.history.clone();
    let cfg = app.cfg.clone();
    let cancel = app.cancel.clone();
    std::thread::spawn(move || {
        // Bridge agent Ev's into the app's StreamEvent channel.
        let (etx, erx) = std::sync::mpsc::channel::<moneyball_core::agent::Ev>();
        let fwd = std::thread::spawn(move || {
            for ev in erx {
                if tx.send(StreamEvent::Agent(ev)).is_err() {
                    break; // UI dropped the receiver
                }
            }
        });
        let exec = crate::agent_tools::SnapshotTools { cfg };
        // Only implemented tools go on the wire - the model must never
        // be steered into stubs (audit F4).
        let tools = vec![
            moneyball_core::tools::brief_tool(),
            moneyball_core::tools::funnel_tool(),
            moneyball_core::tools::diagnose_tool(),
        ];
        moneyball_core::agent::run_turn(
            &pid, &pinfo, &model, &sys, history, &tools, &exec, &cancel, &etx,
        );
        drop(etx);
        let _ = fwd.join();
    });
    app.stream = Some(rx);
    app.turn_active = true;
    app.turn_started = std::time::Instant::now();
}

/// Free-form chat: plain text in the input bar is a question for the
/// advisor, which pulls its numbers through its tools (the LLM is the
/// default, as in codex and Claude Code).
pub(crate) fn run_freeform(app: &mut App, question: &str) {
    // No pre-stuffed context: the question goes in verbatim.
    let sys = format!("{}\n\n{}", AGENT_SYSTEM_PROMPT, app_state_block(app));
    let turn = vec![moneyball_core::agent::Item::User {
        text: question.to_string(),
    }];
    call_agent(app, &sys, turn);
}
