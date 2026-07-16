//! Slash-command surface: registry, dispatch (`submit`), and the LLM
//! call paths (fetch, free-form chat, streaming agent).

use crate::*;

use moneyball_core::brief::{self};

// Two chrono types collide on import name; alias the plain Utc.

pub(crate) const COMMANDS: &[(&str, &str)] = &[
    ("/brief", "7-day portfolio brief"),
    ("/fetch", "pull daily insights from Meta into a snapshot"),
    ("/funnel", "per-entity funnel for a product"),
    ("/diagnose", "run all 5 diagnostic commands for a product"),
    ("/ask", "free-form question (LLM picks commands)"),
    ("/snapshot", "list or validate snapshots"),
    ("/ledger", "prediction ledger view"),
    ("/setup", "re-run the setup wizard"),
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
            app.chat
                .push(Cell::System(cells::System("exiting moneyball.".into())));
            app.quit = true;
        }
        "/brief" => {
            let started = std::time::Instant::now();
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "loading portfolio snapshot...".into(),
                streaming: false,
            }));
            app.load_brief();
            // If brief loaded, push the deterministic numbers as a tool
            // call (so the user can see the source data), then call the
            // configured LLM for an interpretation.
            if let Some(b) = &app.brief {
                let out = format_brief_as_lines(b);
                let user_prompt = build_brief_prompt(b);
                app.chat
                    .push_tool("brief", "", out, true, started.elapsed().as_millis() as u64);
                let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
                call_agent(app, &sys, &user_prompt);
            } else if moneyball_core::secrets::load_meta_token().is_some()
                && app.cfg.has_workspace()
            {
                // No snapshot but we CAN get one ourselves: self-heal by
                // fetching from Meta, then show the brief (run_fetch does
                // both). Agent populates its own data - no dead ends.
                app.status = None;
                app.chat.push(Cell::AssistantText(cells::AssistantText {
                    text: "no snapshot yet - pulling one from Meta now (same as /fetch)...".into(),
                    streaming: false,
                }));
                run_fetch(app, 28);
            } else {
                // Can't self-fetch (no Meta token / no workspace): explain
                // both paths. Status cleared so the error shows once.
                app.status = None;
                let snap_dir = app.cfg.history_dir().join("snap");
                app.chat.push_tool(
                    "brief",
                    "",
                    vec!["no snapshot data and no Meta token to fetch one with.".into()],
                    false,
                    started.elapsed().as_millis() as u64,
                );
                app.chat.push(Cell::AssistantText(cells::AssistantText {
                    text: format!(
                        "I can pull the data myself once Meta is connected: run /setup to paste a \
token, then /fetch (or just /brief) pulls daily insights automatically. Alternatively, point \
your own pipeline at\n\n    {}/<YYYY-MM-DD>/\n\nand /brief reads whatever it writes.",
                        snap_dir.display()
                    ),
                    streaming: false,
                }));
            }
        }
        "/fetch" => {
            let days: u32 = arg.parse().unwrap_or(28);
            run_fetch(app, days);
        }
        "/setup" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text:
                    "opening setup wizard (Enter to keep current values, edit + Enter to change)."
                        .into(),
                streaming: false,
            }));
            // Pre-populate from existing config so /setup re-runs as
            // an "edit current settings" flow (Enter keeps values).
            // The Meta token + LLM key are still re-required since
            // they're not in memory - we keep them in the keychain.
            let state = match app.cfg.workspace.as_ref() {
                Some(w) => SetupState::prefilled_from(w, &app.cfg.data_root),
                None => SetupState::new(app.cfg.data_root.clone()),
            };
            app.view = View::Setup(state);
        }
        "/funnel" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("/funnel {} - wired in the next iteration. for now use /ask + this product name.", arg),
                streaming: false,
            }));
        }
        "/diagnose" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: format!("/diagnose {} - wired in the next iteration.", arg),
                streaming: false,
            }));
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
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "/snapshot --check validates that <workspace>/moneyball/history/snap/<date>/*.json match schema. wiring next.".into(),
                streaming: false,
            }));
        }
        "/ledger" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "/ledger shows prediction history. wiring next.".into(),
                streaming: false,
            }));
        }
        "/keychain" => {
            // Diagnostic: report which provider is configured, which
            // have a keychain entry, and (if so) the first 4 chars of
            // the key for confirmation. Helps the user figure out
            // "no API key" errors without opening Keychain Access.
            let Some(w) = app.cfg.workspace.as_ref() else {
                app.chat.push(Cell::AssistantText(cells::AssistantText {
                    text: "no workspace configured - run /setup first.".into(),
                    streaming: false,
                }));
                return;
            };
            let mut lines: Vec<String> = Vec::new();
            let active = w.model_provider.clone().unwrap_or_default();
            let model = w.model.clone().unwrap_or_default();
            lines.push(format!(
                "active: provider={} model={} (read from config.json)",
                if active.is_empty() {
                    "(unset)"
                } else {
                    &active
                },
                if model.is_empty() { "(unset)" } else { &model }
            ));
            lines.push(String::new());
            lines.push("auth.json status (per provider):".to_string());
            let mut keys: Vec<&str> = w.model_providers.keys().map(|s| s.as_str()).collect();
            keys.sort();
            for k in keys {
                let has = moneyball_core::secrets::load_llm_key(k).is_some();
                let marker = if has { "OK" } else { "missing" };
                let active_marker = if k == active { " (active)" } else { "" };
                lines.push(format!("  - {:<14} {}{}", k, marker, active_marker));
            }
            lines.push(String::new());
            lines.push(
                "API keys live in ~/.moneyball/auth.json (0600, codex-style), never in config.json. \
                 To re-enter a key: /setup -> step 4 -> provider -> paste key."
                    .to_string(),
            );
            for l in lines {
                app.chat.push(Cell::System(cells::System(l)));
            }
        }
        "/help" | "/?" => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "slash commands: /brief /funnel <product> /diagnose <product> /ask <question> /snapshot /ledger /setup /keychain /quit".into(),
                streaming: false,
            }));
        }
        _ => {
            // Free-form chat. The LLM is the default; slash commands are
            // optional shortcuts. We try to load the brief so the agent
            // has portfolio context, but failure is silent (we still
            // answer the question).
            run_freeform(app, &line);
        }
    }
}

