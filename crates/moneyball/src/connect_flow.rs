//! Interactive `crm connect` - the keys-only catalog flow.
//!
//! Pick your CRM -> paste credentials (with where-to-find help) -> a
//! live test pull gates the save (Fivetran connect-card pattern), then
//! observed-but-unmapped stage names get mapped interactively (Merge
//! field-mapping pattern). Unknown CRMs fall back to paste-a-curl +
//! LLM-drafted spec.

use anyhow::Result;

use moneyball_core::crm::{connect, fetch as crm_fetch, presets, source};
use moneyball_core::AppConfig;

use crate::{ask, print_check_lines};

pub fn run(cfg: &AppConfig) -> Result<()> {
    let catalog = presets::catalog();
    eprintln!("connect which CRM?");
    for (i, p) in catalog.iter().enumerate() {
        eprintln!("  {}) {}", i + 1, p.display);
    }
    eprintln!(
        "  {}) other (paste a curl command; AI drafts the spec)",
        catalog.len() + 1
    );
    let choice: usize = ask("> ")?.parse().unwrap_or(0);
    match choice {
        n if (1..=catalog.len()).contains(&n) => preset_flow(cfg, &catalog[n - 1]),
        n if n == catalog.len() + 1 => custom_flow(cfg),
        _ => anyhow::bail!("pick a number between 1 and {}", catalog.len() + 1),
    }
}

/// Catalog path: credentials only, then test, then stage mapping.
fn preset_flow(cfg: &AppConfig, p: &presets::Preset) -> Result<()> {
    for s in p.secrets {
        let v = ask(&format!("{} ({}): ", s.label, s.help))?;
        if v.is_empty() {
            anyhow::bail!("{} is required", s.label);
        }
        moneyball_core::secrets::store_crm_key(&p.secret_name(s), &v)?;
    }
    let path = crm_fetch::spec_path(cfg);
    std::fs::write(&path, p.render())?;
    eprintln!("wrote {} ({})", path.display(), p.note);
    test_and_map_stages(cfg, &path)
}

/// Unknown CRM: paste the curl from its API docs; header values are
/// stored as secrets (the spec on disk only carries refs); the LLM
/// drafts the field map from a live sample, dry-run gated as before.
fn custom_flow(cfg: &AppConfig) -> Result<()> {
    let name = ask("CRM name (e.g. acme-crm): ")?;
    if name.is_empty() {
        anyhow::bail!("a name is required");
    }
    eprintln!("paste the 'list leads' request from your CRM's API docs -");
    eprintln!("a full curl command (POST bodies fine) or just the URL:");
    let line = ask("> ")?;
    let mut input = connect::ConnectInput::from_curl(name.clone(), &line)?;
    for (k, v) in input.headers.iter_mut() {
        let sname = format!("{}_{}", name, k.to_lowercase().replace('-', "_"));
        moneyball_core::secrets::store_crm_key(&sname, v)?;
        *v = format!("secret:{}", sname);
        eprintln!("  header {} stored as secret:{}", k, sname);
    }
    // Query params carry credentials in many CRMs (LeadSquared puts
    // accessKey/secretKey there). Secretize per param - dates and flags
    // must stay literal so the LLM can turn them into templates.
    for (k, v) in input.query.iter_mut() {
        let looks_secret = looks_credential(k);
        let hint = if looks_secret { "[Y/n]" } else { "[y/N]" };
        let a = ask(&format!(
            "query param \"{}\" - store its value as a secret? {}: ",
            k, hint
        ))?;
        let yes = if a.is_empty() {
            looks_secret
        } else {
            a.eq_ignore_ascii_case("y")
        };
        if yes {
            let sname = format!("{}_{}", name, k.to_lowercase().replace('-', "_"));
            moneyball_core::secrets::store_crm_key(&sname, v)?;
            *v = format!("secret:{}", sname);
            eprintln!("  query {} stored as secret:{}", k, sname);
        }
    }

    eprintln!("probing the endpoint for a sample...");
    let sample = connect::probe_sample(&input)?;
    let preview = sample.to_string();
    eprintln!(
        "sample received ({} bytes): {}...",
        preview.len(),
        source::truncate_chars(&preview, 300)
    );

    eprintln!("drafting crm.toml with your configured LLM (a truncated sample is sent to it)...");
    let draft = connect::draft_spec(cfg, &input, &sample)?;
    println!(
        "\n----- drafted crm.toml -----\n{}\n----------------------------",
        draft
    );
    let (n, report) = connect::dry_run(cfg, &draft, &sample)?;
    println!("dry run over the sample: {} record(s)", n);
    print_check_lines(&report);
    let verdict = if report.passed() {
        "PASS"
    } else {
        "has errors"
    };
    let path = crm_fetch::spec_path(cfg);
    if !ask(&format!(
        "dry run {} - save to {}? [y/N]: ",
        verdict,
        path.display()
    ))?
    .eq_ignore_ascii_case("y")
    {
        println!("not saved. re-run crm connect, or hand-edit via: moneyball crm init");
        return Ok(());
    }
    std::fs::write(&path, &draft)?;
    test_and_map_stages(cfg, &path)
}

