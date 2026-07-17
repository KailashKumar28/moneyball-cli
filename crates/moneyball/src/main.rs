//! moneyball binary - clap dispatch + session handling.

mod connect_flow;

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
    /// Write an annotated crm.toml starter spec into the workspace.
    Init,
    /// Run the crm.toml spec: pull leads, validate, write crm.json.
    Fetch {
        /// How many trailing days of leads to request.
        #[arg(long, default_value_t = 28)]
        days: u32,
    },
    /// Guided setup: probe your CRM, let the LLM draft crm.toml once,
    /// validate against a live sample, save on your approval.
    Connect,
    /// Import a CSV lead export through the crm.toml [map] (column names).
    Import {
        /// Path to the CSV file (header row required).
        file: std::path::PathBuf,
    },
    /// Store a CRM secret (value read from stdin, never from argv).
    Secret {
        /// Name referenced from crm.toml as "secret:<name>".
        name: String,
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

    // --resume <id>: reopen that session's log and replay its transcript.
    if let Some(id) = cli.resume.clone() {
        let (log, items) = moneyball_core::session::SessionLog::open(&id)
            .with_context(|| format!("no session found with id '{}'", id))?;
        println!(
            "resuming session {} ({} items, started {})",
            log.meta.id,
            items.len(),
            log.meta.started_at
        );
        return moneyball_tui::run_with(Some((log, items)));
    }

    // -c / --continue: reopen the latest session if one exists.
    let resume_session = if cli.continue_last {
        match moneyball_core::session::latest_id()? {
            Some(id) => Some(moneyball_core::session::SessionLog::open(&id)?),
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
        Cmd::Funnel {
            product,
            by,
            window,
        } => {
            let strict = AppConfig::resolve(cli.data_root.as_deref(), cli.date.as_deref())?;
            moneyball_core::funnel::run(&strict, &product, &by, window, cli.date.as_deref())?;
        }
        Cmd::Crm { cmd } => match cmd {
            CrmCmd::Contract => print!("{}", moneyball_core::crm::CONTRACT_MD),
            CrmCmd::Check { file } => {
                if !run_crm_check(&cfg, &file)? {
                    std::process::exit(1);
                }
            }
            CrmCmd::Init => {
                let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
                let path = moneyball_core::crm::fetch::spec_path(&strict);
                if path.exists() {
                    println!("crm.toml already exists: {}", path.display());
                } else {
                    std::fs::write(&path, moneyball_core::crm::source::TEMPLATE_TOML)?;
                    println!("wrote starter spec: {}", path.display());
                    println!("edit it for your CRM, then run: moneyball crm fetch");
                }
            }
            CrmCmd::Fetch { days } => {
                let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
                let r = moneyball_core::crm::fetch::fetch_crm(&strict, days)
                    .with_context(|| "crm fetch failed")?;
                println!(
                    "crm fetch ({}): {} ad-attributed ticket(s) over {} page(s){}",
                    r.name,
                    r.tickets,
                    r.pages,
                    if r.dropped_no_ad_id > 0 {
                        format!(" ({} organic/direct dropped - no ad id)", r.dropped_no_ad_id)
                    } else {
                        String::new()
                    }
                );
                print_ingest_outcome(&r);
            }
            CrmCmd::Connect => {
                let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
                connect_flow::run(&strict)?;
            }
            CrmCmd::Import { file } => {
                let strict = AppConfig::resolve(cli.data_root.as_deref(), None)?;
                let r = moneyball_core::crm::fetch::import_csv(&strict, &file)
                    .with_context(|| "crm import failed")?;
                println!(
                    "crm import ({}): {} ad-attributed ticket(s) from {}{}",
                    r.name,
                    r.tickets,
                    file.display(),
                    if r.dropped_no_ad_id > 0 {
                        format!(" ({} organic/direct dropped - no ad id)", r.dropped_no_ad_id)
                    } else {
                        String::new()
                    }
                );
                print_ingest_outcome(&r);
            }
            CrmCmd::Secret { name } => {
                eprintln!("paste the value for \"{}\" and press Enter:", name);
                let mut value = String::new();
                std::io::stdin().read_line(&mut value)?;
                let value = value.trim();
                if value.is_empty() {
                    eprintln!("empty value - nothing stored");
                    std::process::exit(1);
                }
                moneyball_core::secrets::store_crm_key(&name, value)?;
                println!("stored CRM secret \"{}\" in ~/.moneyball/auth.json", name);
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
    let report = moneyball_core::crm::check_with_workspace(cfg, &parsed);
    println!("crm check: {} ({} tickets)", file.display(), report.tickets);
    print_check_lines(&report);
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

/// One prompt -> one trimmed line. On a real terminal this is a
/// rustyline editor (arrow keys, backspace, paste all behave); piped
/// stdin (tests, scripts) falls back to a plain line read.
pub(crate) fn ask(prompt: &str) -> Result<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        let mut rl = rustyline::DefaultEditor::new()?;
        return match rl.readline(prompt) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(
                rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof,
            ) => Ok(String::new()),
            Err(e) => Err(e.into()),
        };
    }
    use std::io::Write;
    eprint!("{}", prompt);
    std::io::stderr().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn print_ingest_outcome(r: &moneyball_core::crm::fetch::CrmFetchReport) {
    print_check_lines(&r.check);
    match &r.path {
        Some(p) => println!("PASS - crm.json written: {}", p.display()),
        None => {
            println!("FAIL - validation errors above; crm.json NOT written");
            std::process::exit(1);
        }
    }
}

pub(crate) fn print_check_lines(report: &moneyball_core::crm::CheckReport) {
    for line in &report.info {
        println!("  -     {}", line);
    }
    for line in &report.warnings {
        println!("  warn  {}", line);
    }
    for line in &report.errors {
        println!("  error {}", line);
    }
}
