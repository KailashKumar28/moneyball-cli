//! Nothing the advisor reads here names a slash command: this app has
//! none, and the model hands the user whatever it is told. Checked over
//! the full system prompt and every tool output for the demo workspace,
//! with CRM data and without.

use moneyball_core::agent::tool_text::{brief_text, diagnose_output, funnel_output};
use moneyball_core::funnel::By;
use moneyball_core::{brief, snapshot, AppConfig};

use super::demo;
use crate::action::tests::slash_command;
use crate::front_end::{crm_note, system_prompt, DESKTOP};

/// Everything a turn could put in front of the model for `cfg`.
fn model_reads(cfg: &AppConfig) -> Vec<(String, String)> {
    let loaded = brief::load(cfg);
    let note = crm_note(cfg);
    let date = loaded.brief.as_ref().map(|b| b.snapshot_date.clone());
    let prompt = system_prompt(cfg, date.as_deref(), note.as_deref());
    // The workspace's own path is not a command.
    let root = cfg.data_root.display().to_string();
    let mut out = vec![("system prompt".to_string(), prompt.replace(&root, "ROOT"))];
    let Some(b) = &loaded.brief else {
        return out;
    };
    out.push(("brief".into(), brief_text(b, note.as_deref())));
    let snap = snapshot::load(&cfg.snap_for(None).unwrap()).unwrap();
    for product in ["Namma Mane", "Cityville"] {
        for by in [By::Campaign, By::Adset, By::Ad] {
            let text = funnel_output(cfg, &snap, product, by, &DESKTOP.say);
            out.push((format!("funnel {} by {}", product, by.name()), text));
        }
        let text = diagnose_output(cfg, &snap, product, &DESKTOP.say);
        out.push((format!("diagnose {}", product), text));
    }
    out
}

#[test]
fn the_advisor_is_never_told_a_slash_command() {
    let cases = [
        ("say-crm", true, true),
        ("say-nocrm", true, false),
        ("say-empty", false, false),
    ];
    for (name, snapshots, crm) in cases {
        let cfg = demo::workspace(name, snapshots, crm);
        let reads = model_reads(&cfg);
        assert_eq!(reads.len(), if snapshots { 10 } else { 1 });
        if snapshots && !crm {
            assert!(
                reads[0].1.contains("`moneyball connect crm` in Terminal"),
                "{}",
                reads[0].1
            );
        }
        for (what, text) in &reads {
            assert_eq!(slash_command(text), None, "{} ({}):\n{}", what, name, text);
        }
        demo::remove(&cfg);
    }
}
