//! moneyball binary - clap dispatch + session handling.

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
    /// Connect CRM data - print the crm.json contract, validate an export.
    Crm {
        #[command(subcommand)]
        cmd: CrmCmd,
    },
}

#[derive(Subcommand, Debug)]
enum CrmCmd {
    /// Print the crm.json contract (paste into your CRM's coding agent).
    Contract,
    /// Validate a crm.json export against the contract. Exit 0 = PASS.
    Check {
        /// Path to the crm.json file to validate.
        file: std::path::PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // --list: print saved sessions and exit.
    if cli.list {
        let sessions = moneyball_core::session::list()?;
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

    // --resume <id>: load that session before handing off to the REPL.
    if let Some(id) = cli.resume.clone() {
        let s = moneyball_core::session::load(&id)
            .with_context(|| format!("no session found with id '{}'", id))?;
        println!(
            "resuming session {} (started {})",
            s.meta.id, s.meta.started_at
        );
        return moneyball_tui::run_with(Some(s));
    }

    // -c / --continue: load latest session if one exists; else fall back to new.
    let resume_session = if cli.continue_last {
        match moneyball_core::session::latest()? {
            Some(s) => Some(s),
            None => {
                eprintln!("no previous session to continue - starting a fresh one");
                None
            }
        }
    } else {
        None
    };

    let cfg = AppConfig::resolve_optional(cli.data_root.as_deref(), cli.date.as_deref());

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
            println!("fetching {} days of insights from Meta...", days);
            let report = moneyball_core::fetch::fetch_snapshot(&strict, days)
                .with_context(|| "fetch failed")?;
            for (name, n) in &report.per_product {
                println!("  {:<40} {:>5} rows", name, n);
            }
            println!("snapshot written: {}", report.path.display());
        }
        Cmd::Crm { cmd } => match cmd {
            CrmCmd::Contract => print!("{}", moneyball_core::crm::CONTRACT_MD),
            CrmCmd::Check { file } => {
                if !run_crm_check(&cfg, &file)? {
                    std::process::exit(1);
                }
            }
        },
    }
    Ok(())
}

/// Validate a crm.json export; print the report. Returns pass/fail.
fn run_crm_check(cfg: &AppConfig, file: &std::path::Path) -> Result<bool> {
    let raw =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL: {} is not valid JSON: {}", file.display(), e);
            return Ok(false);
        }
    };
    let stages = cfg
        .workspace
        .as_ref()
        .map(|w| w.crm.stages.clone())
        .unwrap_or_default();
    let snap = cfg
        .snap_for(None)
        .ok()
        .and_then(|p| moneyball_core::snapshot::load(&p).ok());
    let report = moneyball_core::crm::check(&parsed, &stages, snap.as_ref());
    println!("crm check: {} ({} tickets)", file.display(), report.tickets);
    for line in &report.info {
        println!("  -     {}", line);
    }
    for line in &report.warnings {
        println!("  warn  {}", line);
    }
    for line in &report.errors {
        println!("  error {}", line);
    }
    if report.passed() {
        println!("PASS");
    } else {
        println!(
            "FAIL ({} error line(s)) - see moneyball crm contract",
            report.errors.len()
        );
    }
    Ok(report.passed())
}
