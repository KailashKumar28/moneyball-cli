//! `moneyball report` - the creative report (slice B1, docs/CLOUD_PLAN.md).
//!
//! Reads ONE snapshot (ads_daily + creatives + crm) with no network and
//! computes the typed `CreativeReport` aggregate - report.json IS the
//! product; HTML (B2), bot text, and the future service all render or
//! ingest this artifact. Behavioral spec: pipeline/creative_report.py
//! (reimplemented, never imported - AGENTS.md).
//!
//! Window semantics match brief/funnel exactly: a snapshot dated D
//! covers through D-1; `window_days` trailing complete days end at D-1.

mod card;
mod group;
pub mod html;

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::schema::*;
use crate::snapshot::Snapshot;
use group::{AdAgg, GroupAgg};

/// Compute the report aggregate from one snapshot. Pure - no clock, no
/// network, no writes (generated_at/workspace_id come from the caller
/// so tests are deterministic).
pub fn build(
    snap: &Snapshot,
    workspace_id: &str,
    generated_at: &str,
    window_days: u32,
) -> Result<CreativeReport> {
    let snap_date = NaiveDate::parse_from_str(&snap.date, "%Y-%m-%d")
        .map_err(|_| Error::Config(format!("bad snapshot date {}", snap.date)))?;
    let d1 = snap_date - Duration::days(1);
    let d0 = d1 - Duration::days(window_days.max(1) as i64 - 1);
    let (d0s, d1s) = (
        d0.format("%Y-%m-%d").to_string(),
        d1.format("%Y-%m-%d").to_string(),
    );

    // ---- per-ad delivery aggregates over the window ----
    let mut ads: BTreeMap<String, AdAgg> = BTreeMap::new();
    for r in &snap.ads_daily {
        if r.date_start < d0s || r.date_start > d1s {
            continue;
        }
        ads.entry(r.ad_id.clone()).or_default().add_row(r);
    }

    // ---- group ads by creative identity, product-scoped ----
    let creative_by_ad: BTreeMap<&str, &CreativeRow> = snap
        .creatives
        .iter()
        .flat_map(|f| f.rows.iter())
        .map(|c| (c.ad_id.as_str(), c))
        .collect();
    let mut groups: BTreeMap<(String, String), GroupAgg> = BTreeMap::new();
    for (ad_id, agg) in &ads {
        let cr = creative_by_ad.get(ad_id.as_str()).copied();
        let (kind, key) = group::group_key(ad_id, agg, cr);
        groups
            .entry((agg.product.clone(), key))
            .or_insert_with(|| GroupAgg::new(kind))
            .add_ad(ad_id, agg, cr);
    }

    // ---- CRM pass: delivery-time window (IST), joined by ad_id ----
    let d1_ist = crate::brief::ist_midnight_epoch(snap_date);
    let d0_ist = d1_ist - window_days.max(1) as i64 * 86400;
    let ad_group: BTreeMap<&str, &(String, String)> = groups
        .iter()
        .flat_map(|(k, g)| g.ad_ids.iter().map(move |a| (a.as_str(), k)))
        .collect();
    let mut crm_hits: Vec<(String, String, bool, bool, bool, String, i64)> = Vec::new();
    let mut unattributed = 0u64;
    let crm_present = !crate::crm::is_empty(&snap.crm);
    crate::crm::for_each_ticket(&snap.crm, |t, ep| {
        if ep < d0_ist || ep >= d1_ist {
            return;
        }
        let aid = crate::crm::ticket_ad_id(t).unwrap_or_default();
        let Some((product, key)) = ad_group.get(aid.as_str()).map(|k| (*k).clone()) else {
            // No joinable ad in the window (includes the intentional
            // "Stattic Ad" rows) - counted, never silently dropped.
            unattributed += 1;
            return;
        };
        let (q, v, b) =
            crate::crm::milestones(&crate::crm::ticket_stage(t), &crate::crm::ticket_funnel(t));
        let targeting = ads
            .get(&aid)
            .map(|a| a.targeting.clone())
            .unwrap_or_default();
        crm_hits.push((product, key, q, v, b, targeting, ep));
    });
    for (product, key, q, v, b, targeting, ep) in crm_hits {
        if let Some(g) = groups.get_mut(&(product, key)) {
            g.add_ticket(q, v, b, &targeting, ep, snap_date);
        }
    }

    // ---- assemble cards per product ----
    let mut products: BTreeMap<String, Vec<CreativeCard>> = BTreeMap::new();
    for ((product, key), g) in groups {
        // python: keep groups that delivered (impressions or leads).
        if g.delivery.impressions == 0 && g.delivery.m_leads == 0 && g.crm.l_leads == 0 {
            continue;
        }
        products
            .entry(product)
            .or_default()
            .push(g.into_card(key, d0, d1, snap));
    }
    let mut sections: Vec<ProductSection> = Vec::new();
    for (product, mut cards) in products {
        // python sort: booking, visit, qualified, l_leads, m_leads desc.
        cards.sort_by(|a, b| {
            let k = |c: &CreativeCard| {
                let f = |i: usize| c.funnel[i].count;
                (f(6), f(5), f(4), f(3), f(2), c.delivery.spend as u64)
            };
            k(b).cmp(&k(a))
        });
        let kpis = card::section_kpis(&cards);
        sections.push(ProductSection {
            product,
            kpis,
            creatives: cards,
        });
    }
    let mut portfolio = card::portfolio_kpis(&sections);
    portfolio.unattributed_l_leads = unattributed;

    Ok(CreativeReport {
        schema: CREATIVE_REPORT_SCHEMA.into(),
        workspace_id: workspace_id.into(),
        report_date: snap.date.clone(),
        window: ReportWindow {
            since: d0s,
            until: d1s,
        },
        generated_at: generated_at.into(),
        source: ReportSource {
            snapshot_date: snap.date.clone(),
            crm_present,
            creatives_schema: snap.creatives.as_ref().map(|f| f.schema.clone()),
        },
        portfolio,
        products: sections,
    })
}

