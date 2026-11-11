//! Nothing the advisor reads here names a slash command: this app has
//! none, and the model hands the user whatever it is told. Checked over
//! the full system prompt and every tool output for the demo workspace,
//! with CRM data and without. Nor does it send anyone to a screen that
//! would only sit empty, nor does any pinned fixture put a slash command
//! on screen.

use moneyball_core::agent::tool_text::{brief_text, diagnose_output, funnel_output, no_snapshot};
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
        // What every tool answers with no snapshot to read.
        let text = no_snapshot(&DESKTOP.say, DESKTOP.self_heal);
        out.push(("a tool with no snapshot".into(), text));
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
        assert_eq!(reads.len(), if snapshots { 10 } else { 2 });
        if !snapshots {
            // The dead end: nothing here pulls data but Refresh.
            assert!(reads[0].1.contains("the Refresh button pulls one"));
            assert_eq!(
                reads[1].1,
                "no snapshot on disk - tell the user to run the Refresh button before asking for numbers"
            );
        }
        if snapshots && !crm {
            assert!(
                reads[0].1.contains("`moneyball connect crm` in Terminal"),
                "{}",
                reads[0].1
            );
        }
        for (what, text) in &reads {
            assert_eq!(slash_command(text), None, "{} ({}):\n{}", what, name, text);
            assert!(
                !text.contains("self-heal"),
                "{} ({}):\n{}",
                what,
                name,
                text
            );
        }
        demo::remove(&cfg);
    }
}

/// Every string of `v` the interface could show. Left out: a path, the
/// report's own page (a document of its own, with links), and Today's
/// `health`, which is core's record as data (the interface reads its
/// state only; the run it holds is shown through `last_run`).
fn shown<'a>(key: &str, v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    use serde_json::Value;
    match v {
        _ if key == "html" || key == "health" => {}
        Value::String(s) if !s.starts_with(super::SHOWN_ROOT) => out.push(s),
        Value::Array(all) => all.iter().for_each(|x| shown(key, x, out)),
        Value::Object(map) => map.iter().for_each(|(k, x)| shown(k, x, out)),
        _ => {}
    }
}

/// AGENTS.md: the desktop never tells a user to type a slash command.
/// Held over every pinned fixture, so a sentence core words for the
/// terminal cannot reach a screen untranslated.
pub fn nothing_shown_is_a_slash_command(pins: &[(&'static str, String)]) {
    for (name, json) in pins {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut strings = Vec::new();
        shown("", &v, &mut strings);
        for s in strings {
            assert_eq!(slash_command(s), None, "{}.json shows: {}", name, s);
        }
    }
}
