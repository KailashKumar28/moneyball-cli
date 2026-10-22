//! `diagnose` - the checks to run on a product before cutting anything:
//! where its leads die in the CRM, which ads are tiring, how the last 7
//! days compare with the weeks before, and what changed in its ad
//! account. Pure over one snapshot; mb.py's stages / creatives / history
//! / changes, natively (its geo check needs city-level delivery, which
//! Meta does not report for India).

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Local, NaiveDate};

use crate::config::AppConfig;
use crate::crm::{self, QUALIFIED_PLUS, VISIT_PLUS};
use crate::schema::ChangeRow;
use crate::snapshot::{AdsDailyRow, Snapshot};

mod print;
pub use print::{run, text};

/// The recent window, in complete days before the snapshot day.
pub const RECENT_DAYS: i64 = 7;
/// How far back "before" reaches (a pull holds 28 days).
const HISTORY_DAYS: i64 = 28;
/// Logged changes no person made: an automatic audience, an ad's first
/// delivery.
const NOISE: &[&str] = &["create_audience", "first_delivery_event"];

pub struct Diagnosis {
    pub product: String,
    pub snapshot_date: String,
    /// None when the snapshot has no CRM data. Over `before` and
    /// `recent` together, as the trend.
    pub stages: Option<Vec<StageRow>>,
    /// Ads that spent in the last 7 days, most spend first.
    pub ads: Vec<AdRow>,
    pub recent: Window,
    pub before: Window,
    /// None when the snapshot holds no change log.
    pub changes: Option<Vec<ChangeRow>>,
}

