//! moneyball binary - clap dispatch + session handling.

mod connect_cli;
mod crm_cli;
mod debug_cli;
mod refresh_cli;
mod schedule_cli;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use moneyball_core::AppConfig;

#[derive(Parser, Debug)]
#[command(name = "moneyball", version, about = "Read-only Meta-ads advisor CLI")]
struct Cli {
    /// Path to the data workspace (default: the current directory when it
    /// holds one, else the one setup saved).
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
    /// Open the moneyball app (what plain `moneyball` does).
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
    /// Pull today's Meta snapshot only - no CRM, no report (refresh is the
    /// daily path).
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
    /// Before cutting a product: where its CRM leads die, tired ads, the
    /// last 7 days against the days before, what changed in its account.
    Diagnose {
        /// Product name (as configured in the workspace).
        product: String,
    },
    /// The CRM: its status, connecting one, pulling or importing leads,
    /// checking a crm.json against the contract.
    Crm {
        #[command(subcommand)]
        cmd: crm_cli::CrmCmd,
    },
    /// Build the daily report from a snapshot, no network: report.json and
    /// the HTML page beside it.
    Report {
        /// Snapshot date YYYY-MM-DD (default: latest).
        #[arg(long)]
        date: Option<String>,
        /// Trailing complete days the report covers (ending the day
        /// before the snapshot date).
        #[arg(long, default_value_t = 1)]
        window: u32,
    },
    /// Daily refresh: pull Meta, then CRM for the same day, then write
    /// yesterday's report. Exit 0 ok, 2 degraded, 1 failed.
    Refresh {
        /// Pull Meta again even when today's snapshot already has it.
        #[arg(long)]
        force: bool,
        /// Trailing days to pull, ending yesterday.
        #[arg(long, default_value_t = 28)]
        days: u32,
        /// Set by the morning schedule, so its run record says so.
        #[arg(long, hide = true)]
        scheduled: bool,
    },
    /// The morning refresh: --at HH:MM installs it (launchd), --off removes
    /// it, --print shows the plist without installing; alone, shows it.
    Schedule {
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        off: bool,
        #[arg(long)]
        print: bool,
    },
    /// The last 14 data days: what landed for each and how its refresh went.
    Status,
    /// Where every source stands (ads, leads, advisor) and its fix;
    /// `connect crm` connects a CRM.
    Connect {
        #[command(subcommand)]
        what: Option<connect_cli::Connect>,
    },
    /// Saved sessions: replay one with an audit of its invariants; file,
    /// list and archive bug reports.
    Debug(debug_cli::DebugArgs),
}

fn main() {
    let cli = Cli::parse();
    let agent = AppConfig::resolve_optional(cli.data_root.as_deref(), None).agent;
    if let Err(e) = run(cli) {
        // MB_AGENT=1 reads stdout as JSON: a failure is JSON too, with
        // its kind and the one command that fixes it when there is one.
        if agent {
            let core = e
                .chain()
                .find_map(|c| c.downcast_ref::<moneyball_core::Error>());
            println!(
                "{}",
                serde_json::json!({"error": {
                    "kind": core.map_or("other", |c| c.kind()),
                    "message": core.map_or_else(|| format!("{:#}", e), |c| c.reason()),
                    "fix": core.and_then(|c| c.fix()),
                }})
            );
        } else {
            eprintln!("Error: {:?}", e);
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
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
                anyhow::anyhow!("no Meta token saved - open moneyball and run /setup meta")
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
            match &report.changes_error {
                None => println!("  account changes captured: {}", report.changes),
                Some(e) => println!("  warn: account change log failed ({})", e),
            }
            match &report.ads_7d_error {
                None => println!("  7-day reach captured: {} ads", report.ads_7d),
                Some(e) => println!("  warn: 7-day reach failed ({})", e),
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
        Cmd::Diagnose { product } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::diagnose::run(&strict, &product, cli.date.as_deref())?;
        }
        Cmd::Report { date, window } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::report::run(&strict, date.as_deref().or(cli.date.as_deref()), window)?;
        }
        Cmd::Refresh {
            force,
            days,
            scheduled,
        } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
            refresh_cli::run(&strict, force, days, scheduled)?;
        }
        Cmd::Schedule { at, off, print } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
            schedule_cli::run(&strict, at.as_deref(), off, print)?;
        }
        Cmd::Connect { what } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
            connect_cli::run(&strict, what)?;
        }
        Cmd::Status => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
            let days = moneyball_core::status::days(&strict, 14);
            if strict.agent {
                println!("{}", serde_json::to_string(&days)?);
            } else {
                for line in moneyball_core::status::lines(&strict, &days) {
                    println!("{}", line);
                }
            }
        }
        Cmd::Debug(args) => debug_cli::run(args, root)?,
        Cmd::Crm { cmd } => crm_cli::run(cmd, &cfg, cli.data_root.as_deref())?,
    }
    Ok(())
}
