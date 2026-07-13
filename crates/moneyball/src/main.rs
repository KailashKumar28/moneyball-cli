//! moneyball binary - clap dispatch.

use anyhow::Result;
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

    /// Emit JSON instead of plain text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Start the interactive REPL (default if no sub-command).
    Repl,
    /// 7-day portfolio brief - per-product summary + feasibility math.
    Brief {
        /// Snapshot date YYYY-MM-DD (overrides --date).
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
    // Use resolve_optional so the TUI can show the setup wizard when no
    // workspace config exists. Sub-commands that strictly need a config
    // re-resolve with the strict variant.
    let cfg = AppConfig::resolve_optional(cli.data_root.as_deref(), cli.date.as_deref());

    match cli.cmd.unwrap_or(Cmd::Repl) {
        Cmd::Repl => {
            moneyball_tui::run()?;
        }
        Cmd::Brief { date } => {
            // Strict re-resolve: brief needs an actual workspace.
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