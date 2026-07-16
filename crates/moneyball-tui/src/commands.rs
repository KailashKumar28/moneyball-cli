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
                let mut out = format_brief_as_lines(b);
                let mut user_prompt = build_brief_prompt(b);
                if app.crm_missing {
                    out.push(format!("warn: {}", moneyball_core::crm::NO_CRM_WARNING));
                    user_prompt.push_str(&format!(
                        "\n\nIMPORTANT: {}. Do not diagnose lead quality from l/q/v.",
                        moneyball_core::crm::NO_CRM_WARNING
                    ));
                }
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
            run_funnel(app, arg);
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
                        "no snapshots in {} - run /fetch to pull one.",
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

/// Pull `days` of insights from Meta on a worker thread (the network pull
/// takes seconds - blocking the event loop here froze the UI). The result
/// arrives as StreamEvent::FetchDone/FetchFailed on the tick drain, which
/// hands it to `on_fetch_done`/`on_fetch_failed` below. Shared by /fetch
/// and by /brief's self-heal path when no snapshot exists yet.
fn run_fetch(app: &mut App, days: u32) {
    use crate::chat::cells;
    use crate::chat::Cell;
    if app.stream.is_some() {
        app.status = Some("still working - esc to interrupt, then resend".into());
        return;
    }
    app.chat.push(Cell::AssistantText(cells::AssistantText {
        text: format!(
            "fetching {} days of insights from Meta (this can take a moment)...",
            days
        ),
        streaming: false,
    }));
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let cfg = app.cfg.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let ev = match moneyball_core::fetch::fetch_snapshot(&cfg, days) {
            Ok(report) => StreamEvent::FetchDone {
                report,
                days,
                ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => StreamEvent::FetchFailed {
                err: format!("{}", e),
                days,
                ms: started.elapsed().as_millis() as u64,
            },
        };
        let _ = tx.send(ev);
    });
    app.stream = Some(rx);
}

/// Fetch worker succeeded: show the per-product rows, then load the fresh
/// snapshot and chain into the brief + streaming LLM commentary. Called
/// from the event loop's drain AFTER it cleared `app.stream`, so
/// `call_agent` is free to start the LLM stream.
pub(crate) fn on_fetch_done(
    app: &mut App,
    report: moneyball_core::fetch::FetchReport,
    days: u32,
    ms: u64,
) {
    let mut out: Vec<String> = report
        .per_product
        .iter()
        .map(|(name, n)| format!("{:<40} {:>5} rows", name, n))
        .collect();
    out.push(String::new());
    out.push(format!("snapshot written: {}", report.path.display()));
    app.chat
        .push_tool("fetch", &format!("{} days", days), out, true, ms);
    app.load_brief();
    if let Some(b) = &app.brief {
        let lines = format_brief_as_lines(b);
        let user_prompt = build_brief_prompt(b);
        app.chat.push_tool("brief", "", lines, true, 0);
        let sys = format!("{}\n\n{}", BRIEF_SYSTEM_PROMPT, app_state_block(app));
        call_agent(app, &sys, &user_prompt);
    }
}

pub(crate) fn on_fetch_failed(app: &mut App, err: String, days: u32, ms: u64) {
    app.chat
        .push_tool("fetch", &format!("{} days", days), vec![err], false, ms);
}

/// `/funnel <product> [campaign|adset|ad]` - per-entity funnel as a tool
/// cell, then a streaming LLM read of it (scale/kill/wait per entity).
/// Compute is instant (snapshot on disk), so no worker thread needed.
fn run_funnel(app: &mut App, arg: &str) {
    use crate::chat::cells;
    use crate::chat::Cell;
    let started = std::time::Instant::now();
    let products: Vec<String> = app
        .cfg
        .workspace
        .as_ref()
        .map(|w| w.products.iter().map(|p| p.name.clone()).collect())
        .unwrap_or_default();

    // Trailing token may be the level; everything before it is the
    // product (product names contain spaces).
    let (product, by) = match arg.rsplit_once(' ') {
        Some((head, lvl)) if ["campaign", "adset", "ad"].contains(&lvl) => {
            (head.trim().to_string(), lvl.to_string())
        }
        _ => (arg.to_string(), "adset".to_string()),
    };
    if product.is_empty() || !products.iter().any(|p| p == &product) {
        app.chat.push(Cell::AssistantText(cells::AssistantText {
            text: format!(
                "usage: /funnel <product> [campaign|adset|ad]\nconfigured products: {}",
                if products.is_empty() {
                    "(none - run /setup)".into()
                } else {
                    products.join(", ")
                }
            ),
            streaming: false,
        }));
        return;
    }

    let snap = match app
        .cfg
        .snap_for(app.cfg.date.as_deref())
        .and_then(|p| moneyball_core::snapshot::load(&p))
    {
        Ok(s) => s,
        Err(_) => {
            app.chat.push(Cell::AssistantText(cells::AssistantText {
                text: "no snapshot yet - run /fetch first (or /brief, which self-heals).".into(),
                streaming: false,
            }));
            return;
        }
    };
    let level = moneyball_core::funnel::By::parse(&by).expect("level pre-validated");
    let rows = moneyball_core::funnel::compute(&snap, &app.cfg, &product, 7, level);
    let mut lines = vec![format!(
        "FUNNEL {} - by {} - 7d - snapshot {}",
        product, by, snap.date
    )];
    lines.extend(
        moneyball_core::funnel::table(&rows)
            .lines()
            .map(String::from),
    );
    let table_text = lines.join("\n");
    app.chat.push_tool(
        "funnel",
        arg,
        lines,
        true,
        started.elapsed().as_millis() as u64,
    );

    let sys = format!("{}\n\n{}", FUNNEL_SYSTEM_PROMPT, app_state_block(app));
    let user = format!(
        "Here is the 7-day per-{} funnel for {} (kill = spend passed the kill table \
         with <=2 qualified; immature = leads still inside the 72h maturation lag):\n\n{}",
        by, product, table_text
    );
    call_agent(app, &sys, &user);
}

