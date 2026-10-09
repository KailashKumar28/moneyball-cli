//! moneyball binary - clap dispatch + session handling.

mod connect_flow;
mod crm_cli;
mod debug_cli;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use moneyball_core::AppConfig;

#[derive(Parser, Debug)]
#[command(name = "moneyball", version, about = "Read-only Meta-ads advisor CLI")]
struct Cli {
    /// Path to the data workspace (sibling fin_campaign_analysis/ by default).
    #[arg(long, global = true)]
    data_root: Option<String>,

    /// Snapshot date YYYY-MM-DD (default: latest).
    #[arg(long, global = true)]
    date: Option<String>,

    /// Continue from the most-recent session (alias --last).
    #[arg(short = 'c', long = "continue", conflicts_with_all = ["resume", "list"])]
    continue_last: bool,

    /// Resume a specific session by ID.
    #[arg(long = "resume", value_name = "ID", conflicts_with_all = ["continue_last", "list"])]
    resume: Option<String>,

    /// List saved sessions and exit.
    #[arg(long = "list", conflicts_with_all = ["continue_last", "resume"])]
    list: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Start the interactive REPL (default if no sub-command).
    Repl,
    /// 7-day portfolio brief - per-product summary + feasibility math.
    Brief {
        #[arg(long)]
        date: Option<String>,
    },
    /// Validate the snapshot for a date.
    Snapshot {
        #[arg(long)]
        check: bool,
    },
    /// Pull daily ad insights from Meta and write today's snapshot.
    Fetch {
        /// How many trailing days of daily rows to pull (ending yesterday).
        #[arg(long, default_value_t = 28)]
        days: u32,
    },
    /// Per-entity funnel for a product: Meta + CRM joined, kill math.
    Funnel {
        /// Product name (as configured in the workspace).
        product: String,
        /// Aggregation level: campaign, adset or ad.
        #[arg(long, default_value = "adset")]
        by: String,
        /// Window in complete days ending yesterday.
        #[arg(long, default_value_t = 7)]
        window: u32,
    },
    /// Connect CRM data - print the crm.json contract, validate an export.
    Crm {
        #[command(subcommand)]
        cmd: crm_cli::CrmCmd,
    },
    /// Compute the creative report from a snapshot (no network):
    /// per-creative delivery + CRM funnel, written as report.json.
    Report {
        /// Snapshot date YYYY-MM-DD (default: latest).
        #[arg(long)]
        date: Option<String>,
        /// Trailing complete days the report covers (ending the day
        /// before the snapshot date).
        #[arg(long, default_value_t = 1)]
        window: u32,
    },
    /// Dump a saved session transcript with an invariant audit; file,
    /// review, and archive bug reports (see debug_cli.rs).
    Debug(debug_cli::DebugArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Workspace first: sessions live in <workspace>/.moneyball/sessions/,
    // so every session flag needs the resolved config. Owned so the
    // borrow doesn't pin cfg (the REPL arm moves cfg into the TUI).
    let cfg = AppConfig::resolve_optional(cli.data_root.as_deref(), cli.date.as_deref());
    let root_buf = cfg.sessions_root().map(std::path::Path::to_path_buf);
    let root = root_buf.as_deref();

    // --list: print saved sessions and exit.
    if cli.list {
        let sessions = moneyball_core::session::list(root)?;
        if sessions.is_empty() {
            println!("(no saved sessions)");
            return Ok(());
        }
        println!("saved sessions (newest first):");
        for m in sessions {
            println!("{}", moneyball_core::session::fmt_meta_line(&m));
        }
        return Ok(());
    }

    // --resume <id>: reopen that session's log and replay its transcript.
    if let Some(id) = cli.resume.clone() {
        let (log, items) = moneyball_core::session::SessionLog::open(&id, root)
            .with_context(|| format!("no session found with id '{}'", id))?;
        println!(
            "resuming session {} ({} items, started {})",
            log.meta.id,
            items.len(),
            log.meta.started_at
        );
        return moneyball_tui::run_with_cfg(Some((log, items)), Some(cfg));
    }

    // -c / --continue: reopen the latest session if one exists.
    let resume_session = if cli.continue_last {
        match moneyball_core::session::latest_id(root)? {
            Some(id) => Some(moneyball_core::session::SessionLog::open(&id, root)?),
            None => {
                eprintln!("no previous session to continue - starting a fresh one");
                None
            }
        }
    } else {
        None
    };

    match cli.cmd.unwrap_or(Cmd::Repl) {
        Cmd::Repl => {
            moneyball_tui::run_with_cfg(resume_session, Some(cfg))?;
        }
        Cmd::Brief { date } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::brief::run(&strict, date.as_deref().or(cli.date.as_deref()))?;
        }
        Cmd::Snapshot { check: _ } => {
            let p = cfg.snap_for(cli.date.as_deref())?;
            println!("snapshot ok: {}", p.display());
        }
        Cmd::Fetch { days } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            let token = moneyball_core::secrets::load_meta_token().ok_or_else(|| {
                anyhow::anyhow!("no Meta token in keychain - run /setup to connect Meta")
            })?;
            println!("fetching {} days of insights from Meta...", days);
            let report = moneyball_core::fetch::fetch_snapshot(&strict, &token, days)
                .with_context(|| "fetch failed")?;
            for (name, rows) in &report.per_product {
                match rows {
                    Ok(n) => println!("  {:<40} {:>5} rows", name, n),
                    Err(e) => println!("  {:<40} no data - {}", name, e),
                }
            }
            match &report.creatives_error {
                None => println!(
                    "  creatives captured: {} ({} images cached, {} downloaded)",
                    report.creatives, report.assets, report.assets_downloaded
                ),
                Some(e) => println!(
                    "  warn: creatives capture failed ({}) - snapshot still ok",
                    e
                ),
            }
            match &report.adsets_error {
                None => println!(
                    "  adsets captured: {} (learning + targeting)",
                    report.adsets
                ),
                Some(e) => println!("  warn: adsets capture failed ({})", e),
            }
            match &report.leads_error {
                None => println!("  leads captured: {} (leads.json, 0600)", report.leads),
                Some(e) => println!(
                    "  warn: leads capture failed ({}) - Diff breakdown unavailable",
                    e
                ),
            }
            println!("snapshot written: {}", report.path.display());
        }
        Cmd::Funnel {
            product,
            by,
            window,
        } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::funnel::run(&strict, &product, &by, window, cli.date.as_deref())?;
        }
        Cmd::Report { date, window } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::report::run(&strict, date.as_deref().or(cli.date.as_deref()), window)?;
        }
        Cmd::Debug(args) => debug_cli::run(args, root)?,
        Cmd::Crm { cmd } => crm_cli::run(cmd, &cfg, cli.data_root.as_deref())?,
    }
    Ok(())
}