/// The shared tail: live test pull, then interactive stage mapping for
/// whatever stage names the real data contained, then a confirming pull.
fn test_and_map_stages(cfg: &AppConfig, spec_path: &std::path::Path) -> Result<()> {
    eprintln!("running a live test pull (7 days)...");
    let r = crm_fetch::fetch_crm(cfg, 7)?;
    println!(
        "test pull ({}): {} tickets over {} page(s)",
        r.name, r.tickets, r.pages
    );
    print_check_lines(&r.check);
    if r.path.is_none() {
        println!(
            "FAIL - fix the errors above (edit {}) and re-run: moneyball crm fetch",
            spec_path.display()
        );
        std::process::exit(1);
    }
    if !r.check.unknown_stages.is_empty() {
        let pairs = map_stages_interactively(&r.check.unknown_stages)?;
        if !pairs.is_empty() {
            let spec = std::fs::read_to_string(spec_path)?;
            std::fs::write(spec_path, source::add_stage_mappings(&spec, &pairs)?)?;
            eprintln!("stage map saved - confirming with another pull...");
            let r2 = crm_fetch::fetch_crm(cfg, 7)?;
            print_check_lines(&r2.check);
        }
    }
    println!("connected. daily pull: moneyball crm fetch --days 28  (cron-able)");
    Ok(())
}

/// Does this query-param name look like it carries a credential?
/// Drives only the DEFAULT of the store-as-secret prompt above.
fn looks_credential(key: &str) -> bool {
    let k = key.to_lowercase();
    ["key", "token", "secret", "auth", "sig", "pass", "pwd"]
        .iter()
        .any(|n| k.contains(n))
}

const CANONICAL: &[&str] = moneyball_core::crm::CANONICAL_STAGES;

/// One question per observed unknown stage, Merge-style: map it to a
/// canonical stage or keep it (kept stages count as leads, never as
/// qualified/visit/booking).
fn map_stages_interactively(unknown: &[String]) -> Result<Vec<(String, String)>> {
    eprintln!("your CRM uses stage names moneyball doesn't know. map each one:");
    for (i, c) in CANONICAL.iter().enumerate() {
        eprintln!("  {}) {}", i + 1, c);
    }
    eprintln!("  0) keep as-is (counts as a lead, never as qualified)");
    let mut pairs = Vec::new();
    for stage in unknown {
        let a = ask(&format!("\"{}\" -> [0-{}]: ", stage, CANONICAL.len()))?;
        match a.parse::<usize>() {
            Ok(n) if (1..=CANONICAL.len()).contains(&n) => {
                pairs.push((stage.clone(), CANONICAL[n - 1].to_string()));
            }
            _ => eprintln!("  keeping \"{}\" as-is", stage),
        }
    }
    Ok(pairs)
}
