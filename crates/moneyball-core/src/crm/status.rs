//! `moneyball crm status` / TUI `/crm` - where CRM data stands and the
//! exact next command. Pure file reads, no network. Exists because the
//! stale-vs-missing distinction was invisible: brief says "no CRM data
//! in this snapshot" whether crm.json never existed, sits in an older
//! snapshot, or the token expired weeks ago (live QA 2026-08-08).

use crate::config::AppConfig;
use crate::error::Result;

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
    let mut missing: Vec<String> = Vec::new();
    if let Some(req) = &spec.request {
        for v in req.headers.values().chain(req.query.values()) {
            if let Some(name) = v.trim_matches(['{', '}']).strip_prefix("secret:") {
                if crate::secrets::load_crm_key(name).is_none() {
                    missing.push(name.to_string());
                }
            }
        }
    }
    if missing.is_empty() {
        out.push("secret: stored".into());
    } else {
        out.push(format!("secret: MISSING ({})", missing.join(", ")));
    }

    // 3. Newest crm.json across snapshots, vs the newest snapshot.
    let snap_root = cfg.snap_dir();
    let dates = crate::snapshot::list_dates(&snap_root).unwrap_or_default();
    let latest_snap = dates.last().cloned();
    let crm_home = dates
        .iter()
        .rev()
        .find(|d| snap_root.join(d).join("crm.json").is_file())
        .cloned();
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
        "moneyball crm fetch   (writes crm.json into today's snapshot)".into()
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
}
