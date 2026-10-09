//! Slash-command surface: registry, dispatch (`submit`), and the LLM
//! call paths (brief, free-form chat, streaming agent).

use crate::*;

// CLAUDE.md hard rule: only commands with real handlers may appear
// here - this list feeds the palette, /help, AND every LLM system
// prompt (app_state_block). Re-add /ledger in the same commit that
// wires it.
pub(crate) const COMMANDS: &[(&str, &str)] = &[
    ("/brief", "7-day portfolio brief"),
    (
        "/refresh",
        "pull Meta, then CRM, then write yesterday's report",
    ),
    ("/status", "the last 14 data days and how each refresh went"),
    ("/schedule", "refresh every morning; /schedule off stops it"),
    ("/funnel", "per-entity funnel for a product"),
    (
        "/diagnose",
        "before cutting a product: where leads die, tired ads, trend, changes",
    ),
    (
        "/report",
        "creative report (per-creative funnel) as HTML + summary",
    ),
    ("/ask", "free-form question (LLM picks commands)"),
    (
        "/debug",
        "flag this session as buggy for review (optional comment)",
    ),
    (
        "/crm",
        "CRM status: spec, secret, data freshness, next step",
    ),
    ("/snapshot", "list or validate snapshots"),
    ("/clear", "start a fresh conversation (new session file)"),
    (
        "/connect",
        "where ads, leads and the advisor stand; /connect crm connects one",
    ),
    (
        "/setup",
        "re-run the setup wizard; /setup meta replaces the Meta token",
    ),
    ("/help", "list slash commands"),
    ("/quit", "exit moneyball"),
];

pub(crate) fn completions(prefix: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| c.starts_with(prefix))
        .collect()
}

