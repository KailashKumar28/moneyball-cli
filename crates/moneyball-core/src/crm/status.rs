//! `moneyball crm status` / TUI `/crm` - where CRM data stands and the
//! exact next command. Pure file reads, no network. Exists because the
//! stale-vs-missing distinction was invisible: brief says "no CRM data
//! in this snapshot" whether crm.json never existed, sits in an older
//! snapshot, or the token expired weeks ago (live QA 2026-08-08).

use crate::config::AppConfig;
use crate::error::Result;
use crate::snapshot::Snapshot;

/// Where a snapshot's CRM data stands: one type and one sentence for every
/// surface (brief, the agent's tools, the TUI, /crm), so "connect a CRM"
/// is never said to a user whose CRM is connected but not fetched.
#[derive(Debug, Clone, PartialEq)]
pub enum CrmHealth {
    /// No crm.toml: the CRM was never connected.
    NoSpec,
    /// Connected, but this snapshot has no CRM data. `last` is the newest
    /// snapshot that has a crm.json, if any.
    NoData { last: Option<String> },
    /// This snapshot has CRM data.
    Fresh,
}

impl CrmHealth {
    pub fn of(cfg: &AppConfig, snap: &Snapshot) -> Self {
        if !super::is_empty(&snap.crm) {
            CrmHealth::Fresh
        } else if !super::fetch::spec_path(cfg).is_file() {
            CrmHealth::NoSpec
        } else {
            CrmHealth::NoData {
                last: newest_crm_snapshot(cfg),
            }
        }
    }

    /// The one sentence for a state that isn't fresh; None when fresh.
    pub fn note(&self) -> Option<String> {
        const UNKNOWN: &str =
            "CRM leads and qualified are unknown, not zero, and verdicts are withheld";
        match self {
            CrmHealth::Fresh => None,
            CrmHealth::NoSpec => Some(format!(
                "no CRM connected - {}. Fix: moneyball crm connect",
                UNKNOWN
            )),
            CrmHealth::NoData { last: Some(d) } => Some(format!(
                "no CRM data in this snapshot (newest CRM pull: snapshot {}) - {}. Fix: moneyball crm fetch",
                d, UNKNOWN
            )),
            CrmHealth::NoData { last: None } => Some(format!(
                "no CRM data pulled yet - {}. Fix: moneyball crm fetch",
                UNKNOWN
            )),
        }
    }
}

/// The snapshot crm fetch writes into: today's if its ads are already
/// there, else the newest snapshot that has ads (an ads-free "latest" dir
/// would make /brief read zero spend). Shared with crm status, so the two
/// never disagree about where the data goes.
pub(crate) fn target_snapshot(cfg: &AppConfig) -> Result<Option<String>> {
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let snap_root = cfg.history_dir().join("snap");
    if snap_root.join(&today).join("ads_daily.json").is_file() {
        return Ok(Some(today));
    }
    Ok(crate::snapshot::list_dates(&snap_root)?
        .into_iter()
        .rev()
        .find(|d| snap_root.join(d).join("ads_daily.json").is_file()))
}

/// Atomic write of crm.json into the date's snapshot dir (tmp + rename,
/// same pattern as fetch::write_snapshot).
pub(super) fn write_crm_json(
    cfg: &AppConfig,
    date: &str,
    crm: &serde_json::Value,
) -> Result<std::path::PathBuf> {
    let dir = cfg.history_dir().join("snap").join(date);
    std::fs::create_dir_all(&dir)?;
    // Tickets carry phone/email/lead_id when the spec maps them.
    crate::secrets::write_private(&dir, "crm.json", serde_json::to_string_pretty(crm)?)
}

/// `crm_contacts.json` (0600 - raw PII) next to crm.json.
pub(super) fn write_contacts(
    cfg: &AppConfig,
    date: &str,
    rows: Vec<crate::schema::ContactRow>,
) -> Result<()> {
    let file = crate::schema::CrmContactsFile {
        schema: crate::schema::CRM_CONTACTS_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        rows,
    };
    let dir = cfg.history_dir().join("snap").join(date);
    crate::secrets::write_private(
        &dir,
        "crm_contacts.json",
        serde_json::to_string_pretty(&file)?,
    )?;
    Ok(())
}

