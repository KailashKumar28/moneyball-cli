//! `funnel` - per-entity funnel for one product: Meta spend/leads joined
//! with CRM outcomes per campaign/adset/ad, plus kill-eligibility math.
//! Reimplements mb.py `funnel_agg`/`cmd_funnel` natively (window = last
//! N complete days ending yesterday; CRM bucketed by delivery time IST).

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::snapshot::Snapshot;

pub mod print;
pub use print::table;

/// Spend as a multiple of target Rs/qualified at 0/1/2 qualified leads
/// before an entity becomes a kill candidate (mb.py KILL_TABLE).
pub const KILL_TABLE: [f64; 3] = [3.0, 4.7, 6.3];
const LAG_HOURS: i64 = 72;
/// mb.py fallback when the workspace sets no target_rs_per_q.
const DEFAULT_TARGET_RPQ: f64 = 2500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Campaign,
    Adset,
    Ad,
}

impl By {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "campaign" => Ok(By::Campaign),
            "adset" => Ok(By::Adset),
            "ad" => Ok(By::Ad),
            other => Err(Error::Config(format!(
                "--by must be campaign, adset or ad (got \"{}\")",
                other
            ))),
        }
    }
    fn ids<'a>(&self, r: &'a crate::snapshot::AdsDailyRow) -> (&'a str, &'a str) {
        match self {
            By::Campaign => (&r.campaign_id, &r.campaign_name),
            By::Adset => (&r.adset_id, &r.adset_name),
            By::Ad => (&r.ad_id, &r.ad_name),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FunnelRow {
    pub id: String,
    pub name: String,
    pub spend: u64,
    pub m: u64,
    pub l: u64,
    pub q: u64,
    pub v: u64,
    pub b: u64,
    pub cpl: Option<u64>,
    pub rs_per_q: Option<u64>,
    pub l_to_q: Option<f64>,
    pub kill_mult: f64,
    pub kill: bool,
    pub sufficient: bool,
    pub immature: bool,
    pub learning: String,
}

#[derive(Default)]
struct Agg {
    name: String,
    spend: f64,
    m: u64,
    l: u64,
    q: u64,
    v: u64,
    b: u64,
    recent_l: u64,
}

/// Aggregate the funnel for `product` over the last `days` complete days,
/// keyed by `by`. Sorted by spend descending.
pub fn compute(
    snap: &Snapshot,
    cfg: &AppConfig,
    product: &str,
    days: u32,
    by: By,
) -> Vec<FunnelRow> {
    let snap_date = NaiveDate::parse_from_str(&snap.date, "%Y-%m-%d")
        .expect("snapshot::load validated the date");
    let d1 = snap_date - Duration::days(1);
    let d0 = d1 - Duration::days(days.max(1) as i64 - 1);
    let (d0s, d1s) = (
        d0.format("%Y-%m-%d").to_string(),
        d1.format("%Y-%m-%d").to_string(),
    );

    let mut agg: HashMap<String, Agg> = HashMap::new();
    let mut ad_to_key: HashMap<String, String> = HashMap::new();
    for r in &snap.ads_daily {
        if r._product != product || r.date_start < d0s || r.date_start > d1s {
            continue;
        }
        let (id, name) = by.ids(r);
        let e = agg.entry(id.to_string()).or_default();
        if e.name.is_empty() {
            e.name = if name.is_empty() {
                id.into()
            } else {
                name.into()
            };
        }
        e.spend += r.spend_num();
        e.m += crate::brief::count_m_leads(&r.actions);
        ad_to_key
            .entry(r.ad_id.clone())
            .or_insert_with(|| id.into());
    }

    // CRM outcomes joined by ad_id, delivery-bucketed (IST).
    let d1_ist = crate::brief::ist_midnight_epoch(snap_date);
    let d0_ist = d1_ist - days.max(1) as i64 * 86400;
    let lag_cut = d1_ist - LAG_HOURS * 3600;
    crate::crm::for_each_ticket(&snap.crm, |t, ep| {
        if ep < d0_ist || ep >= d1_ist {
            return;
        }
        let aid = crate::crm::ticket_ad_id(t).unwrap_or_default();
        let Some(key) = ad_to_key.get(&aid) else {
            return;
        };
        let (is_q, is_v, is_b) =
            crate::crm::milestones(&crate::crm::ticket_stage(t), &crate::crm::ticket_funnel(t));
        let e = agg.entry(key.clone()).or_default();
        e.l += 1;
        e.q += is_q as u64;
        e.v += is_v as u64;
        e.b += is_b as u64;
        if ep >= lag_cut {
            e.recent_l += 1;
        }
    });

    let target_rpq = cfg
        .workspace
        .as_ref()
        .and_then(|w| w.target_rs_per_q)
        .unwrap_or(DEFAULT_TARGET_RPQ);

    let mut rows: Vec<FunnelRow> = agg
        .into_iter()
        .map(|(id, e)| {
            let mult = e.spend / target_rpq;
            let kill = e.q <= 2 && mult >= KILL_TABLE[e.q.min(2) as usize];
            FunnelRow {
                learning: learning_of(snap, &id, by),
                cpl: (e.m > 0).then(|| (e.spend / e.m as f64).round() as u64),
                rs_per_q: (e.q > 0).then(|| (e.spend / e.q as f64).round() as u64),
                l_to_q: (e.l > 0).then(|| (e.q as f64 / e.l as f64 * 1000.0).round() / 10.0),
                kill_mult: (mult * 10.0).round() / 10.0,
                kill,
                sufficient: e.spend >= 500.0 && e.m >= 3,
                immature: e.recent_l > 0,
                spend: e.spend.round() as u64,
                id,
                name: e.name,
                m: e.m,
                l: e.l,
                q: e.q,
                v: e.v,
                b: e.b,
            }
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.spend));
    rows
}

/// Adset learning-stage status from the snapshot's adsets blob; "-" for
/// other levels or when absent (mirrors mb.py).
fn learning_of(snap: &Snapshot, id: &str, by: By) -> String {
    if by != By::Adset {
        return "-".into();
    }
    snap.adsets
        .get(id)
        .and_then(|a| a.pointer("/learning_stage_info/status"))
        .and_then(|s| s.as_str())
        .unwrap_or("-")
        .to_string()
}

/// Headless runner: load the snapshot, aggregate, print the table.
pub fn run(cfg: &AppConfig, product: &str, by: &str, days: u32, date: Option<&str>) -> Result<()> {
    let by = By::parse(by)?;
    let snap = crate::snapshot::load(&cfg.snap_for(date)?)?;
    let product: String = match cfg.workspace.as_ref() {
        Some(w) if !w.products.is_empty() => crate::product::resolve_product(product, &w.products)
            .map_err(Error::Config)?
            .to_string(),
        _ => product.to_string(),
    };
    let product = product.as_str();
    let rows = compute(&snap, cfg, product, days, by);
    let by_name = match by {
        By::Campaign => "campaign",
        By::Adset => "adset",
        By::Ad => "ad",
    };
    println!(
        "FUNNEL {} - by {} - {}d - snapshot {}",
        product, by_name, days, snap.date
    );
    print!("{}", table(&rows));
    if let Some(rank) = cpl_ranking(&rows) {
        println!("{}", rank);
    }
    println!(
        "\nkill = spend >= {}x/{}x/{}x target Rs/qualified with 0/1/2 qualified; immature = leads arrived in trailing {}h",
        KILL_TABLE[0], KILL_TABLE[1], KILL_TABLE[2], LAG_HOURS
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn snap() -> Snapshot {
        let ads = json!([
            { "campaign_id": "c1", "campaign_name": "Camp One", "adset_id": "s1",
              "adset_name": "Set One", "ad_id": "a1", "ad_name": "Ad One",
              "spend": "9000", "date_start": "2026-07-14", "date_stop": "2026-07-14",
              "_product": "P",
              "actions": [{"action_type": "lead", "value": "4"}] },
            { "campaign_id": "c1", "campaign_name": "Camp One", "adset_id": "s2",
              "adset_name": "Set Two", "ad_id": "a2", "ad_name": "Ad Two",
              "spend": "1000", "date_start": "2026-07-14", "date_stop": "2026-07-14",
              "_product": "P",
              "actions": [] },
            { "campaign_id": "c9", "campaign_name": "Other", "adset_id": "s9",
              "adset_name": "S9", "ad_id": "a9", "ad_name": "A9",
              "spend": "500", "date_start": "2026-07-14", "date_stop": "2026-07-14",
              "_product": "OTHER", "actions": [] }
        ]);
        // 2026-07-14 ~noon IST as epoch (inside the 7d window before 07-16).
        let ep = crate::brief::ist_midnight_epoch(
            NaiveDate::parse_from_str("2026-07-14", "%Y-%m-%d").unwrap(),
        ) + 12 * 3600;
        let crm = json!([
            { "ad_id": "a1", "stage": "Contactable", "delivery": ep },
            { "ad_id": "a1", "stage": "Visit", "delivery": ep },
            { "ad_id": "a2", "stage": "Lost", "delivery": ep },
            { "ad_id": "a9", "stage": "Booking", "delivery": ep },
            { "ad_id": "a1", "stage": "Booking", "delivery": 1 }
        ]);
        Snapshot {
            path: PathBuf::new(),
            date: "2026-07-16".into(),
            ads_daily: serde_json::from_value(ads).unwrap(),
            adsets: json!({ "s1": { "learning_stage_info": { "status": "LEARNING" } } }),
            creatives: None,
            crm,
            regions: json!([]),
            changes: json!([]),
            campaigns: json!([]),
        }
    }

    fn cfg() -> AppConfig {
        AppConfig::resolve_optional(Some("/nonexistent-mb-funnel"), None)
    }

    #[test]
    fn aggregates_by_adset_with_crm_join() {
        let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
        assert_eq!(rows.len(), 2);
        // Sorted by spend desc: s1 first.
        assert_eq!(rows[0].id, "s1");
        assert_eq!(rows[0].m, 4);
        assert_eq!((rows[0].l, rows[0].q, rows[0].v), (2, 2, 1));
        assert_eq!(rows[0].learning, "LEARNING");
        // a9's Booking belongs to another product; a1's epoch-1 ticket is
        // outside the window - neither may leak in.
        assert_eq!(rows[1].id, "s2");
        assert_eq!((rows[1].l, rows[1].q), (1, 0));
    }

    #[test]
    fn kill_logic_follows_the_table() {
        let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
        // s1: spend 9000, q 2, target 2500 -> mult 3.6 < 6.3 -> keep.
        assert!(!rows[0].kill);
        assert_eq!(rows[0].kill_mult, 3.6);
        // s2: spend 1000, q 0 -> mult 0.4 < 3.0 -> keep.
        assert!(!rows[1].kill);
        // sufficient: s1 (spend 9000, m 4) yes; s2 (m 0) no.
        assert!(rows[0].sufficient && !rows[1].sufficient);
    }

    #[test]
    fn campaign_level_merges_adsets() {
        let rows = compute(&snap(), &cfg(), "P", 7, By::Campaign);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "c1");
        assert_eq!(rows[0].spend, 10000);
        assert_eq!((rows[0].l, rows[0].q), (3, 2));
        assert_eq!(rows[0].learning, "-");
    }

    #[test]
    fn table_renders_header_and_rows() {
        let rows = compute(&snap(), &cfg(), "P", 7, By::Adset);
        let t = table(&rows);
        assert!(t.starts_with("id"));
        assert!(t.contains("Set One"));
        assert!(!table(&[]).contains("id"));
    }
}

/// Pre-computed CPL ordering, cheapest first. Injected into funnel tool
/// output so the model NARRATES a ranking instead of comparing raw
/// numbers itself (live QA 2026-08-08: the model called Rs.721 CPL
/// "mid-pack" against Rs.1,216 "cheapest" - bug mb-20260808T143646Z-rrrr).
/// None when fewer than two rows have a CPL.
pub fn cpl_ranking(rows: &[FunnelRow]) -> Option<String> {
    let mut with_cpl: Vec<(&FunnelRow, u64)> =
        rows.iter().filter_map(|r| r.cpl.map(|c| (r, c))).collect();
    if with_cpl.len() < 2 {
        return None;
    }
    with_cpl.sort_by_key(|(_, c)| *c);
    let parts: Vec<String> = with_cpl
        .iter()
        .map(|(r, c)| format!("{} Rs.{}", r.name, c))
        .collect();
    Some(format!(
        "cpl ranking (cheapest Meta lead first, PRE-COMPUTED - trust this order): {}",
        parts.join(" < ")
    ))
}

#[cfg(test)]
mod ranking_tests {
    use super::*;

    fn row(name: &str, cpl: Option<u64>) -> FunnelRow {
        FunnelRow {
            name: name.into(),
            cpl,
            ..Default::default()
        }
    }

    #[test]
    fn ranking_orders_cheapest_first_and_skips_no_cpl() {
        let rows = vec![
            row("NonAdv", Some(1216)),
            row("Pincode", Some(2381)),
            row("Lookalike", Some(721)),
            row("Silent", None),
        ];
        let r = cpl_ranking(&rows).unwrap();
        assert!(
            r.contains("Lookalike Rs.721 < NonAdv Rs.1216 < Pincode Rs.2381"),
            "{}",
            r
        );
        assert!(cpl_ranking(&[row("only", Some(9))]).is_none());
    }
}
