//! The preset CRM connect as plain steps (connections::crm), the way the
//! Mac app drives them: the form, save, a test pull against a loopback
//! stand-in CRM, a stage map and the confirming pull. Hermetic: auth.json
//! is a temp file (MONEYBALL_AUTH_PATH, set once - the only test in this
//! process), and no request leaves the machine.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc;

use moneyball_core::config::WorkspaceConfig;
use moneyball_core::connections::crm::{
    preset_forms, save_preset, save_stage_map, test_pull, SpecSaved,
};
use moneyball_core::AppConfig;

const LEADS: &str = r#"[
  {"ad": "a1", "st": "Hot Lead", "at": "2026-11-05T10:00:00Z"},
  {"ad": "a1", "st": "Hot Lead", "at": "2026-11-06T10:00:00Z"},
  {"ad": "a1", "st": "Cold", "at": "2026-11-06T11:00:00Z"},
  {"ad": "a1", "st": "Contactable", "at": "2026-11-07T10:00:00Z"},
  {"ad": "", "st": "Contactable", "at": "2026-11-07T10:00:00Z"}
]"#;

const ADS: &str = r#"[{"campaign_id": "c1", "campaign_name": "C", "adset_id": "s1",
  "adset_name": "S", "ad_id": "a1", "ad_name": "A", "spend": "10", "impressions": "100",
  "clicks": "5", "inline_link_clicks": "4", "actions": [], "date_start": "2026-11-09",
  "date_stop": "2026-11-09", "_product": "P"}]"#;

/// A loopback CRM answering one request per canned (status, body); each
/// request's authorization header goes to the receiver. Its base URL.
fn serve(replies: Vec<(&'static str, &'static str)>) -> (String, mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let (mut conn, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut auth = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("authorization:") {
                    auth = v.trim().to_string();
                }
            }
            tx.send(auth).unwrap();
            let _ = write!(
                conn,
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
        }
    });
    (base, rx)
}