/// The newest snapshot holding a crm.json.
pub(crate) fn newest_crm_snapshot(cfg: &AppConfig) -> Option<String> {
    let snap_root = cfg.snap_dir();
    crate::snapshot::list_dates(&snap_root)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|d| snap_root.join(d).join("crm.json").is_file())
}

/// Human-readable status lines + the single most useful next command.
/// Same lines feed the CLI printout and the TUI tool cell.
pub fn status_lines(cfg: &AppConfig) -> Result<Vec<String>> {
    let mut out = Vec::new();

    // 1. Spec.
    let spec_file = super::fetch::spec_path(cfg);
    let raw = match std::fs::read_to_string(&spec_file) {
        Ok(r) => r,
        Err(_) => {
            out.push("spec:   none".into());
            out.push("data:   (nothing without a spec)".into());
            out.push("next:   moneyball crm connect (guided) or moneyball crm init".into());
            return Ok(out);
        }
    };
    let spec = super::source::parse(&raw)?;
    out.push(format!("spec:   crm.toml ({})", spec.name));

    // 2. Secrets the spec references - present in the store?
    let missing: Vec<String> = spec
        .request
        .as_ref()
        .map(|r| r.secret_names())
        .unwrap_or_default()
        .into_iter()
        .filter(|name| crate::secrets::load_crm_key(cfg.workspace_id(), name).is_none())
        .collect();
    if missing.is_empty() {
        out.push("secret: stored".into());
    } else {
        out.push(format!("secret: MISSING ({})", missing.join(", ")));
    }

    // 3. Newest crm.json across snapshots, vs the newest snapshot.
    let snap_root = cfg.snap_dir();
    let dates = crate::snapshot::list_dates(&snap_root).unwrap_or_default();
    let latest_snap = dates.last().cloned();
    let crm_home = newest_crm_snapshot(cfg);
    match (&crm_home, &latest_snap) {
        (None, _) => {
            out.push("data:   no crm.json in any snapshot".into());
        }
        (Some(d), latest) => {
            let raw = std::fs::read_to_string(snap_root.join(d).join("crm.json"))?;
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            let mut n = 0usize;
            let (mut min_ep, mut max_ep) = (i64::MAX, i64::MIN);
            super::for_each_ticket(&v, |_, ep| {
                n += 1;
                if ep > i64::MIN {
                    min_ep = min_ep.min(ep);
                    max_ep = max_ep.max(ep);
                }
            });
            let range = if max_ep > i64::MIN {
                format!(", deliveries {} .. {}", ep_date(min_ep), ep_date(max_ep))
            } else {
                String::new()
            };
            out.push(format!(
                "data:   crm.json in snapshot {} ({} ticket(s){})",
                d, n, range
            ));
            if latest.as_deref() != Some(d.as_str()) {
                out.push(format!(
                    "        STALE: the latest snapshot {} has no crm.json - brief/report/",
                    latest.as_deref().unwrap_or("?")
                ));
                out.push(
                    "        funnel run against the latest snapshot and see ZERO CRM there.".into(),
                );
            }
        }
    }

    // 4. The one next command, most-blocking first.
    let next = if !missing.is_empty() {
        format!(
            "moneyball crm secret {}",
            missing.join(" && moneyball crm secret ")
        )
    } else if crm_home.is_none() || crm_home != latest_snap {
        match target_snapshot(cfg)? {
            Some(d) => format!(
                "moneyball crm fetch   (writes crm.json into snapshot {})",
                d
            ),
            None => "moneyball fetch first: crm fetch needs a snapshot with ads".into(),
        }
    } else {
        "up to date - moneyball report / brief will show the full funnel".into()
    };
    out.push(format!("next:   {}", next));
    Ok(out)
}

