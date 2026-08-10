//! Card assembly + KPI roll-ups: turning an accumulated GroupAgg into
//! the schema's CreativeCard, and summing sections/portfolio. Split
//! from group.rs (size cap).

use chrono::NaiveDate;

use super::group::GroupAgg;
use crate::schema::*;
use crate::snapshot::Snapshot;

impl GroupAgg {
    /// python status_label mapping (ASCII labels).
    fn status(&self, snap: &Snapshot) -> CardStatus {
        let get = |k: &str| self.statuses.get(k).copied().unwrap_or(0);
        if get("ACTIVE") > 0 {
            let stages: Vec<String> = self
                .live_adsets
                .iter()
                .filter_map(|a| {
                    snap.adsets
                        .get(a)
                        .and_then(|x| x.pointer("/learning_stage_info/status"))
                        .and_then(|s| s.as_str())
                        .map(String::from)
                })
                .collect();
            if stages.iter().any(|s| s == "LEARNING") {
                return CardStatus {
                    code: StatusCode::Learn,
                    label: "Live - learning".into(),
                };
            }
            if stages.iter().any(|s| s == "FAIL") {
                return CardStatus {
                    code: StatusCode::Learn,
                    label: "Live - learning limited".into(),
                };
            }
            return CardStatus {
                code: StatusCode::Live,
                label: "Live".into(),
            };
        }
        if get("PENDING_REVIEW") > 0 {
            return CardStatus {
                code: StatusCode::Learn,
                label: "In review".into(),
            };
        }
        if get("DISAPPROVED") > 0 {
            return CardStatus {
                code: StatusCode::Stop,
                label: "Rejected".into(),
            };
        }
        CardStatus {
            code: StatusCode::Stop,
            label: "Stopped".into(),
        }
    }

    pub fn into_card(
        self,
        key: String,
        d0: NaiveDate,
        d1: NaiveDate,
        snap: &Snapshot,
    ) -> CreativeCard {
        let status = self.status(snap);
        let display_name = self
            .names
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "-".into());
        let funnel = vec![
            stage("Impressions", self.delivery.impressions),
            stage("Clicks", self.delivery.clicks),
            stage("M-Leads", self.delivery.m_leads),
            stage("L-Leads", self.crm.l_leads),
            stage("Qualified", self.crm.qualified),
            stage("Visit", self.crm.visit),
            stage("Booking", self.crm.booking),
        ];
        // Trend: every window day present, oldest first, zeros filled.
        let mut trend = Vec::new();
        let mut d = d0;
        while d <= d1 {
            let ds = d.format("%Y-%m-%d").to_string();
            trend.push(self.daily.get(&ds).cloned().unwrap_or(TrendBucket {
                date: ds,
                ..Default::default()
            }));
            d += chrono::Duration::days(1);
        }
        let targetings = self
            .targetings
            .into_iter()
            .map(|(targeting, (delivery, crm))| TargetingBreakdown {
                targeting,
                delivery,
                crm,
            })
            .collect();
        CreativeCard {
            group_key: key,
            group_kind: self.kind,
            display_name,
            ad_ids: self.ad_ids,
            campaigns: self.campaigns,
            is_video: self.is_video,
            status,
            created: self.created,
            permalink: self.permalink,
            image: self.image,
            delivery: self.delivery,
            funnel,
            targetings,
            trend,
            segmentation: None, // attached by the segmentation pass
        }
    }
}

pub(super) fn stage(name: &str, count: u64) -> FunnelStage {
    FunnelStage {
        stage: name.into(),
        count,
    }
}

pub(super) fn non_empty(s: &str) -> String {
    if s.is_empty() {
        "-".into()
    } else {
        s.into()
    }
}

pub(super) fn truncate10(s: &str) -> String {
    s.chars().take(10).collect()
}

/// Sum a product section's cards into its KPI block.
pub(super) fn section_kpis(cards: &[CreativeCard]) -> Kpis {
    let mut k = Kpis::default();
    for c in cards {
        k.spend += c.delivery.spend;
        k.impressions += c.delivery.impressions;
        k.clicks += c.delivery.clicks;
        k.funnel.m_leads += c.funnel[2].count;
        k.funnel.l_leads += c.funnel[3].count;
        k.funnel.qualified += c.funnel[4].count;
        k.funnel.visit += c.funnel[5].count;
        k.funnel.booking += c.funnel[6].count;
    }
    finish_kpis(k)
}

pub(super) fn portfolio_kpis(sections: &[ProductSection]) -> Kpis {
    let mut k = Kpis::default();
    for s in sections {
        k.spend += s.kpis.spend;
        k.impressions += s.kpis.impressions;
        k.clicks += s.kpis.clicks;
        k.funnel.m_leads += s.kpis.funnel.m_leads;
        k.funnel.l_leads += s.kpis.funnel.l_leads;
        k.funnel.qualified += s.kpis.funnel.qualified;
        k.funnel.visit += s.kpis.funnel.visit;
        k.funnel.booking += s.kpis.funnel.booking;
    }
    finish_kpis(k)
}

fn finish_kpis(mut k: Kpis) -> Kpis {
    k.spend = (k.spend * 100.0).round() / 100.0;
    k.cost_per_qualified = (k.funnel.qualified > 0)
        .then(|| ((k.spend / k.funnel.qualified as f64) * 10.0).round() / 10.0);
    k.l_to_q_pct = (k.funnel.l_leads > 0)
        .then(|| (k.funnel.qualified as f64 / k.funnel.l_leads as f64 * 1000.0).round() / 10.0);
    k
}
