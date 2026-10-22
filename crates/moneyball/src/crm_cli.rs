//! `moneyball crm ...`: status, contract, check, init, fetch, connect,
//! import, secret. Split from main.rs (size cap).

use anyhow::{Context, Result};
use clap::Subcommand;

use moneyball_core::AppConfig;

#[derive(Subcommand, Debug)]
pub(crate) enum CrmCmd {
    /// Where CRM data stands: spec, secret, newest crm.json, next step.
    Status,
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

/// The workspace a CRM key belongs to: keys are per client, so storing
/// one outside a workspace has nowhere to go.
pub(crate) fn workspace_of(cfg: &AppConfig) -> Result<&str> {
    cfg.workspace_id().context(
        "no workspace here - CRM keys belong to a client's workspace; run this in its folder",
    )
}

pub(crate) fn run(cmd: CrmCmd, cfg: &AppConfig, data_root: Option<&str>) -> Result<()> {
    match cmd {
        CrmCmd::Status => {
            for line in moneyball_core::crm::status::status_lines(cfg)? {
                println!("{}", line);
            }
        }
        CrmCmd::Contract => print!("{}", moneyball_core::crm::CONTRACT_MD),
        CrmCmd::Check { file } => {
            if !run_crm_check(cfg, &file)? {
                std::process::exit(1);
            }
        }
        CrmCmd::Init => {
            let strict = AppConfig::resolve(data_root, None)?;
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
            let strict = AppConfig::resolve(data_root, None)?;
            let r = moneyball_core::crm::fetch::fetch_crm(&strict, days)
                .with_context(|| "crm fetch failed")?;
            if strict.agent {
                return ingest_json(&r);
            }
            println!(
                "crm fetch ({}): {} ad-attributed ticket(s) over {} page(s){}",
                r.name,
                r.tickets,
                r.pages,
                if r.dropped_no_ad_id > 0 {
                    format!(
                        " ({} organic/direct dropped - no ad id)",
                        r.dropped_no_ad_id
                    )
                } else {
                    String::new()
                }
            );
            print_ingest_outcome(&r);
        }
        CrmCmd::Connect => {
            let strict = AppConfig::resolve(data_root, None)?;
            crate::connect_cli::crm(&strict)?;
        }
        CrmCmd::Import { file } => {
            let strict = AppConfig::resolve(data_root, None)?;
            let r = moneyball_core::crm::ingest::import_csv(&strict, &file)
                .with_context(|| "crm import failed")?;
            if strict.agent {
                return ingest_json(&r);
            }
            println!(
                "crm import ({}): {} ad-attributed ticket(s) from {}{}",
                r.name,
                r.tickets,
                file.display(),
                if r.dropped_no_ad_id > 0 {
                    format!(
                        " ({} organic/direct dropped - no ad id)",
                        r.dropped_no_ad_id
                    )
                } else {
                    String::new()
                }
            );
            print_ingest_outcome(&r);
        }
        CrmCmd::Secret { name } => {
            // Where it goes before what it is: no workspace, no prompt.
            let ws = workspace_of(cfg)?;
            let value = ask_secret(&format!("paste the value for \"{}\" (hidden): ", name))?
                .unwrap_or_default();
            if value.is_empty() {
                eprintln!("empty value - nothing stored");
                std::process::exit(1);
            }
            moneyball_core::secrets::store_crm_key(ws, &name, &value)?;
            println!(
                "stored CRM secret \"{}\" for this workspace in ~/.moneyball/auth.json",
                name
            );
        }
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
            let why = format!("{} is not valid JSON: {}", file.display(), e);
            match cfg.agent {
                true => println!("{}", serde_json::json!({"passed": false, "errors": [why]})),
                false => println!("FAIL: {}", why),
            }
            return Ok(false);
        }
    };
    let report = moneyball_core::crm::check_with_workspace(cfg, &parsed);
    if cfg.agent {
        let mut v = serde_json::to_value(&report)?;
        v["passed"] = report.passed().into();
        println!("{}", v);
        return Ok(report.passed());
    }
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
pub(crate) fn ask(prompt: &str) -> Result<Option<String>> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        let mut rl = rustyline::DefaultEditor::new()?;
        return readline(rl.readline(prompt));
    }
    use std::io::Write;
    eprint!("{}", prompt);
    std::io::stderr().flush()?;
    let mut s = String::new();
    // End of input is a cancel, never an empty answer.
    Ok((std::io::stdin().read_line(&mut s)? > 0).then(|| s.trim().to_string()))
}

/// Ctrl-C and Ctrl-D cancel (None); an empty line is an empty answer.
fn readline(r: rustyline::Result<String>) -> Result<Option<String>> {
    match r {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(
            rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof,
        ) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `ask` for a secret: on a terminal every character typed or pasted shows
/// as `*` and nothing enters the line history (rustyline's masking
/// highlighter); piped stdin has no screen to hide it from.
pub(crate) fn ask_secret(prompt: &str) -> Result<Option<String>> {
    use rustyline::config::Configurer;
    use rustyline::highlight::{CmdKind, Highlighter};
    use std::io::IsTerminal;

    struct Mask;
    impl Highlighter for Mask {
        fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
            "*".repeat(line.chars().count()).into()
        }
        fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
            kind != CmdKind::MoveCursor
        }
    }
    impl rustyline::completion::Completer for Mask {
        type Candidate = String;
    }
    impl rustyline::hint::Hinter for Mask {
        type Hint = String;
    }
    impl rustyline::validate::Validator for Mask {}
    impl rustyline::Helper for Mask {}

    if !std::io::stdin().is_terminal() {
        return ask(prompt);
    }
    let mut rl = rustyline::Editor::<Mask, rustyline::history::DefaultHistory>::new()?;
    rl.set_helper(Some(Mask));
    rl.set_color_mode(rustyline::ColorMode::Forced);
    rl.set_auto_add_history(false);
    readline(rl.readline(prompt))
}

/// MB_AGENT=1: the pull or import as JSON; exit 1 when crm.json was not
/// written, as in the text path.
fn ingest_json(r: &moneyball_core::crm::fetch::CrmFetchReport) -> Result<()> {
    println!("{}", serde_json::to_string(r)?);
    if r.path.is_none() {
        std::process::exit(1);
    }
    Ok(())
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
    for line in report.lines() {
        println!("{}", line);
    }
}