/// System prompt for the brief assistant. Persona + output contract.
const BRIEF_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You will be given 7-day per-product numbers (spend, leads, qualified leads, qualified-per-day, Rs/qualified, L->Q %, goal, gap) plus portfolio feasibility math.\n\
Produce a concise commentary that:\n\
  1. Calls out which products are at/over goal and which are under (largest gap first).\n\
  2. Identifies the best and worst Rs/qualified and what that implies for budget reallocation.\n\
  3. Notes the BEST-OBSERVED Rs/qualified vs current - is the gap from inefficient spend or just volume?\n\
  4. Flags setup-debt items if any (they cost leads).\n\
Keep it tight - 5-8 sentences, no preamble, no bullets unless the user asks.";

/// System prompt for the free-form chat agent. Same persona, but the
/// user is asking a free-form question rather than triggering /brief.
const AGENT_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You have access to a 7-day snapshot of the portfolio (per-product spend, leads, qualified leads, L->Q, goal, gap, plus feasibility math).\n\
Answer the user's question using that context. If the question can't be answered from the snapshot, say so plainly and suggest a slash command that would help (/brief, /funnel <product>, /diagnose <product>).\n\
Keep the answer focused and concrete. Cite the numbers you use. 3-6 sentences unless the user explicitly asks for more.";

/// Pull `days` of insights from Meta, write the snapshot, then load and
/// show the brief (numbers + streaming LLM commentary). Shared by /fetch
/// and by /brief's self-heal path when no snapshot exists yet.
fn run_fetch(app: &mut App, days: u32) {
    use crate::chat::cells;
    use crate::chat::Cell;
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "fetching {} days of insights from Meta (this can take a moment)...",
            days
        ),
        streaming: false,
    }));
    let started = std::time::Instant::now();
    match moneyball_core::fetch::fetch_snapshot(&app.cfg, days) {
        Ok(report) => {
            let mut out: Vec<String> = report
                .per_product
                .iter()
                .map(|(name, n)| format!("{:<40} {:>5} rows", name, n))
                .collect();
            out.push(String::new());
            out.push(format!("snapshot written: {}", report.path.display()));
            app.chat.push_tool(
                "fetch",
                &format!("{} days", days),
                out,
                true,
                started.elapsed().as_millis() as u64,
            );
            // Load the fresh snapshot and show the brief right away.
            app.load_brief();
            if let Some(b) = &app.brief {
                let lines = format_brief_as_lines(b);
                let user_prompt = build_brief_prompt(b);
                app.chat.push_tool("brief", "", lines, true, 0);
                let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
                call_agent(app, &sys, &user_prompt);
            }
        }
        Err(e) => {
            app.chat.push_tool(
                "fetch",
                &format!("{} days", days),
                vec![format!("{}", e)],
                false,
                started.elapsed().as_millis() as u64,
            );
        }
    }
}