/// Epoch seconds -> local calendar date, for display only.
fn ep_date(ep: i64) -> String {
    chrono::DateTime::from_timestamp(ep, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn cfg_at(root: &std::path::Path) -> AppConfig {
        AppConfig {
            data_root: root.to_path_buf(),
            date: None,
            workspace: None,
            agent: true,
        }
    }

    #[test]
    fn no_spec_points_at_connect() {
        let tmp = std::env::temp_dir().join(format!("mb-crmst-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let lines = status_lines(&cfg_at(&tmp)).unwrap();
        assert!(lines[0].contains("none"));
        assert!(lines.last().unwrap().contains("crm connect"), "{:?}", lines);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn stale_crm_json_is_called_out_with_fetch_next() {
        let tmp = std::env::temp_dir().join(format!("mb-crmst2-{}", std::process::id()));
        let mb = tmp.join(".moneyball");
        let snap = mb.join("history").join("snap");
        std::fs::create_dir_all(snap.join("2026-08-01")).unwrap();
        std::fs::create_dir_all(snap.join("2026-08-08")).unwrap();
        std::fs::write(
            mb.join("crm.toml"),
            "name = \"testcrm\"\n[map]\nroot=\"content\"\nad_id=\"a\"\nstage=\"s\"\ndelivery=\"d\"\n",
        )
        .unwrap();
        // crm.json only in the OLD snapshot.
        std::fs::write(
            snap.join("2026-08-01").join("crm.json"),
            r#"[{"ad_id":"1","stage":"Contactable","delivery":1754000000}]"#,
        )
        .unwrap();
        let lines = status_lines(&cfg_at(&tmp)).unwrap();
        let all = lines.join("\n");
        assert!(all.contains("snapshot 2026-08-01"), "{}", all);
        assert!(all.contains("STALE"), "{}", all);
        assert!(all.contains("crm fetch"), "{}", all);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// One sentence per state: a connected-but-unfetched CRM is never told
    /// to "connect", and the fix names where the data last came from.
    #[test]
    fn crm_health_says_fetch_not_connect_when_connected() {
        let tmp = std::env::temp_dir().join(format!("mb-crmhealth-{}", std::process::id()));
        let mb = tmp.join(".moneyball");
        let snap_root = mb.join("history").join("snap");
        std::fs::create_dir_all(snap_root.join("2026-08-12")).unwrap();
        std::fs::write(snap_root.join("2026-08-12").join("crm.json"), "[]").unwrap();
        let cfg = cfg_at(&tmp);
        let empty = crate::snapshot::Snapshot {
            path: snap_root.join("2026-09-28"),
            date: "2026-09-28".into(),
            ads_daily: vec![],
            adsets: serde_json::json!({}),
            creatives: None,
            crm: serde_json::json!({}),
            leads: None,
            crm_contacts: None,
            regions: serde_json::json!([]),
            changes: serde_json::json!([]),
            campaigns: serde_json::json!([]),
        };

        // No crm.toml yet: connect.
        let h = CrmHealth::of(&cfg, &empty);
        assert_eq!(h, CrmHealth::NoSpec);
        assert!(h.note().unwrap().contains("crm connect"));

        // Connected, this snapshot empty: fetch, naming the newest pull.
        std::fs::write(mb.join("crm.toml"), "name = \"t\"\n").unwrap();
        let h = CrmHealth::of(&cfg, &empty);
        assert_eq!(
            h,
            CrmHealth::NoData {
                last: Some("2026-08-12".into())
            }
        );
        let note = h.note().unwrap();
        assert!(
            note.contains("crm fetch") && note.contains("2026-08-12"),
            "{}",
            note
        );
        assert!(!note.contains("connect"), "{}", note);

        // CRM data present: nothing to say.
        let mut fresh = empty;
        fresh.crm = serde_json::json!([{"ad_id": "1", "stage": "Contactable", "delivery": 1}]);
        assert_eq!(CrmHealth::of(&cfg, &fresh).note(), None);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