/// One campaign's CRM leads by how far they got.
pub struct StageRow {
    pub campaign: String,
    pub leads: u64,
    pub lost: u64,
    pub unreachable: u64,
    pub qualified: u64,
    pub visited: u64,
    pub spend: u64,
    pub call: StageCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageCall {
    /// Over half Lost before contact: the form or the audience's intent.
    Form,
    /// Over 40% unreachable: bad phone numbers.
    Data,
    /// Leads reach contact but under a fifth of them visit: the sales
    /// team's speed or script.
    Sales,
    Healthy,
    /// No lead reached contact yet.
    Stuck,
}

/// One ad over the last 7 days.
pub struct AdRow {
    pub name: String,
    pub adset: String,
    pub spend: u64,
    pub m: u64,
    /// Link click-through rate, %, and the ad's own over the days before
    /// (when it had the impressions to say).
    pub ctr: Option<f64>,
    pub ctr_before: Option<f64>,
    /// How many times each person saw it over the 7 days, as Meta counts
    /// the week (ads_7d.json); None in a snapshot without it.
    pub freq: Option<f64>,
    pub age_days: Option<i64>,
    pub call: Fatigue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fatigue {
    /// Clicked 30%+ less than in its own earlier days while each person
    /// saw it twice or more in the week.
    Fatigued,
    /// Clicked 30%+ less than before, or seen three times or more.
    Watch,
    Ok,
    /// No impressions to judge by.
    Unknown,
}

/// A run of days: its length and totals; `q` None without CRM data.
#[derive(Debug, Clone, Copy, Default)]
pub struct Window {
    pub days: i64,
    pub spend: f64,
    pub m: u64,
    pub q: Option<u64>,
}

impl Window {
    pub fn per_day(&self, v: f64) -> f64 {
        v / self.days.max(1) as f64
    }
    pub fn cpl(&self) -> Option<f64> {
        (self.m > 0).then(|| self.spend / self.m as f64)
    }
    pub fn rs_per_q(&self) -> Option<f64> {
        self.q.filter(|q| *q > 0).map(|q| self.spend / q as f64)
    }
}

pub fn compute(snap: &Snapshot, cfg: &AppConfig, product: &str) -> Diagnosis {
    let day = NaiveDate::parse_from_str(&snap.date, "%Y-%m-%d")
        .expect("snapshot::load validated the date");
    let ymd = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let rows: Vec<&AdsDailyRow> = snap
        .ads_daily
        .iter()
        .filter(|r| r._product == product)
        .collect();
    let recent_from = day - Duration::days(RECENT_DAYS);
    // "Before" starts where the data does, at most HISTORY_DAYS back.
    let first = rows
        .iter()
        .filter_map(|r| NaiveDate::parse_from_str(&r.date_start, "%Y-%m-%d").ok())
        .min()
        .unwrap_or(recent_from)
        .max(day - Duration::days(HISTORY_DAYS));
    let has_crm = !crm::is_empty(&snap.crm);
    let window = |from: NaiveDate, to: NaiveDate| {
        let (from_s, to_s) = (ymd(from), ymd(to));
        let mut w = Window {
            days: (to - from).num_days() + 1,
            ..Default::default()
        };
        for r in rows
            .iter()
            .filter(|r| r.date_start >= from_s && r.date_start <= to_s)
        {
            w.spend += r.spend_num();
            w.m += crate::brief::count_m_leads(&r.actions);
        }
        w.q = has_crm.then(|| qualified_between(snap, &rows, from, to));
        w
    };
    let recent = window(recent_from, day - Duration::days(1));
    let before = match first < recent_from {
        true => window(first, recent_from - Duration::days(1)),
        false => Window::default(),
    };
    Diagnosis {
        product: product.to_string(),
        snapshot_date: snap.date.clone(),
        stages: has_crm.then(|| stages(snap, &rows, first, day)),
        ads: ads(snap, &rows, &ymd(recent_from), day),
        recent,
        before,
        changes: snap.changes.as_ref().map(|c| {
            let account = cfg
                .workspace
                .as_ref()
                .and_then(|w| w.products.iter().find(|p| p.name == product))
                .map(|p| crate::fetch::act_id(&p.ad_account))
                .unwrap_or_default();
            let since = recent_from.and_hms_opt(0, 0, 0).expect("midnight exists");
            c.rows
                .iter()
                .filter(|r| r.account == account && !NOISE.contains(&r.event_type.as_str()))
                .filter(|r| local_time(&r.time).is_some_and(|t| t.naive_local() >= since))
                .cloned()
                .collect()
        }),
    }
}

/// Meta's "2026-10-08T11:18:00+0000", in local time.
pub fn local_time(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z")
        .ok()
        .map(|t| t.with_timezone(&Local))
}

/// Qualified CRM leads delivered on [from, to] (IST days) from these ads.
fn qualified_between(
    snap: &Snapshot,
    rows: &[&AdsDailyRow],
    from: NaiveDate,
    to: NaiveDate,
) -> u64 {
    let ads: std::collections::HashSet<&str> = rows.iter().map(|r| r.ad_id.as_str()).collect();
    let (start, end) = (
        crate::brief::ist_midnight_epoch(from),
        crate::brief::ist_midnight_epoch(to + Duration::days(1)),
    );
    let mut q = 0;
    crm::for_each_ticket(&snap.crm, |t, ep| {
        let ours = crm::ticket_ad_id(t).is_some_and(|a| ads.contains(a.as_str()));
        if ours
            && ep >= start
            && ep < end
            && QUALIFIED_PLUS.contains(&crm::ticket_stage(t).as_str())
        {
            q += 1;
        }
    });
    q
}

/// The CRM leads from this product's ads delivered on [from, day) (IST
/// days), by campaign and how far each got - over the same days as the
/// spend beside them.
fn stages(
    snap: &Snapshot,
    rows: &[&AdsDailyRow],
    from: NaiveDate,
    day: NaiveDate,
) -> Vec<StageRow> {
    let from_s = from.format("%Y-%m-%d").to_string();
    // Keyed by id: two campaigns may share a name.
    let mut campaign_of: HashMap<&str, (&str, &str)> = HashMap::new();
    let mut spend: HashMap<&str, f64> = HashMap::new();
    for r in rows.iter().filter(|r| r.date_start >= from_s) {
        campaign_of.insert(&r.ad_id, (&r.campaign_id, &r.campaign_name));
        *spend.entry(&r.campaign_id).or_default() += r.spend_num();
    }
    let (start, end) = (
        crate::brief::ist_midnight_epoch(from),
        crate::brief::ist_midnight_epoch(day),
    );
    let mut by: BTreeMap<&str, StageRow> = BTreeMap::new();
    crm::for_each_ticket(&snap.crm, |t, ep| {
        if ep < start || ep >= end {
            return;
        }
        let Some((id, name)) =
            crm::ticket_ad_id(t).and_then(|a| campaign_of.get(a.as_str()).copied())
        else {
            return;
        };
        let e = by.entry(id).or_insert_with(|| StageRow {
            campaign: name.to_string(),
            leads: 0,
            lost: 0,
            unreachable: 0,
            qualified: 0,
            visited: 0,
            spend: spend.get(id).copied().unwrap_or(0.0).round() as u64,
            call: StageCall::Stuck,
        });
        let stage = crm::ticket_stage(t);
        e.leads += 1;
        e.lost += (stage == "Lost") as u64;
        e.unreachable += (stage == "NonContactable") as u64;
        e.qualified += QUALIFIED_PLUS.contains(&stage.as_str()) as u64;
        e.visited += VISIT_PLUS.contains(&stage.as_str()) as u64;
    });
    let mut out: Vec<StageRow> = by.into_values().collect();
    for r in &mut out {
        r.call = stage_call(r);
    }
    out.sort_by(|a, b| {
        b.leads
            .cmp(&a.leads)
            .then_with(|| a.campaign.cmp(&b.campaign))
    });
    out
}

/// mb.py's `_diagnose`, over cumulative stages: a visit counts as having
/// been reached too.
fn stage_call(r: &StageRow) -> StageCall {
    let share = |n: u64| n as f64 / r.leads.max(1) as f64;
    if share(r.lost) > 0.5 {
        StageCall::Form
    } else if share(r.unreachable) > 0.4 {
        StageCall::Data
    } else if r.qualified >= 5 && (r.visited as f64) < 0.2 * r.qualified as f64 {
        StageCall::Sales
    } else if r.qualified > 0 {
        StageCall::Healthy
    } else {
        StageCall::Stuck
    }
}

/// Every ad that spent since `from` (the last 7 days), with its fatigue
/// call.
fn ads(snap: &Snapshot, rows: &[&AdsDailyRow], from: &str, day: NaiveDate) -> Vec<AdRow> {
    /// [0] the last 7 days, [1] the days before.
    #[derive(Default)]
    struct Agg<'a> {
        name: &'a str,
        adset: &'a str,
        spend: f64,
        m: u64,
        impressions: [u64; 2],
        clicks: [u64; 2],
    }
    let mut by: HashMap<&str, Agg> = HashMap::new();
    for r in rows {
        let recent = r.date_start.as_str() >= from;
        let i = usize::from(!recent);
        let a = by.entry(&r.ad_id).or_default();
        (a.name, a.adset) = (&r.ad_name, &r.adset_name);
        a.impressions[i] += r.impressions_num();
        a.clicks[i] += r.clicks_num();
        if recent {
            a.spend += r.spend_num();
            a.m += crate::brief::count_m_leads(&r.actions);
        }
    }
    let week: HashMap<&str, f64> = snap
        .ads_7d
        .iter()
        .flat_map(|w| &w.rows)
        .map(|r| (r.ad_id.as_str(), r.frequency))
        .collect();
    let created: HashMap<&str, &str> = snap
        .creatives
        .iter()
        .flat_map(|c| &c.rows)
        .filter_map(|c| Some((c.ad_id.as_str(), c.created_time.as_deref()?)))
        .collect();
    let mut out: Vec<AdRow> = by
        .into_iter()
        .filter(|(_, a)| a.spend > 0.0)
        .map(|(id, a)| {
            let ctr = |i: usize, floor: u64| {
                (a.impressions[i] > floor)
                    .then(|| a.clicks[i] as f64 / a.impressions[i] as f64 * 100.0)
            };
            let (ctr_shown, ctr_before) = (ctr(0, 0), ctr(1, MIN_IMPRESSIONS));
            let freq = week.get(id).copied();
            AdRow {
                name: if a.name.is_empty() { id } else { a.name }.to_string(),
                adset: a.adset.to_string(),
                spend: a.spend.round() as u64,
                m: a.m,
                ctr: ctr_shown,
                ctr_before,
                freq,
                age_days: created
                    .get(id)
                    .and_then(|t| local_time(t))
                    .map(|t| (day - t.date_naive()).num_days()),
                // Judged only on enough impressions: 40 with no click is noise.
                call: fatigue(ctr(0, MIN_IMPRESSIONS), ctr_before, freq),
            }
        })
        .collect();
    // HashMap order is random: ties break on the names.
    out.sort_by(|a, b| {
        b.spend
            .cmp(&a.spend)
            .then_with(|| (&a.name, &a.adset).cmp(&(&b.name, &b.adset)))
    });
    out
}

/// Fewer impressions than this say nothing about an ad's CTR.
const MIN_IMPRESSIONS: u64 = 1000;

/// An ad tires when people see it more and click it less: its link CTR
/// against its own earlier days, and how often each person sees it.
/// (mb.py held CTR to fixed 0.7% / 1% floors; on an account whose ads
/// normally run near 0.8%, that called every ad "watch".)
fn fatigue(ctr: Option<f64>, before: Option<f64>, freq: Option<f64>) -> Fatigue {
    let Some(ctr) = ctr else {
        return Fatigue::Unknown;
    };
    let fell = before.is_some_and(|b| ctr < 0.7 * b);
    let freq = freq.unwrap_or(0.0);
    if fell && freq >= 2.0 {
        Fatigue::Fatigued
    } else if fell || freq >= 3.0 {
        Fatigue::Watch
    } else {
        Fatigue::Ok
    }
}

#[cfg(test)]
mod tests;
