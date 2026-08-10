//! Targeting roll-up + verdict engine (marketing spec 2026-08-10).
//!
//! The table shows the report window; VERDICTS compute on a trailing
//! 7-complete-day aggregate - a 1-day window of Indian RE lead-gen is
//! 0-3 qualified and statistically dishonest to verdict on. Rules are
//! deterministic mirrors of the funnel kill-table where applicable and
//! never fire on immature data (72h maturation guard). Fatigue and
//! overlap rules are deferred: fatigue needs reliable per-targeting
//! reach, overlap needs geo-circle math - both weekly-report material.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::funnel::KILL_TABLE;
use crate::schema::*;
use crate::snapshot::Snapshot;

const DEFAULT_TARGET_RPQ: f64 = 2500.0;

/// Build the per-product targeting reports. `d0/d1` = report window;
/// 7d basis = the 7 complete days ending `d1`.
pub(super) fn build(
    snap: &Snapshot,
    cfg_target_rpq: Option<f64>,
    d0: NaiveDate,
    d1: NaiveDate,
) -> BTreeMap<String, Vec<TargetingReport>> {
    let target_rpq = cfg_target_rpq.unwrap_or(DEFAULT_TARGET_RPQ);
    let (d0s, d1s) = (fmt(d0), fmt(d1));
    let d7_0s = fmt(d1 - chrono::Duration::days(6));

    // (product, targeting) -> aggregates; ad -> (product, targeting).
    #[derive(Default)]
    struct Agg {
        win: Delivery,
        win_crm: TargetingCrm,
        seven: SevenDay,
        recent_l: u64,
        adset_ids: Vec<String>,
    }
    let mut aggs: BTreeMap<(String, String), Agg> = BTreeMap::new();
    let mut ad_key: BTreeMap<&str, (String, String)> = BTreeMap::new();
    for r in &snap.ads_daily {
        if r.date_start < d7_0s || r.date_start > d1s {
            continue;
        }
        let targeting = super::group::base_targeting(&r.adset_name);
        let key = (r._product.clone(), targeting);
        let a = aggs.entry(key.clone()).or_default();
        let m = crate::brief::count_m_leads(&r.actions);
        a.seven.spend += r.spend_num();
        a.seven.impressions += r.impressions_num();
        a.seven.clicks += r.clicks_num();
        a.seven.m_leads += m;
        if r.date_start >= d0s {
            a.win.spend += r.spend_num();
            a.win.impressions += r.impressions_num();
            a.win.clicks += r.clicks_num();
            a.win.m_leads += m;
        }
        if !r.adset_id.is_empty() && !a.adset_ids.contains(&r.adset_id) {
            a.adset_ids.push(r.adset_id.clone());
        }
        ad_key.entry(r.ad_id.as_str()).or_insert(key);
    }

    // CRM: 7d + window + 72h-recent, delivery-bucketed IST.
    let snap_date = NaiveDate::parse_from_str(&snap.date, "%Y-%m-%d").expect("validated");
    let d1_ist = crate::brief::ist_midnight_epoch(snap_date);
    let win_days = (d1 - d0).num_days() + 1;
    let d0_ist = d1_ist - win_days * 86400;
    let d7_ist = d1_ist - 7 * 86400;
    let lag_ist = d1_ist - 72 * 3600;
    type Hit = ((String, String), bool, bool, bool, i64);
    let mut hits: Vec<Hit> = Vec::new();
    crate::crm::for_each_ticket(&snap.crm, |t, ep| {
        if ep < d7_ist || ep >= d1_ist {
            return;
        }
        let aid = crate::crm::ticket_ad_id(t).unwrap_or_default();
        let Some(key) = ad_key.get(aid.as_str()).cloned() else {
            return;
        };
        let (q, v, b) =
            crate::crm::milestones(&crate::crm::ticket_stage(t), &crate::crm::ticket_funnel(t));
        hits.push((key, q, v, b, ep));
    });
    for (key, q, v, b, ep) in hits {
        if let Some(a) = aggs.get_mut(&key) {
            a.seven.l_leads += 1;
            a.seven.qualified += q as u64;
            if ep >= d0_ist {
                a.win_crm.l_leads += 1;
                a.win_crm.qualified += q as u64;
                a.win_crm.visit += v as u64;
                a.win_crm.booking += b as u64;
            }
            if ep >= lag_ist {
                a.recent_l += 1;
            }
        }
    }

    // Product 7d totals for shares and the quality-leak baseline.
    let mut prod_7d: BTreeMap<&str, (f64, u64, u64)> = BTreeMap::new(); // spend, l, q
    for ((product, _), a) in &aggs {
        let p = prod_7d.entry(product.as_str()).or_default();
        p.0 += a.seven.spend;
        p.1 += a.seven.l_leads;
        p.2 += a.seven.qualified;
    }

    let mut out: BTreeMap<String, Vec<TargetingReport>> = BTreeMap::new();
    for ((product, targeting), a) in &aggs {
        let (pspend, pl, pq) = *prod_7d.get(product.as_str()).unwrap();
        let specs = super::specs::specs_for(snap, &a.adset_ids);
        let verdicts = verdicts(
            a.seven.clone(),
            a.recent_l,
            &specs,
            target_rpq,
            pspend,
            pl,
            pq,
        );
        out.entry(product.clone())
            .or_default()
            .push(TargetingReport {
                targeting: targeting.clone(),
                archetype: super::specs::archetype(targeting),
                specs,
                window: a.win.clone(),
                window_crm: a.win_crm.clone(),
                window_7d: a.seven.clone(),
                recent_l_72h: a.recent_l,
                verdicts,
            });
    }
    for reports in out.values_mut() {
        reports.sort_by(|x, y| y.window.spend.total_cmp(&x.window.spend));
    }
    out
}

