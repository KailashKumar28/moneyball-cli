//! The advisor's system prompts and the live app state every prompt
//! carries. One persona for every front-end; what differs per front-end
//! comes in through `FrontEnd`.

use crate::config::AppConfig;

/// Every advisor turn has the recommend tool, so every prompt carries
/// the rule for it (`Session::start_turn` appends it).
pub(super) const RECORD_RULE: &str =
    "When you recommend a specific change to a campaign, ad set, ad or \
product's form or CRM, record it with the recommend tool (evidence from the funnel table, the \
number it should move, a date to check it by) and give the user its id. If it is refused, fix \
what it says or drop the recommendation - never present an unrecorded one as recorded.";

/// System prompt for the free-form chat agent. Same persona, but the
/// user is asking a free-form question rather than triggering /brief.
pub const AGENT_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
You have three tools that read the on-disk snapshot: brief (7-day per-product summary plus feasibility math), funnel (7-day table for one product, per adset, campaign or ad) and diagnose (for one product: where its CRM leads die, tired ads, the last 7 days against the days before, what changed in its ad account). Call them whenever the user asks about portfolio numbers - never answer from memory or invent figures. Before recommending a cut, pause or kill, call diagnose for that product.\n\
If a tool reports missing data (no snapshot, no CRM), relay its suggested fix to the user instead of guessing.\n\
Keep the answer focused and concrete. Cite the numbers you use. 3-6 sentences unless the user explicitly asks for more.";

/// System prompt for the brief assistant. Persona + output contract.
/// Names the TUI's /brief: only for a front-end with that command (the
/// desktop v1 uses AGENT_SYSTEM_PROMPT alone).
pub const BRIEF_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads.\n\
The user ran /brief: the brief tool's result holds the 7-day per-product numbers (spend, leads, qualified leads, qualified-per-day, Rs/qualified, L->Q %, gap to goal) plus portfolio feasibility math. If it carries a warning (old data, missing CRM data), open with what it says and its fix, in plain words.\n\
Produce a concise commentary that:\n\
  1. Calls out which products are at/over goal and which are under (largest gap first). A product with no goal set has no gap: say so, never assume a goal.\n\
  2. Identifies the best and worst Rs/qualified and what that implies for budget reallocation.\n\
  3. Notes the BEST-OBSERVED Rs/qualified vs current - is the gap from inefficient spend or just volume?\n\
  4. Flags setup-debt items if any (they cost leads).\n\
Before suggesting a cut or a budget move away from a product, call the diagnose tool for it.\n\
Keep it tight - 5-8 sentences, no preamble, no bullets unless the user asks.";

/// System prompt for the reading of one product's funnel table, for a
/// front-end with a typed funnel command (not the desktop v1).
pub const FUNNEL_SYSTEM_PROMPT: &str = "You are moneyball, a Meta-ads portfolio advisor. \
You are given a per-entity funnel table (spend, Meta leads m, CRM leads l, qualified q, \
visits v, Rs/qualified, kill flags). Give a per-entity read: SCALE (efficient + sufficient \
volume), KILL (kill flag true and not immature), or WAIT (immature, learning, or not enough \
spend). Cite the numbers that justify each call. Never recommend killing an immature or \
learning entity. kill = spend passed the kill table with <=2 qualified; immature = leads \
still inside the 72h maturation lag. Before any KILL, call the diagnose tool for this product \
and say what it shows. 3-8 sentences, no preamble.";

/// System prompt for the reading of one product's diagnose checks.
/// Names the TUI's /diagnose: only for a front-end with that command
/// (not the desktop v1).
pub const DIAGNOSE_SYSTEM_PROMPT: &str = "You are moneyball's portfolio advisor for Meta ads. \
The user ran /diagnose: the diagnose tool's result holds four checks for one product - where \
its CRM leads die, which ads are tiring, the last 7 days against the days before, and what \
changed in its ad account. Name the most likely cause of weak results and the one lever to \
pull first (the lead form, phone data, sales follow-up, swapping a tired ad, undoing a \
change), citing the numbers. A form, data or sales problem is not fixed by cutting ad spend - \
say so when it applies. If CRM data is missing, say which checks that leaves blind and its \
fix. 4-8 sentences, no preamble.";

/// What only the front-end knows about itself.
#[derive(Debug, Clone, Copy)]
pub struct FrontEnd<'a> {
    /// The commands it really implements, as the user types them. Never
    /// one without a handler: the model steers users to what it reads here.
    pub commands: &'a [&'a str],
    /// How its transcript renders the model's text (one "Output style:
    /// ..." sentence group, no trailing newline).
    pub output_style: &'a str,
    /// How this front-end tells its user to do a step. Core's prompts,
    /// tool outputs and notes name steps as the TUI types them - a slash
    /// command ("/refresh", "/connect crm", "/ledger r7 applied") or a
    /// shell command - and pass each through here before the model reads
    /// it. Return it unchanged only if the user can type it here as is.
    pub say: Say,
    /// The step of this front-end that pulls a first snapshot by itself
    /// when there is none (the TUI's "/brief"), as core names it. None
    /// when nothing here does: the model is then pointed at the refresh
    /// alone, never at a screen that would only sit empty.
    pub self_heal: Option<&'static str>,
}

