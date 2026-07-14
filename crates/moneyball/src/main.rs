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
        println!("resuming session {} (started {})", s.meta.id, s.meta.started_at);
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
            moneyball_tui::run_with(resume_session)?;
        }
        Cmd::Brief { date } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::brief::run(&strict, date.as_deref().or(cli.date.as_deref()))?;
        }
        Cmd::Snapshot { check: _ } => {
            let p = cfg.snap_for(cli.date.as_deref())?;
            println!("snapshot ok: {}", p.display());
        }
    }
    Ok(())
}