/// Verdict precedence per the marketing spec; at most 2 chips.
fn verdicts(
    s7: SevenDay,
    recent_l: u64,
    specs: &Option<TargetingSpecs>,
    target_rpq: f64,
    prod_spend_7d: f64,
    prod_l_7d: u64,
    prod_q_7d: u64,
) -> Vec<Verdict> {
    let mut out = Vec::new();
    if s7.spend == 0.0 {
        return out;
    }
    let immature = recent_l > 0;
    let mult = s7.spend / target_rpq;
    let learning = specs
        .as_ref()
        .and_then(|s| s.learning.as_deref())
        .unwrap_or("");
    fn push(out: &mut Vec<Verdict>, code: &str, label: &str, detail: String) {
        if out.len() < 2 {
            out.push(Verdict {
                code: code.into(),
                label: label.into(),
                detail,
            });
        }
    }
    // 1. Kill: the funnel kill-table, 7d basis, maturation-guarded.
    if s7.qualified <= 2 && mult >= KILL_TABLE[s7.qualified.min(2) as usize] && !immature {
        push(
            &mut out,
            "kill",
            "Kill",
            format!(
                "Rs {:.0} = {:.1}x target with {} qualified in 7d",
                s7.spend, mult, s7.qualified
            ),
        );
    }
    // 2. Zero-CRM spend: capture/form failure alarm, fires before kill mult.
    if s7.spend >= 2000.0 && s7.l_leads == 0 && !immature {
        push(
            &mut out,
            "zero_crm",
            "0 CRM",
            format!("no CRM leads on Rs {:.0} in 7 settled days", s7.spend),
        );
    }
    // 3. Learning-limited: unexited learning starves comparability.
    if learning == "FAIL" || (learning == "LEARNING" && s7.m_leads < 7) {
        push(
            &mut out,
            "learning",
            "Learning",
            format!("{} leads/7d - CPL not comparable yet", s7.m_leads),
        );
    }
    // 5. Quality leak: enough leads, conversion half the product's.
    if s7.l_leads >= 8 && prod_l_7d > 0 && prod_q_7d > 0 {
        let l2q = s7.qualified as f64 / s7.l_leads as f64;
        let prod_l2q = prod_q_7d as f64 / prod_l_7d as f64;
        if l2q < 0.5 * prod_l2q {
            push(
                &mut out,
                "quality_leak",
                "Weak L>Q",
                format!(
                    "{:.0}% L>Q vs product {:.0}%",
                    l2q * 100.0,
                    prod_l2q * 100.0
                ),
            );
        }
    }
    // 7. Scale: proven Rs/Q with headroom, no negative chip, share cap.
    let share = if prod_spend_7d > 0.0 {
        s7.spend / prod_spend_7d
    } else {
        0.0
    };
    if out.is_empty()
        && s7.qualified >= 2
        && s7.spend / s7.qualified as f64 <= 1.5 * target_rpq
        && share < 0.4
        && learning != "FAIL"
    {
        push(
            &mut out,
            "scale",
            "Scale",
            format!(
                "Rs {:.0}/qualified on {:.0}% of spend",
                s7.spend / s7.qualified as f64,
                share * 100.0
            ),
        );
    }
    if out.is_empty() {
        push(&mut out, "wait", "Wait", "no actionable signal yet".into());
    }
    out
}