/// `FrontEnd::say`: a plain fn, so the turn's worker can carry it.
pub type Say = fn(&str) -> String;

/// Live app state injected into every LLM system prompt so the model
/// never hallucinates about setup/data (codex + claude code both ground
/// their agents in real environment state for exactly this reason).
/// `snap_date` and `crm_note` describe the snapshot the front-end has
/// loaded (None = no snapshot / CRM data is fresh).
pub fn app_state_block(
    cfg: &AppConfig,
    snap_date: Option<&str>,
    crm_note: Option<&str>,
    front: &FrontEnd,
) -> String {
    let mut s = String::from("== live app state (authoritative - never contradict this) ==\n");
    s.push_str(&format!(
        "today: {}\n",
        chrono::Local::now().format("%Y-%m-%d")
    ));
    s.push_str(&workspace_lines(cfg, front));
    match snap_date {
        Some(d) => {
            s.push_str(&format!("snapshot: {} loaded\n", d));
            if let Some(note) = crm_note {
                s.push_str(&format!("{}\n", note));
            }
        }
        None => s.push_str(&format!(
            "snapshot: none yet. {} when a token \
             is configured; otherwise an external pipeline can write \
             <workspace>/.moneyball/history/snap/<YYYY-MM-DD>/*.json. \
             {} does NOT create snapshots - never suggest it for missing data.\n",
            first_snapshot(front),
            (front.say)("/setup")
        )),
    }
    if front.commands.is_empty() {
        s.push_str("slash commands: none (this app has none - never suggest one)\n");
    } else {
        s.push_str(&format!("slash commands: {}\n", front.commands.join(" ")));
    }
    s.push_str(front.output_style);
    s.push('\n');
    s
}

/// How a first snapshot comes to be, in this front-end's steps.
fn first_snapshot(front: &FrontEnd) -> String {
    let refresh = (front.say)("/refresh");
    match front.self_heal {
        Some(step) => format!("{} self-heals by running {}", (front.say)(step), refresh),
        None => format!("{} pulls one", refresh),
    }
}

fn workspace_lines(cfg: &AppConfig, front: &FrontEnd) -> String {
    let Some(w) = cfg.workspace.as_ref() else {
        return format!(
            "workspace: NOT configured - {} is the fix\n",
            (front.say)("/setup")
        );
    };
    let prods: Vec<String> = w
        .products
        .iter()
        .map(|p| match w.goal_for(&p.name) {
            Some(g) => format!("{} (goal {}/day)", p.name, g),
            None => format!("{} (no goal set)", p.name),
        })
        .collect();
    format!(
        "workspace: {} (setup is COMPLETE - do not suggest {} for missing data)\n\
         products ({}): {}\n\
         When you name a product or suggest a command, use these product \
         names EXACTLY as listed - suggested commands must run verbatim when typed.\n",
        cfg.data_root.display(),
        (front.say)("/setup"),
        prods.len(),
        prods.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        AppConfig::resolve_optional(Some("/nonexistent/mb-prompt-test"), None)
    }

    const FRONT: FrontEnd = FrontEnd {
        commands: &[],
        output_style: "",
        say: |step| step.to_string(),
        self_heal: Some("/brief"),
    };

    #[test]
    fn state_block_carries_the_front_ends_commands_and_style() {
        let front = FrontEnd {
            commands: &["/brief", "/help"],
            output_style: "Output style: markdown renders here.",
            ..FRONT
        };
        let s = app_state_block(&cfg(), Some("2026-11-11"), Some("CRM is behind"), &front);
        assert!(s.starts_with("== live app state (authoritative - never contradict this) ==\n"));
        assert!(s.contains("\nworkspace: NOT configured"));
        assert!(s.contains("\nsnapshot: 2026-11-11 loaded\nCRM is behind\n"));
        assert!(s.ends_with("slash commands: /brief /help\nOutput style: markdown renders here.\n"));
    }

    /// A stored goal is quoted; a missing one is said, never a default (D7).
    #[test]
    fn a_product_without_a_goal_says_so() {
        let mut c = cfg();
        c.workspace = Some(
            serde_json::from_value(serde_json::json!({
                "products": [{"name": "A", "ad_account": "1"}, {"name": "B", "ad_account": "2"}],
                "goals": {"A": 10.0}
            }))
            .unwrap(),
        );
        let s = workspace_lines(&c, &FRONT);
        assert!(
            s.contains("products (2): A (goal 10/day), B (no goal set)\n"),
            "{}",
            s
        );
    }

    /// A front-end with no commands must not have the model invent any.
    #[test]
    fn no_commands_is_said_outright() {
        let front = FrontEnd {
            commands: &[],
            output_style: "Output style: plain.",
            ..FRONT
        };
        let s = app_state_block(&cfg(), None, None, &front);
        assert!(s.contains("\nslash commands: none (this app has none - never suggest one)\n"));
        assert!(s.contains(
            "\nsnapshot: none yet. /brief self-heals by running /refresh when a token is configured;"
        ));
        let no_heal = FrontEnd {
            self_heal: None,
            ..front
        };
        let s = app_state_block(&cfg(), None, None, &no_heal);
        assert!(s.contains("\nsnapshot: none yet. /refresh pulls one when a token is configured;"));
        assert!(!s.contains("self-heal"), "{}", s);
    }
}