pub(crate) fn submit(app: &mut App) {
    use crate::chat::cells;
    use crate::chat::Cell;
    use chrono::Local;
    // One response at a time: while a stream is in flight, hold new
    // submissions (esc interrupts, like codex).
    if app.stream.is_some() {
        app.status = Some("still responding - esc to interrupt, then resend".into());
        return;
    }
    let line = app.input.trim().to_string();
    // A second /quit confirms; anything else in between disarms it.
    let quit_armed = std::mem::take(&mut app.quit_armed);
    app.input.clear();
    app.cursor = 0;
    app.completions.clear();
    app.completion_idx = None;
    // Clear any stale status hint from a prior command/error so the
    // bottom status line doesn't keep showing the previous result.
    app.status = None;
    if line.is_empty() {
        return;
    }

    // User prompt cell (every input becomes part of the scrollback).
    app.chat.push(Cell::UserPrompt(cells::UserPrompt {
        text: line.clone(),
        at: Local::now(),
    }));

    let mut parts = line.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "/quit" | "/exit" | "/q" => {
            if app.refresh.is_some() && !quit_armed {
                app.quit_armed = true;
                app.chat.note(
                    "a refresh is still running - /quit again to stop it and leave \
                     (its run record keeps the steps that finished)",
                );
                return;
            }
            app.chat.note("exiting moneyball.");
            app.quit = true;
        }
        "/brief" => crate::brief_flow::run_brief(app),
        // /fetch is the old name, kept for muscle memory (unlisted).
        "/status" => crate::status_view::run_status(app),
        "/schedule" => crate::schedule_view::run_schedule(app, arg),
        "/refresh" | "/fetch" => crate::refresh_flow::run_refresh(app, arg == "--force", false),
        "/setup" => {
            // Prefilled from the saved config, so /setup re-runs as "edit
            // current settings" (Enter keeps values).
            let mut state = match app.cfg.workspace.as_ref() {
                Some(w) => SetupState::prefilled_from(w, &app.cfg.data_root),
                None => SetupState::new(app.cfg.data_root.clone()),
            };
            // `/setup meta`: straight to the token paste, nothing else
            // re-asked (a configured workspace otherwise opens at the model).
            // The saved token shows as bullets: enter re-checks it against
            // every product, typing replaces it.
            let token_only = arg == "meta" && !state.products.is_empty();
            if token_only {
                state.step = 1;
                state.meta_substep = 0;
                state.token_only = true;
                state.meta_input = "\u{2022}".repeat(state.meta_token_len);
            }
            app.chat.note(if token_only {
                "replacing the Meta token: paste the new one, or enter to re-check the saved one."
            } else {
                "opening setup wizard (Enter to keep current values, edit + Enter to change)."
            });
            app.view = View::Setup(state);
        }
        "/funnel" => {
            crate::funnel_flow::run_funnel(app, arg);
        }
        "/diagnose" => {
            crate::diagnose_flow::run_diagnose(app, arg);
        }
        "/debug" => {
            crate::report::run_debug_report(app, arg);
        }
        "/report" => {
            crate::creative_flow::run_report(app, arg);
        }
        "/crm" => {
            crate::crm_flow::run_crm_status(app);
        }
        "/ask" => {
            // /ask is now equivalent to free-form chat (the LLM is the
            // default). Strip the prefix and fall through to the free-
            // form path below.
            let question = if arg.is_empty() {
                line.clone()
            } else {
                arg.to_string()
            };
            run_freeform(app, &question);
        }
        "/snapshot" => {
            // List available snapshot dates and validate the latest one
            // parses against the schema.
            let started = std::time::Instant::now();
            let snap_root = app.cfg.snap_dir();
            let dates = moneyball_core::snapshot::list_dates(&snap_root).unwrap_or_default();
            if dates.is_empty() {
                app.chat.push_tool(
                    "snapshot",
                    "",
                    vec![format!(
                        "no snapshots in {} - run /refresh to pull one.",
                        snap_root.display()
                    )],
                    false,
                    started.elapsed().as_millis() as u64,
                );
            } else {
                let latest = dates.last().cloned().unwrap_or_default();
                let mut out: Vec<String> = vec![format!(
                    "{} snapshot(s) in {}:",
                    dates.len(),
                    snap_root.display()
                )];
                out.extend(dates.iter().map(|d| format!("  {}", d)));
                match moneyball_core::snapshot::load(&snap_root.join(&latest)) {
                    Ok(s) => out.push(format!(
                        "latest {} validates: {} ads_daily rows",
                        latest,
                        s.ads_daily.len()
                    )),
                    Err(e) => out.push(format!("latest {} FAILS validation: {}", latest, e)),
                }
                app.chat.push_tool(
                    "snapshot",
                    "",
                    out,
                    true,
                    started.elapsed().as_millis() as u64,
                );
            }
        }
        "/clear" | "/new" => {
            // Escape hatch for context growth (codex /new): fresh
            // history + fresh session file. The old session stays on
            // disk untouched (append-only, resumable via -c or --resume).
            if app.turn_active || app.stream.is_some() {
                app.chat
                    .note("still responding - esc to interrupt first, then /clear");
                return;
            }
            // Its answers and result land in its cell: keep it until done.
            if app.connect.is_some() {
                app.chat
                    .note("a CRM connect is running - finish or esc it, then /clear");
                return;
            }
            app.history.clear();
            // A refresh still running keeps its cell: its outcome and any
            // fix land there.
            app.chat
                .cells
                .retain(|c| matches!(c, Cell::Refresh(r) if r.end.is_none()));
            app.chat.scroll_to_bottom();
            match moneyball_core::session::SessionLog::create(
                app.cfg.data_root.clone(),
                app.cfg.sessions_root(),
            ) {
                Ok(log) => {
                    app.session = Some(log);
                    app.chat.push(Cell::System(cells::System(
                        "fresh conversation - previous session saved on disk (resume with \
                         moneyball --continue)"
                            .into(),
                    )));
                }
                Err(e) => {
                    app.session = None;
                    app.chat.push(Cell::System(cells::System(format!(
                        "fresh conversation - but the new session log is unavailable ({})",
                        e
                    ))));
                }
            }
        }
        // /keychain is the old name, kept for muscle memory (unlisted).
        "/connect" | "/keychain" if arg == "crm" => crate::crm_connect::run(app),
        "/connect" | "/keychain" => crate::connect_view::run_connect(app),
        "/help" | "/?" => {
            // Generated from COMMANDS so the three command surfaces
            // (palette, /help, app_state_block) cannot drift.
            let mut text = String::from("slash commands:");
            for (c, desc) in COMMANDS {
                text.push_str(&format!("\n  {:<10} {}", c, desc));
            }
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text,
                streaming: false,
            }));
        }
        _ => {
            // A slash typo must never reach the paid LLM path - reject
            // locally and instantly (codex/Claude Code behavior).
            if cmd.starts_with('/') {
                app.chat.push(Cell::System(cells::System(format!(
                    "unknown command {} - type / to see the command list.",
                    cmd
                ))));
                return;
            }
            // Free-form chat: the LLM is the default; slash commands are
            // optional shortcuts. The agent pulls data via its tools.
            run_freeform(app, &line);
        }
    }
}

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

/// Look up the active provider + model from `app.cfg.workspace` and run
/// a single non-streaming completion. Pushes the response (or a red
/// error cell) directly into `app.chat`. Returns the elapsed ms in
/// the success case, or `None` if the call could not be made (no
/// provider configured, runtime init failed, etc.). Used by /brief
/// AND by the free-form chat fallback.
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
}

/// Free-form chat path. Tries to load the brief silently so the LLM has
/// portfolio context; if no snapshot exists, we still answer with just
/// the user's question. The slash commands (`/brief`, `/funnel`, etc.)
/// are convenience shortcuts that bypass the LLM dispatch and emit a
/// tool result directly. Plain text in the input bar falls into this
/// function - the LLM is the default, mirroring how codex / claude
/// code work.
fn run_freeform(app: &mut App, question: &str) {
    // The agent loop pulls numbers itself via the brief/funnel tools -
    // no pre-stuffed context, the question goes in verbatim.
    let sys = format!("{}\n\n{}", AGENT_SYSTEM_PROMPT, app_state_block(app));
    let turn = vec![moneyball_core::agent::Item::User {
        text: question.to_string(),
    }];
    call_agent(app, &sys, turn);
}

// ---------- setup-wizard keys ----------