/// Cross-read A: a creative whose CPL in its best targeting beats its
/// pooled CPL elsewhere by 40%+, with minimum-data gates so 1-lead
/// cells never print. Max 2 lines per product.
pub(super) fn cross_reads(cards: &[CreativeCard]) -> Vec<String> {
    let mut out = Vec::new();
    for c in cards {
        if c.targetings.len() < 2 || out.len() >= 2 {
            continue;
        }
        let mut best: Option<(&str, f64)> = None;
        let (mut rest_spend, mut rest_m) = (0.0, 0u64);
        for t in &c.targetings {
            if t.delivery.m_leads >= 5 && t.delivery.spend >= 2500.0 {
                let cpl = t.delivery.spend / t.delivery.m_leads as f64;
                if best.map(|(_, b)| cpl < b).unwrap_or(true) {
                    if let Some((prev_name, _)) = best {
                        // previous best joins the rest pool
                        let p = c.targetings.iter().find(|x| x.targeting == prev_name);
                        if let Some(p) = p {
                            rest_spend += p.delivery.spend;
                            rest_m += p.delivery.m_leads;
                        }
                    }
                    best = Some((&t.targeting, cpl));
                    continue;
                }
            }
            rest_spend += t.delivery.spend;
            rest_m += t.delivery.m_leads;
        }
        if let Some((tname, bcpl)) = best {
            if rest_m >= 5 {
                let rcpl = rest_spend / rest_m as f64;
                if bcpl <= 0.6 * rcpl {
                    out.push(format!(
                        "\"{}\" earns leads at Rs {:.0} in {} vs Rs {:.0} elsewhere - concentrate it there.",
                        c.display_name, bcpl, tname, rcpl
                    ));
                }
            }
        }
    }
    out
}

fn fmt(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s7(spend: f64, m: u64, l: u64, q: u64) -> SevenDay {
        SevenDay {
            spend,
            impressions: 10_000,
            clicks: 100,
            m_leads: m,
            l_leads: l,
            qualified: q,
        }
    }

    #[test]
    fn kill_fires_only_mature_and_over_table() {
        // 3x target, 0 qualified, mature -> kill.
        let v = verdicts(s7(7500.0, 10, 5, 0), 0, &None, 2500.0, 20_000.0, 20, 4);
        assert_eq!(v[0].code, "kill", "{:?}", v);
        // Same but immature (recent leads) -> never kill; falls to wait.
        let v = verdicts(s7(7500.0, 10, 5, 0), 3, &None, 2500.0, 20_000.0, 20, 4);
        assert!(v.iter().all(|x| x.code != "kill"), "{:?}", v);
    }

    #[test]
    fn zero_crm_alarm_beats_waiting() {
        let v = verdicts(s7(2500.0, 8, 0, 0), 0, &None, 2500.0, 20_000.0, 20, 4);
        assert!(v.iter().any(|x| x.code == "zero_crm"), "{:?}", v);
    }

    #[test]
    fn scale_needs_clean_slate_and_headroom() {
        let v = verdicts(s7(6000.0, 20, 15, 3), 1, &None, 2500.0, 40_000.0, 60, 10);
        assert_eq!(v[0].code, "scale", "{:?}", v);
        // Over 40% share: no scale, falls to wait.
        let v = verdicts(s7(6000.0, 20, 15, 3), 1, &None, 2500.0, 10_000.0, 60, 10);
        assert!(v.iter().all(|x| x.code != "scale"), "{:?}", v);
    }

    #[test]
    fn archetypes_classify_names() {
        assert_eq!(super::super::specs::archetype("Pincode Ad"), "Pincode");
        assert_eq!(super::super::specs::archetype("Lookalike 2%"), "Lookalike");
        assert_eq!(
            super::super::specs::archetype("NonAdvantage+ RK"),
            "Broad-Income"
        );
        assert_eq!(
            super::super::specs::archetype("Detailed Targeting NRI"),
            "Detailed"
        );
    }
}