/// Live app state injected into every LLM system prompt so the model
/// never hallucinates about setup/data (codex + claude code both ground
/// their agents in real environment state for exactly this reason).
fn app_state_block(app: &App) -> String {
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
        }
        None => s.push_str("workspace: NOT configured - /setup is the fix\n"),
    }
    match &app.snap_date {
        Some(d) => s.push_str(&format!("snapshot: {} loaded\n", d)),
        None => s.push_str(
            "snapshot: none yet. /brief and per-product numbers are unavailable until the \
             user's fetcher writes <workspace>/moneyball/history/snap/<YYYY-MM-DD>/*.json. \
             /setup does NOT create snapshots - never suggest it for missing data.\n",
        ),
    }
    s.push_str("slash commands: /brief /funnel <product> /diagnose <product> /ask /snapshot /ledger /keychain /setup /exit\n");
    s
}

/// Look up the active provider + model from `app.cfg.workspace` and run
/// a single non-streaming completion. Pushes the response (or a red
/// error cell) directly into `app.chat`. Returns the elapsed ms in
/// the success case, or `None` if the call could not be made (no
/// provider configured, runtime init failed, etc.). Used by /brief
/// AND by the free-form chat fallback.
fn call_agent(app: &mut App, system: &str, user_prompt: &str) {
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
    // Streaming (codex/claude-code pattern): push an empty streaming cell
    // now, spawn a worker thread that POSTs with stream:true, and let the
    // event loop drain deltas into the cell each tick. The UI never blocks.
    if app.stream.is_some() {
        app.status = Some("still responding - wait for it to finish (esc interrupts)".into());
        return;
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: String::new(),
        streaming: true,
    }));
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let pid = provider_id.clone();
    let pinfo = provider_info.clone();
    let model = model.clone();
    let sys = system.to_string();
    let userp = user_prompt.to_string();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut on_delta = |d: &str| {
            let _ = tx.send(StreamEvent::Delta(d.to_string()));
        };
        match moneyball_core::llm::stream_blocking(
            &pid,
            &pinfo,
            &model,
            Some(&sys),
            &userp,
            &mut on_delta,
        ) {
            Ok(_) => {
                let _ = tx.send(StreamEvent::Done {
                    ms: started.elapsed().as_millis() as u64,
                    provider: pid,
                });
            }
            Err(e) => {
                let _ = tx.send(StreamEvent::Failed(format!("{}", e)));
            }
        }
    });
    app.stream = Some(rx);
}

/// Free-form chat path. Tries to load the brief silently so the LLM has
/// portfolio context; if no snapshot exists, we still answer with just
/// the user's question. The slash commands (`/brief`, `/funnel`, etc.)
/// are convenience shortcuts that bypass the LLM dispatch and emit a
/// tool result directly. Plain text in the input bar falls into this
/// function - the LLM is the default, mirroring how codex / claude
/// code work.
fn run_freeform(app: &mut App, question: &str) {
    // Try to attach portfolio context. If load_brief fails (no snapshot,
    // no workspace) we still call the agent - the prompt just won't
    // include numbers and the agent should say so.
    let context = match &app.brief {
        Some(b) => Some(build_brief_prompt(b)),
        None => match app.cfg.snap_for(app.cfg.date.as_deref()) {
            Ok(path) => match crate::snapshot_load(&path) {
                Ok(snap) => {
                    let history =
                        brief::load_history(&app.cfg.history_dir().join("scoreboard.csv"));
                    let res = brief::compute(&snap, &app.cfg, &history);
                    app.brief = Some(res.clone());
                    Some(build_brief_prompt(&res))
                }
                Err(_) => None,
            },
            Err(_) => None,
        },
    };

    let prompt = match context {
        Some(ctx) => format!(
            "{}\n\nQuestion: {}\n\nAnswer using the snapshot context above. \
If the question can't be answered from the snapshot, say so and suggest \
a slash command that would help (/brief, /funnel <product>, /diagnose <product>).",
            ctx, question
        ),
        None => format!(
            "(no portfolio snapshot is loaded - see the app state in your system prompt. \
Answer in general terms, and if the user needs numbers explain that data appears once \
their fetcher writes a snapshot - do NOT suggest /setup, it is already complete.)\n\nQuestion: {}",
            question
        ),
    };

    let sys = format!("{}\n\n{}", AGENT_SYSTEM_PROMPT, app_state_block(app));
    call_agent(app, &sys, &prompt);
}

