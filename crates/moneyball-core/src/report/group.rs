//! Grouping + aggregation internals for the creative report. The rules
//! here are ported from pipeline/creative_report.py (the behavioral
//! spec) - see each fn's doc for its python counterpart.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::card::{non_empty, truncate10};
use crate::schema::*;
use crate::snapshot::AdsDailyRow;

/// Per-ad delivery over the window (python's `ads[aid]` dict).
#[derive(Default, Clone)]
pub(super) struct AdAgg {
    pub product: String,
    pub ad_name: String,
    pub campaign: String,
    /// base_targeting(adset_name) - campaign suffix "(...)" stripped so
    /// targetings line up across campaigns (python base_targeting).
    pub targeting: String,
    pub spend: f64,
    pub impressions: u64,
    pub reach: u64,
    pub clicks: u64,
    pub m_leads: u64,
    /// date -> (impressions, clicks, m_leads) for the trend series.
    pub daily: BTreeMap<String, (u64, u64, u64)>,
}

impl AdAgg {
    pub fn add_row(&mut self, r: &AdsDailyRow) {
        if self.ad_name.is_empty() {
            self.product = r._product.clone();
            self.ad_name = r.ad_name.clone();
            self.campaign = r.campaign_name.clone();
            self.targeting = base_targeting(&r.adset_name);
        }
        let m = crate::brief::count_m_leads(&r.actions);
        self.spend += r.spend_num();
        self.impressions += r.impressions_num();
        self.reach += parse_u64(&r.reach);
        self.clicks += r.clicks_num();
        self.m_leads += m;
        let d = self.daily.entry(r.date_start.clone()).or_default();
        d.0 += r.impressions_num();
        d.1 += r.clicks_num();
        d.2 += m;
    }
}

fn parse_u64(s: &str) -> u64 {
    s.trim().parse().unwrap_or(0)
}

/// python base_targeting: strip a trailing "(...)" chunk.
pub(super) fn base_targeting(name: &str) -> String {
    let t = name.trim();
    // rfind('(') returns a char boundary ('(' is one byte), so split_at
    // is safe; lead names around it may be multibyte.
    let stripped = match (t.rfind('('), t.ends_with(')')) {
        (Some(i), true) => t.split_at(i).0.trim_end(),
        _ => t,
    };
    if stripped.is_empty() {
        "-".into()
    } else {
        stripped.to_string()
    }
}

/// python video_name_key's suffix rule: strip a trailing
/// "<dashes> copy [N]" (case-insensitive) from an ad name. All byte
/// positions come from char_indices of the ORIGINAL string - indexes
/// into a to_lowercase() copy would drift on multibyte names.
pub(super) fn strip_copy_suffix(name: &str) -> String {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    // Last case-insensitive "copy": (start byte, end byte) in `name`.
    let hit = chars
        .windows(4)
        .rev()
        .find(|w| {
            w.iter()
                .map(|(_, c)| c.to_ascii_lowercase())
                .eq("copy".chars())
        })
        .map(|w| (w[0].0, w[3].0 + w[3].1.len_utf8()));
    let Some((start, end)) = hit else {
        return name.trim().to_string();
    };
    // After "copy": only spaces/digits allowed.
    let after = name.split_at(end).1;
    if !after.chars().all(|c| c.is_ascii_digit() || c == ' ') {
        return name.trim().to_string();
    }
    // Before "copy": whitespace then at least one dash char.
    let trimmed = name.split_at(start).0.trim_end();
    let mut saw_dash = false;
    let mut cut = trimmed.len();
    for c in trimmed.chars().rev() {
        if c == '-' || c == '\u{2013}' || c == '\u{2014}' {
            saw_dash = true;
            cut -= c.len_utf8();
        } else {
            break;
        }
    }
    if !saw_dash {
        return name.trim().to_string();
    }
    trimmed.split_at(cut).0.trim_end().to_string()
}