#[test]
fn a_preset_connects_through_its_steps_with_no_prompter() {
    let root = std::env::temp_dir().join(format!("mb-crm-steps-{}", std::process::id()));
    let mb = root.join(".moneyball");
    std::fs::create_dir_all(&mb).unwrap();
    let auth = root.join("auth.json");
    std::env::set_var("MONEYBALL_AUTH_PATH", &auth);
    let workspace: WorkspaceConfig = serde_json::from_value(serde_json::json!({
        "workspace_id": "ws-steps", "products": [{"name": "P", "ad_account": "act_1"}]
    }))
    .unwrap();
    let cfg = AppConfig {
        data_root: root.clone(),
        date: None,
        workspace: Some(workspace),
        agent: true,
    };
    let values = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };

    // The form: every preset, its fields, nothing stored yet.
    let forms = preset_forms(&cfg).unwrap();
    let ids: Vec<_> = forms.iter().map(|f| f.id).collect();
    assert_eq!(ids, ["leadzump", "leadsquared"]);
    let token = &forms[0].fields[0];
    assert_eq!((token.key, token.label), ("token", "API token"));
    assert!(token.secret && !token.stored && !token.refused);

    // A missing field stores nothing - not even the fields given.
    let e = save_preset(&cfg, "leadsquared", &values(&[("access_key", "ak-1")])).unwrap_err();
    assert_eq!(e.reason(), "Secret Key is required");
    assert!(!auth.exists(), "nothing stored on a refused save");
    assert!(save_preset(&cfg, "nope", &values(&[])).is_err());

    // A first connect stores the key (trimmed) and writes the preset's spec.
    let saved = save_preset(&cfg, "leadzump", &values(&[("token", " tok-1\n")])).unwrap();
    assert_eq!(saved, SpecSaved::Written);
    let spec = std::fs::read_to_string(mb.join("crm.toml")).unwrap();
    assert!(spec.contains("{secret:leadzump_token}") && !spec.contains("tok-1"));
    assert!(preset_forms(&cfg).unwrap()[0].fields[0].stored);

    // The CRM refused the stored key on the last pull: the form says so.
    std::fs::create_dir_all(mb.join("runs")).unwrap();
    std::fs::write(
        mb.join("runs").join("2026-11-10.json"),
        r#"{"schema": "moneyball.run/1", "workspace_id": "ws-steps", "date": "2026-11-10",
            "trigger": "manual", "started_at": "2026-11-10T01:00:00Z", "outcome": "degraded",
            "steps": [{"name": "crm", "status": "failed", "ms": 1, "error_kind": "crm_auth",
                       "fix": "moneyball crm secret leadzump_token"}]}"#,
    )
    .unwrap();
    let forms = preset_forms(&cfg).unwrap();
    assert!(forms[0].fields[0].refused);
    assert!(forms[1].fields.iter().all(|f| !f.refused));

    // The same CRM again (here a stand-in spec under its name): an empty
    // value keeps the stored key, and the spec - its stage map - stays.
    let (base, auths) = serve(vec![
        ("200 OK", LEADS),
        ("200 OK", LEADS),
        ("401 Unauthorized", ""),
    ]);
    let standin = format!(
        "name = \"leadzump\"\n\n[request]\nurl = \"{}/leads\"\n\n[request.headers]\n\
         authorization = \"secret:leadzump_token\"\n\n[map]\nroot = \"\"\nad_id = \"ad\"\n\
         stage = \"st\"\ndelivery = \"at\"\n",
        base
    );
    std::fs::write(mb.join("crm.toml"), &standin).unwrap();
    let saved = save_preset(&cfg, "leadzump", &values(&[("token", "")])).unwrap();
    assert_eq!(saved, SpecSaved::Kept);
    assert_eq!(std::fs::read_to_string(mb.join("crm.toml")).unwrap(), standin);

    // The test pull, in figures; the day's crm.json is written on a pass.
    let snap = mb.join("history").join("snap").join("2026-11-10");
    std::fs::create_dir_all(&snap).unwrap();
    std::fs::write(snap.join("ads_daily.json"), ADS).unwrap();
    let pull = test_pull(&cfg).unwrap();
    assert_eq!(auths.recv().unwrap(), "tok-1", "the stored key is sent");
    assert_eq!(pull.name, "leadzump");
    assert_eq!((pull.tickets, pull.pages, pull.dropped_no_ad_id), (4, 1, 1));
    assert!(pull.passed && snap.join("crm.json").is_file());
    assert_eq!(pull.unknown_stages, ["Hot Lead", "Cold"]);
    assert!(
        pull.lines.iter().any(|l| l.contains("4/4 tickets match")),
        "{:?}",
        pull.lines
    );

    // Map one stage, keep the other; the confirming pull knows it.
    let choices = [
        ("Hot Lead".to_string(), Some("Contactable".to_string())),
        ("Cold".to_string(), None),
    ];
    assert!(save_stage_map(&cfg, &choices).unwrap());
    let confirm = test_pull(&cfg).unwrap();
    assert!(confirm.passed);
    assert_eq!(confirm.unknown_stages, ["Cold"]);

    // A refused key stays a typed error with its fix, never the key.
    let e = test_pull(&cfg).unwrap_err();
    assert_eq!(e.kind(), "crm_auth");
    assert_eq!(e.fix().as_deref(), Some("moneyball crm secret leadzump_token"));
    let shown = [
        e.to_string(),
        serde_json::to_string(&preset_forms(&cfg).unwrap()).unwrap(),
        serde_json::to_string(&confirm).unwrap(),
    ];
    assert!(shown.iter().all(|s| !s.contains("tok-1")), "{:?}", shown);

    std::env::remove_var("MONEYBALL_AUTH_PATH");
    std::fs::remove_dir_all(&root).ok();
}
