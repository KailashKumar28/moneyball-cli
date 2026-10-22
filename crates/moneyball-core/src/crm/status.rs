//! `moneyball crm status` / TUI `/crm` - where CRM data stands and the
//! exact next command. File reads, no network - and one write: a legacy
//! shared CRM key is adopted into the workspace that used it (see
//! adopt_legacy_keys). Exists because the
//! stale-vs-missing distinction was invisible: brief says "no CRM data
//! in this snapshot" whether crm.json never existed, sits in an older
//! snapshot, or the token expired weeks ago (live QA 2026-08-08).

use super::source::RequestSpec;
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
    /// snapshot that has a crm.json, if any; `failed` why the CRM pull
    /// for this snapshot failed, and its fix, when one did - a refresh
    /// that already failed is no fix.
    NoData {
        last: Option<String>,
        failed: Option<(String, Option<String>)>,
    },
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
            let failed = crate::runs::read(cfg, &snap.date)
                .and_then(|r| r.steps.into_iter().find(|s| s.name == "crm"))
                .filter(|s| s.status == crate::schema::StepStatus::Failed)
                .map(|s| (s.message.unwrap_or_default(), s.fix));
            CrmHealth::NoData {
                last: newest_crm_snapshot(cfg),
                failed,
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
                "no CRM connected - {}. Fix: /connect crm (moneyball connect crm in a shell)",
                UNKNOWN
            )),
            CrmHealth::NoData {
                failed: Some((why, fix)),
                ..
            } => {
                let fix = match fix.as_deref() {
                    Some(f) if f.starts_with('/') => format!(
                        "{} ({} in a shell), then /refresh",
                        f,
                        crate::connections::fix_in_shell(f)
                    ),
                    Some(f) => format!("{} in a shell, then /refresh", f),
                    None => "/refresh (moneyball refresh in a shell) once the CRM answers".into(),
                };
                Some(format!(
                    "no CRM data in this snapshot: its CRM pull failed ({}) - {}. Fix: {}",
                    why, UNKNOWN, fix
                ))
            }
            CrmHealth::NoData {
                last: Some(d),
                failed: None,
            } => Some(format!(
                "no CRM data in this snapshot (newest CRM pull: snapshot {}) - {}. Fix: /refresh (moneyball refresh in a shell)",
                d, UNKNOWN
            )),
            CrmHealth::NoData {
                last: None,
                failed: None,
            } => Some(format!(
                "no CRM data pulled yet - {}. Fix: /refresh (moneyball refresh in a shell)",
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
    let snap_root = cfg.snap_dir();
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
    let dir = cfg.snap_dir().join(date);
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
    let dir = cfg.snap_dir().join(date);
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

/// Per-client CRM keys shipped this day: only a pull before it used the
/// shared store. Snapshot dirs are dated by when the pull ran, so a new
/// client never has one this old.
const PER_CLIENT_KEYS: &str = "2026-10-09";

/// Before per-client keys, CRM keys sat in one shared store. A workspace
/// that pulled CRM data then used them, so it gets its own copy; any
/// other (a new client, a CSV import, a pull since) never reads the
/// shared store - that would wire it to another client's CRM.
pub(crate) fn adopt_legacy_keys(cfg: &AppConfig, request: &RequestSpec) {
    let snap_root = cfg.snap_dir();
    let used_shared = crate::snapshot::list_dates(&snap_root)
        .unwrap_or_default()
        .iter()
        .any(|d| d.as_str() < PER_CLIENT_KEYS && snap_root.join(d).join("crm.json").is_file());
    if let (Some(ws), true) = (cfg.workspace_id(), used_shared) {
        // A failed copy only leaves the key to be stored again.
        let _ = crate::secrets::adopt_legacy_crm_keys(ws, &request.secret_names());
    }
}

/// Adopts legacy keys first (one write to auth.json, see above), then the
/// secrets the request names that this workspace has no key for.
pub(crate) fn unstored_secrets(cfg: &AppConfig, request: Option<&RequestSpec>) -> Vec<String> {
    let Some(request) = request else {
        return Vec::new();
    };
    adopt_legacy_keys(cfg, request);
    request
        .secret_names()
        .into_iter()
        .filter(|n| crate::secrets::load_crm_key(cfg.workspace_id(), n).is_none())
        .collect()
}

/// The last pull's CRM step, when the CRM refused its key.
fn refused_on_last_pull(cfg: &AppConfig) -> Option<crate::schema::RunStep> {
    crate::runs::latest(cfg)?
        .steps
        .into_iter()
        .find(|s| s.name == "crm" && s.error_kind.as_deref() == Some("crm_auth"))
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
    let missing = unstored_secrets(cfg, spec.request.as_ref());
    let refused = refused_on_last_pull(cfg);
    match (&refused, missing.is_empty()) {
        (_, false) => out.push(format!("secret: MISSING ({})", missing.join(", "))),
        (Some(s), true) => out.push(format!(
            "secret: stored - the CRM refused it on the last pull ({})",
            s.message.as_deref().unwrap_or("auth failed")
        )),
        (None, true) => out.push("secret: stored".into()),
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
                "data:   crm.json in snapshot {} ({}{})",
                snap_day(d),
                crate::text::count(n as u64, "ticket", "tickets"),
                range
            ));
            if latest.as_deref() != Some(d.as_str()) {
                out.push(format!(
                    "        stale: the newest snapshot ({}) has no CRM data, so the brief, report \
                     and funnel there show none",
                    latest.as_deref().map_or("?".into(), snap_day)
                ));
            }
        }
    }

    // 4. The one next command, most-blocking first.
    let next = if !missing.is_empty() {
        format!(
            "moneyball crm secret {}",
            missing.join(" && moneyball crm secret ")
        )
    } else if let Some(fix) = refused.and_then(|s| s.fix) {
        format!("{}   (a fresh one, then moneyball refresh)", fix)
    } else if crm_home.is_none() || crm_home != latest_snap {
        match target_snapshot(cfg)? {
            Some(d) => format!(
                "moneyball crm fetch   (writes crm.json into snapshot {})",
                snap_day(&d)
            ),
            None => "moneyball fetch first: crm fetch needs a snapshot with ads".into(),
        }
    } else {
        "up to date - moneyball report / brief will show the full funnel".into()
    };
    out.push(format!("next:   {}", next));
    Ok(out)
}

/// Epoch seconds -> calendar day, for display only ("15 Jul").
fn ep_date(ep: i64) -> String {
    chrono::DateTime::from_timestamp(ep, 0)
        .map(|d| d.format("%d %b").to_string())
        .unwrap_or_else(|| "?".into())
}

/// A snapshot dir name as a day ("Wed 12 Aug").
fn snap_day(d: &str) -> String {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
        .map_or_else(|_| d.to_string(), |d| d.format("%a %d %b").to_string())
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