/// The grouping precedence (python `ckey`, minus hand-curated families
/// which need editorial config that doesn't exist here yet):
/// video-name (re-upload-proof) > video id > image hash > CDN basename
/// > own ad id (unresolvable ads keep their delivery, never dropped).
pub(super) fn group_key(ad_id: &str, agg: &AdAgg, cr: Option<&CreativeRow>) -> (GroupKind, String) {
    if let Some(c) = cr {
        if c.is_video {
            let name = if c.ad_name.is_empty() {
                &agg.ad_name
            } else {
                &c.ad_name
            };
            let base = strip_copy_suffix(name).to_lowercase();
            if !base.is_empty() {
                return (GroupKind::VideoName, format!("vidname:{}", base));
            }
            if let Some(v) = c
                .video_id
                .as_deref()
                .or(c.afs_video_ids.first().map(String::as_str))
            {
                return (GroupKind::VideoId, format!("vid:{}", v));
            }
        }
        if let Some(h) = &c.image_hash {
            return (GroupKind::ImageHash, format!("img:{}", h));
        }
        if let Some(b) = &c.image_basename {
            return (GroupKind::ImageBasename, format!("img:{}", b));
        }
    }
    (GroupKind::AdId, format!("ad:{}", ad_id))
}

/// One creative group being accumulated (python `_new_group()`).
pub(super) struct GroupAgg {
    pub(super) kind: GroupKind,
    pub(super) ad_ids: Vec<String>,
    pub(super) names: BTreeMap<String, u64>,
    pub(super) campaigns: Vec<String>,
    pub(super) is_video: bool,
    pub(super) statuses: BTreeMap<String, u64>,
    pub(super) live_adsets: Vec<String>,
    pub(super) created: Option<String>,
    pub(super) permalink: Option<String>,
    pub(super) image: Option<ImageRef>,
    pub(super) delivery: Delivery,
    pub(super) crm: TargetingCrm,
    pub(super) targetings: BTreeMap<String, (Delivery, TargetingCrm)>,
    pub(super) daily: BTreeMap<String, TrendBucket>,
}

impl GroupAgg {
    pub fn new(kind: GroupKind) -> Self {
        GroupAgg {
            kind,
            ad_ids: Vec::new(),
            names: BTreeMap::new(),
            campaigns: Vec::new(),
            is_video: false,
            statuses: BTreeMap::new(),
            live_adsets: Vec::new(),
            created: None,
            permalink: None,
            image: None,
            delivery: Delivery::default(),
            crm: TargetingCrm::default(),
            targetings: BTreeMap::new(),
            daily: BTreeMap::new(),
        }
    }

    pub fn add_ad(&mut self, ad_id: &str, agg: &AdAgg, cr: Option<&CreativeRow>) {
        self.ad_ids.push(ad_id.to_string());
        *self.names.entry(non_empty(&agg.ad_name)).or_default() += 1;
        if !agg.campaign.is_empty() && !self.campaigns.contains(&agg.campaign) {
            self.campaigns.push(agg.campaign.clone());
        }
        self.delivery.spend += agg.spend;
        self.delivery.impressions += agg.impressions;
        self.delivery.reach += agg.reach;
        self.delivery.clicks += agg.clicks;
        self.delivery.m_leads += agg.m_leads;
        let t = self.targetings.entry(agg.targeting.clone()).or_default();
        t.0.spend += agg.spend;
        t.0.impressions += agg.impressions;
        t.0.clicks += agg.clicks;
        t.0.m_leads += agg.m_leads;
        for (date, (imp, clk, ml)) in &agg.daily {
            let b = self
                .daily
                .entry(date.clone())
                .or_insert_with(|| TrendBucket {
                    date: date.clone(),
                    ..Default::default()
                });
            b.impressions += imp;
            b.clicks += clk;
            b.m_leads += ml;
        }
        if let Some(c) = cr {
            self.is_video |= c.is_video;
            *self
                .statuses
                .entry(c.status.clone().unwrap_or_else(|| "?".into()))
                .or_default() += 1;
            if c.status.as_deref() == Some("ACTIVE") && !c.adset_id.is_empty() {
                self.live_adsets.push(c.adset_id.clone());
            }
            if let Some(ct) = c.created_time.as_deref().map(truncate10) {
                self.created = match self.created.take() {
                    Some(prev) if prev <= ct => Some(prev),
                    _ => Some(ct),
                };
            }
            if self.permalink.is_none() {
                self.permalink = c.permalink.clone().filter(|p| !p.is_empty());
            }
            if self.image.is_none() {
                if let Some(a) = &c.asset {
                    self.image = Some(ImageRef {
                        sha256: a.sha256.clone(),
                        path: format!(
                            "assets/creatives/{}",
                            crate::fetch::assets::rel_path(&a.sha256, &a.content_type).display()
                        ),
                    });
                }
            }
        }
    }