const FUNNEL_SYSTEM_PROMPT: &str = "You are moneyball, a Meta-ads portfolio advisor. \
You are given a per-entity funnel table (spend, Meta leads m, CRM leads l, qualified q, \
visits v, Rs/qualified, kill flags). Give a per-entity read: SCALE (efficient + sufficient \
volume), KILL (kill flag true and not immature), or WAIT (immature, learning, or not enough \
spend). Cite the numbers that justify each call. Never recommend killing an immature or \
learning entity. 3-8 sentences, no preamble.";

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
        Some(d) => {
            s.push_str(&format!("snapshot: {} loaded\n", d));
            if app.crm_missing {
                s.push_str(&format!("{}\n", moneyball_core::crm::NO_CRM_WARNING));
            }
        }
        None => s.push_str(
            "snapshot: none yet. /brief self-heals by running /fetch (Meta pull) when a token \
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
    // Agent turn (ARCHITECTURE.md 6b): record the user item, then run
    // the tool loop on a worker thread. Ev's arrive via the tick drain;
    // the UI never blocks and Esc cancels via the shared flag.
    if app.stream.is_some() {
        app.status = Some("still responding - wait for it to finish (esc interrupts)".into());
        return;
    }
    app.record(moneyball_core::agent::Item::User {
        text: user_prompt.to_string(),
    });
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
        let exec = SnapshotTools { cfg };
        // Only implemented tools go on the wire - the model must never
        // be steered into stubs (audit F4).
        let tools = vec![
            moneyball_core::tools::brief_tool(),
            moneyball_core::tools::funnel_tool(),
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

/// Tool executor over on-disk snapshot data. Runs on the agent worker
/// thread; errors become ToolOutput{is_error} messages the model sees.
struct SnapshotTools {
    cfg: moneyball_core::AppConfig,
}

impl SnapshotTools {
    fn snap(&self) -> Result<moneyball_core::Snapshot, String> {
        self.cfg
            .snap_for(self.cfg.date.as_deref())
            .and_then(|p| moneyball_core::snapshot::load(&p))
            .map_err(|_| {
                "no snapshot on disk - tell the user to run /fetch (or /brief, which \
                 self-heals) before asking for numbers"
                    .to_string()
            })
    }
}

impl moneyball_core::agent::ToolExec for SnapshotTools {
    fn run(&self, name: &str, args: &serde_json::Value) -> Result<String, String> {
        match name {
            "brief" => {
                let snap = self.snap()?;
                let history = brief::load_history(&self.cfg.history_dir().join("scoreboard.csv"));
                let b = brief::compute(&snap, &self.cfg, &history);
                let mut out = format_brief_as_lines(&b).join("\n");
                if moneyball_core::crm::is_empty(&snap.crm) {
                    out.push_str(&format!(
                        "\nIMPORTANT: {}. Do not diagnose lead quality from l/q/v.",
                        moneyball_core::crm::NO_CRM_WARNING
                    ));
                }
                Ok(out)
            }
            "funnel" => {
                let products: Vec<String> = self
                    .cfg
                    .workspace
                    .as_ref()
                    .map(|w| w.products.iter().map(|p| p.name.clone()).collect())
                    .unwrap_or_default();
                let product = args.get("product").and_then(|p| p.as_str()).unwrap_or("");
                if !products.iter().any(|p| p == product) {
                    return Err(format!(
                        "unknown product \"{}\" - pass one of: {}",
                        product,
                        products.join(", ")
                    ));
                }
                let snap = self.snap()?;
                let rows = moneyball_core::funnel::compute(
                    &snap,
                    &self.cfg,
                    product,
                    7,
                    moneyball_core::funnel::By::Adset,
                );
                Ok(format!(
                    "FUNNEL {} - by adset - 7d - snapshot {}\n{}",
                    product,
                    snap.date,
                    moneyball_core::funnel::table(&rows)
                ))
            }
            other => Err(format!(
                "tool \"{}\" is not implemented - answer from the tools you have",
                other
            )),
        }
    }
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
    call_agent(app, &sys, question);
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
