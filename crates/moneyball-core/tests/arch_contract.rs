//! Mechanical enforcement of the ARCHITECTURE.md / AGENTS.md rules that
//! used to be prose-only: the network boundary (ARCHITECTURE §1), the
//! ASCII-in-authored-strings rule (AGENTS.md), and the file size caps
//! (ARCHITECTURE §3). Prose is advisory; this test is binding.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// All .rs files under `dir`, recursively.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(rust_sources(&p));
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// ARCHITECTURE §1: network code lives in exactly four core modules and
/// nowhere else; the tui and bin crates have no HTTP dependency at all.
#[test]
fn network_boundary() {
    let root = workspace_root();
    let allowed = [
        "crates/moneyball-core/src/meta.rs",
        "crates/moneyball-core/src/fetch/mod.rs",
        "crates/moneyball-core/src/fetch/adsets.rs",
        "crates/moneyball-core/src/fetch/creatives.rs",
        "crates/moneyball-core/src/fetch/leads.rs",
        "crates/moneyball-core/src/llm.rs",
        "crates/moneyball-core/src/crm/fetch.rs",
    ];
    let mut violations = Vec::new();
    for krate in ["moneyball", "moneyball-core", "moneyball-tui"] {
        for f in rust_sources(&root.join("crates").join(krate).join("src")) {
            let r = rel(&root, &f);
            if fs::read_to_string(&f).unwrap().contains("reqwest") && !allowed.contains(&r.as_str())
            {
                violations.push(r);
            }
        }
    }
    for krate in ["moneyball", "moneyball-tui"] {
        let manifest = root.join("crates").join(krate).join("Cargo.toml");
        if fs::read_to_string(&manifest).unwrap().contains("reqwest") {
            violations.push(rel(&root, &manifest));
        }
    }
    assert!(
        violations.is_empty(),
        "network code outside the four allowed core modules (ARCHITECTURE.md \u{a7}1):\n{}",
        violations.join("\n")
    );
}

/// docs/CLOUD_PLAN.md hedge: no ambient state in core. HOME/USERPROFILE
/// and current_dir reads live ONLY in config.rs (`config::home_dir` is
/// the sanctioned seam), so a future multi-tenant server can instantiate
/// N cores in one process without implicit "there is one user" bugs.
#[test]
fn ambient_state_only_in_config() {
    let root = workspace_root();
    let needles = ["\"HOME\"", "\"USERPROFILE\"", "current_dir("];
    let mut violations = Vec::new();
    for f in rust_sources(&root.join("crates/moneyball-core/src")) {
        let r = rel(&root, &f);
        if r == "crates/moneyball-core/src/config.rs" {
            continue;
        }
        let src = fs::read_to_string(&f).unwrap();
        for (i, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if needles.iter().any(|n| line.contains(n)) {
                violations.push(format!("{}:{}: {}", r, i + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "ambient-state read outside config.rs (route through config::home_dir; \
         cwd resolution belongs to the binary edge):\n{}",
        violations.join("\n")
    );
}

/// AGENTS.md: ASCII only in strings we author - some terminal fonts can't
/// render multibyte. Comment lines are exempt (they may cite glyphs).
#[test]
fn ascii_only_outside_comments() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for krate in ["moneyball", "moneyball-core", "moneyball-tui"] {
        for dir in ["src", "examples"] {
            for f in rust_sources(&root.join("crates").join(krate).join(dir)) {
                for (i, line) in fs::read_to_string(&f).unwrap().lines().enumerate() {
                    if !line.trim_start().starts_with("//") && !line.is_ascii() {
                        violations.push(format!("{}:{}: {}", rel(&root, &f), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "non-ASCII outside comments (AGENTS.md ASCII rule; use \\u{{...}} escapes \
         or ASCII replacements):\n{}",
        violations.join("\n")
    );
}

/// ARCHITECTURE §3: ~400 lines per file. Existing offenders are frozen at
/// a ceiling and may only shrink - delete an entry when its file drops
/// under 400; never raise a ceiling or add an entry.
#[test]
fn file_size_ratchet() {
    let root = workspace_root();
    let frozen: &[(&str, usize)] = &[
        ("crates/moneyball-core/src/agent.rs", 600),
        ("crates/moneyball-core/src/crm/fetch.rs", 500),
        ("crates/moneyball-core/src/crm/mod.rs", 550),
        ("crates/moneyball-core/src/llm.rs", 850),
        ("crates/moneyball-tui/src/chat.rs", 500),
        ("crates/moneyball-tui/src/commands.rs", 750),
        ("crates/moneyball-tui/src/event.rs", 750),
        ("crates/moneyball-tui/src/setup/mod.rs", 1000),
        ("crates/moneyball-tui/src/setup/render_steps.rs", 650),
    ];
    let mut violations = Vec::new();
    for krate in ["moneyball", "moneyball-core", "moneyball-tui"] {
        for f in rust_sources(&root.join("crates").join(krate).join("src")) {
            let r = rel(&root, &f);
            let lines = fs::read_to_string(&f).unwrap().lines().count();
            let cap = frozen
                .iter()
                .find(|(name, _)| *name == r)
                .map_or(400, |(_, cap)| *cap);
            if lines > cap {
                violations.push(format!("{}: {} lines (cap {})", r, lines, cap));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "files over the size cap (ARCHITECTURE.md \u{a7}3 - split, don't waive):\n{}",
        violations.join("\n")
    );
}