    pub fn add_ticket(
        &mut self,
        q: bool,
        v: bool,
        b: bool,
        targeting: &str,
        ep: i64,
        snap_date: NaiveDate,
    ) {
        self.crm.l_leads += 1;
        self.crm.qualified += q as u64;
        self.crm.visit += v as u64;
        self.crm.booking += b as u64;
        let t = self.targetings.entry(targeting.to_string()).or_default();
        t.1.l_leads += 1;
        t.1.qualified += q as u64;
        t.1.visit += v as u64;
        t.1.booking += b as u64;
        // Trend bucket by IST calendar day of the delivery epoch.
        let day_offset = (crate::brief::ist_midnight_epoch(snap_date) - ep) / 86400;
        if let Some(date) = snap_date
            .pred_opt()
            .and_then(|d| d.checked_sub_days(chrono::Days::new(day_offset.max(0) as u64)))
            .map(|d| d.format("%Y-%m-%d").to_string())
        {
            if let Some(bucket) = self.daily.get_mut(&date) {
                bucket.l_leads += 1;
                bucket.qualified += q as u64;
                bucket.visit += v as u64;
                bucket.booking += b as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_targeting_strips_campaign_suffix() {
        assert_eq!(base_targeting("Income (AI)"), "Income");
        assert_eq!(base_targeting("Lookalike 2% (NM - Leads)"), "Lookalike 2%");
        assert_eq!(base_targeting("Broad"), "Broad");
        assert_eq!(base_targeting(""), "-");
        assert_eq!(base_targeting("(only suffix)"), "-");
    }

    #[test]
    fn copy_suffix_stripping_matches_python_rule() {
        assert_eq!(strip_copy_suffix("NM Video - Copy 2"), "NM Video");
        assert_eq!(strip_copy_suffix("NM Video \u{2013} copy"), "NM Video");
        assert_eq!(strip_copy_suffix("NM Video - COPY 10"), "NM Video");
        // No dash before copy: not the auto-suffix, keep verbatim.
        assert_eq!(strip_copy_suffix("Copywriting Ad"), "Copywriting Ad");
        assert_eq!(strip_copy_suffix("Great copy"), "Great copy");
    }

    #[test]
    fn group_key_precedence() {
        let agg = AdAgg {
            ad_name: "Video X - Copy 3".into(),
            ..Default::default()
        };
        let video = CreativeRow {
            ad_id: "a".into(),
            ad_name: "Video X - Copy 3".into(),
            is_video: true,
            video_id: Some("v1".into()),
            image_hash: Some("h".into()),
            ..Default::default()
        };
        assert_eq!(
            group_key("a", &agg, Some(&video)),
            (GroupKind::VideoName, "vidname:video x".into())
        );
        let image = CreativeRow {
            ad_id: "a".into(),
            image_hash: Some("h1".into()),
            image_basename: Some("x.jpg".into()),
            ..Default::default()
        };
        assert_eq!(
            group_key("a", &agg, Some(&image)),
            (GroupKind::ImageHash, "img:h1".into())
        );
        let basename_only = CreativeRow {
            ad_id: "a".into(),
            image_basename: Some("x.jpg".into()),
            ..Default::default()
        };
        assert_eq!(
            group_key("a", &agg, Some(&basename_only)),
            (GroupKind::ImageBasename, "img:x.jpg".into())
        );
        assert_eq!(
            group_key("a9", &agg, None),
            (GroupKind::AdId, "ad:a9".into())
        );
    }
}