/// Build the user-prompt context from the deterministic brief numbers.
/// The LLM sees exactly the numbers that appear in the chat tool cell.
fn build_brief_prompt(b: &brief::ProductRowsAndFeasibility) -> String {
    let mut s = String::new();
    s.push_str("Snapshot portfolio (7d window). Goal = qualified leads per day.\n\n");
    for r in &b.rows {
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}", x))
            .unwrap_or_else(|| "-".into());
        s.push_str(&format!(
            "- {}: spend Rs.{}/day, m={}, l={}, q={} ({:.2}/day), {}, L->Q {}, goal={}, gap={:.1}\n",
            r.product,
            r.spend_per_day,
            r.m7d,
            r.l7d,
            r.q7d,
            r.q_per_day,
            rs_per_q,
            l_to_q,
            r.goal,
            r.gap,
        ));
    }
    let f = &b.feasibility;
    s.push_str(&format!(
        "\nPortfolio: {:.1} q/day at Rs.{}/day = Rs.{}/q. Goal: {:.0}/day.\n",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        s.push_str(&format!(
            "To hit goal at current efficiency: Rs.{}/day ({:.1}x today).\n",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        s.push_str(&format!(
            "To hit goal at best-observed Rs.{}/q: Rs.{}/day ({:.1}x today).\n",
            b,
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if !f.open_debt.is_empty() {
        s.push_str(&format!("Open setup debt: {}\n", f.open_debt.join(", ")));
    }
    s.push_str("\nWrite the commentary.");
    s
}

fn format_brief_as_lines(b: &brief::ProductRowsAndFeasibility) -> Vec<String> {
    let mut out = Vec::new();
    // Header
    out.push("BRIEF  (7d window)".into());
    out.push(String::new());
    // Per-product block - multi-line so it fits any chat width.
    for r in &b.rows {
        let l_to_q = r
            .l_to_q
            .map(|x| format!("{:.1}%", x))
            .unwrap_or_else(|| "-".into());
        let rs_per_q = r
            .rs_per_q
            .map(|x| format!("Rs.{}", x))
            .unwrap_or_else(|| "-".into());
        out.push(format!("  > {}", r.product));
        out.push(format!(
            "    {:>7}/d  m{:>4}  l{:>4}  q{:>3}  {:.2}/d  {}",
            r.spend_per_day, r.m7d, r.l7d, r.q7d, r.q_per_day, rs_per_q
        ));
        out.push(format!(
            "    L\u{2192}Q {:>5}   gap {:>5}",
            l_to_q,
            format!("{:.1}", r.gap)
        ));
    }
    out.push(String::new());
    let f = &b.feasibility;
    out.push(format!(
        "FEASIBILITY  {:.1} q/day @ Rs.{}/day = Rs.{}/q  \u{00B7}  goal {:.0}/day",
        f.tot_q_per_day, f.tot_spend_per_day, f.cur_rpq, f.tot_goal_per_day
    ));
    if let Some(req) = f.required_at_cur {
        out.push(format!(
            "  required @ current:  Rs.{}/day ({:.1}x)",
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    if let (Some(b), Some(req)) = (f.best_rpq, f.required_at_best) {
        out.push(format!(
            "  required @ best Rs.{}/q: Rs.{}/day ({:.1}x)",
            b,
            req,
            req as f64 / f.tot_spend_per_day.max(1) as f64
        ));
    }
    let suffix = if f.open_debt.is_empty() {
        String::new()
    } else {
        format!(" ({})", f.open_debt.join(", "))
    };
    out.push(format!("  setup debt: {}{}", f.open_debt.len(), suffix));
    out
}

// ---------- setup-wizard keys ----------