/// A generated report on disk - what both the CLI and the TUI surface.
pub struct ReportOutput {
    pub report: CreativeReport,
    pub json_path: std::path::PathBuf,
    pub html_path: std::path::PathBuf,
}

/// Load the snapshot, build the aggregate, write
/// `reports/<date>/creative-report.{json,html}`. Shared by the CLI
/// runner and the TUI /report worker.
pub fn generate(cfg: &AppConfig, date: Option<&str>, window_days: u32) -> Result<ReportOutput> {
    let snap = crate::snapshot::load(&cfg.snap_for(date)?)?;
    let workspace_id = ensure_workspace_id(cfg)?;
    let generated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let report = build(&snap, &workspace_id, &generated_at, window_days)?;

    let dir = cfg.mb_dir().join("reports").join(&report.report_date);
    std::fs::create_dir_all(&dir)?;
    let json_path = dir.join("creative-report.json");
    let tmp = dir.join("creative-report.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&report)?)?;
    std::fs::rename(&tmp, &json_path)?;

    // HTML rendered strictly from the aggregate + asset cache (B2).
    let html_path = dir.join("creative-report.html");
    let html_tmp = dir.join("creative-report.html.tmp");
    std::fs::write(&html_tmp, html::render(&report, &cfg.history_dir()))?;
    std::fs::rename(&html_tmp, &html_path)?;

    Ok(ReportOutput {
        report,
        json_path,
        html_path,
    })
}

/// Headless runner: generate + print the text summary.
pub fn run(cfg: &AppConfig, date: Option<&str>, window_days: u32) -> Result<()> {
    let out = generate(cfg, date, window_days)?;
    print!("{}", text_summary(&out.report));
    if !out.report.source.crm_present {
        println!("note: no CRM data in this snapshot - L/Q/V/B are zeros, not truths.");
    }
    println!("report written: {}", out.json_path.display());
    println!("open in browser: {}", out.html_path.display());
    Ok(())
}

/// Stable workspace UUID, minted into config.json on first use - the
/// tenant key before accounts exist (docs/CLOUD_PLAN.md hedge 2).
pub fn ensure_workspace_id(cfg: &AppConfig) -> Result<String> {
    let ws = cfg
        .workspace
        .as_ref()
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    if let Some(id) = &ws.workspace_id {
        return Ok(id.clone());
    }
    let id = mint_id(&cfg.data_root.display().to_string());
    let mut ws2 = ws.clone();
    ws2.workspace_id = Some(id.clone());
    ws2.save(&cfg.data_root)?;
    Ok(id)
}

/// UUID-shaped id from sha256(data_root, time, pid) - collision-safe
/// across machines without a rand dependency.
fn mint_id(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let h = format!(
        "{:x}",
        Sha256::digest(format!("{}|{}|{}", seed, now, std::process::id()))
    );
    // Hex output is pure ASCII; .get() keeps the no-string-indexing rule.
    let seg = |a: usize, b: usize| h.get(a..b).unwrap_or_default();
    format!(
        "{}-{}-{}-{}-{}",
        seg(0, 8),
        seg(8, 12),
        seg(12, 16),
        seg(16, 20),
        seg(20, 32)
    )
}

/// Plain-text rendering of the aggregate - the CLI output and the dry
/// run for a future bot renderer (contract goal: a bot needs nothing
/// beyond report.json).
pub fn text_summary(r: &CreativeReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let f = &r.portfolio.funnel;
    let _ = writeln!(
        out,
        "CREATIVE REPORT  {}  (window {}..{})",
        r.report_date, r.window.since, r.window.until
    );
    let _ = writeln!(
        out,
        "portfolio: spend Rs.{:.0}  impr {}  clicks {}  M {}  L {}  Q {}  V {}  B {}{}",
        r.portfolio.spend,
        r.portfolio.impressions,
        r.portfolio.clicks,
        f.m_leads,
        f.l_leads,
        f.qualified,
        f.visit,
        f.booking,
        if r.portfolio.unattributed_l_leads > 0 {
            format!("  (+{} unattributed L)", r.portfolio.unattributed_l_leads)
        } else {
            String::new()
        }
    );
    for p in &r.products {
        let _ = writeln!(out, "\n{} - {} creative(s)", p.product, p.creatives.len());
        for c in p.creatives.iter().take(3) {
            let fun: Vec<String> = c
                .funnel
                .iter()
                .skip(2)
                .map(|s| s.count.to_string())
                .collect();
            let _ = writeln!(
                out,
                "  [{}] {}  spend Rs.{:.0}  M/L/Q/V/B {}  {}",
                c.status.label,
                c.display_name,
                c.delivery.spend,
                fun.join("/"),
                if c.image.is_some() { "" } else { "(no image)" }
            );
        }
        if p.creatives.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", p.creatives.len() - 3);
        }
    }
    out
}
