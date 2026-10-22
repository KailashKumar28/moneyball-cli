//! Where a report's data came from, and when: `source` stamped from the
//! snapshot's own files, and the masthead line that says it - pulled when
//! the data was pulled, not when the page was rendered; "your CRM" only
//! when the CRM is in it; compared only with a day from the last week.

use std::path::Path;

use crate::config::AppConfig;
use crate::crm::status::CrmHealth;
use crate::schema::CreativeReport;
use crate::snapshot::Snapshot;

use super::html::{day, esc};

/// Fill `source.meta_fetched_at`, `crm_fetched_at` and `crm_status`.
pub(super) fn stamp(
    cfg: &AppConfig,
    snap_dir: &Path,
    snap: &Snapshot,
    report: &mut CreativeReport,
) {
    let written = |file: &str| {
        let t = std::fs::metadata(snap_dir.join(file))
            .ok()?
            .modified()
            .ok()?;
        Some(
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        )
    };
    let s = &mut report.source;
    s.meta_fetched_at = written("ads_daily.json");
    if s.crm_present {
        s.crm_fetched_at = written("crm.json");
    }
    s.crm_status = Some(
        match CrmHealth::of(cfg, snap) {
            CrmHealth::Fresh => "present",
            CrmHealth::NoData { .. } => "not_pulled",
            CrmHealth::NoSpec => "not_connected",
        }
        .into(),
    );
}

/// "Data: 08 Oct 2026, full day . pulled 09 Oct, 07:30 am IST . sources: ...".
pub(super) fn freshness(
    r: &CreativeReport,
    prior: Option<&CreativeReport>,
    window: &str,
    multi: bool,
) -> String {
    // When the data was pulled, not when this page was rendered.
    let pulled_at = r
        .source
        .meta_fetched_at
        .as_deref()
        .unwrap_or(&r.generated_at);
    let pulled = chrono::DateTime::parse_from_rfc3339(pulled_at)
        .map(|t| {
            (t + chrono::Duration::minutes(330))
                .format("%d %b, %I:%M %P IST")
                .to_string()
        })
        .unwrap_or_else(|_| pulled_at.to_string());
    format!(
        "Data: {}, {} &middot; pulled {} &middot; sources: {}{}",
        esc(window),
        if multi { "full days" } else { "full day" },
        esc(&pulled),
        if r.source.crm_present {
            "Meta Ads + your CRM"
        } else {
            "Meta Ads only (no CRM data for this day)"
        },
        prior
            .map(|p| format!(
                " &middot; compared with {}",
                esc(&day(&p.window.until, "%a %d %b"))
            ))
            .unwrap_or_else(|| format!(
                " &middot; no comparable day in the last {}",
                super::exec::PRIOR_DAYS
            ))
    )
}
