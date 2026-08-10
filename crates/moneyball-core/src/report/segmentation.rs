//! Lead segmentation - explaining the M-Leads -> L-Leads gap per
//! creative (python weekly_funnel_report.lead_segmentation, ported).
//!
//! Window Meta leads (leads.json), classified in submission order:
//!   captured   - lead_id present in the CRM tickets of ITS campaign
//!   invalid    - phone is not a valid Indian mobile (CRM rejects)
//!   duplicate  - contact already in this campaign, or already seen
//!                earlier in the window
//!   reinquiry  - contact exists in the CRM under ANOTHER campaign
//!                (or as an organic ticket) - a returning person
//!   uncaptured - valid, unique, new, but absent from the CRM: a
//!                genuine sync gap worth recovering
//! Known limit vs the python: the CRM index covers the PULLED book
//! (crm fetch --days window + organic contacts), not lifetime history,
//! so a re-inquirer older than that classifies as uncaptured.

use std::collections::{BTreeMap, HashSet};

use crate::schema::Segmentation;
use crate::snapshot::Snapshot;

/// Per-ad segmentation over [d0s, d1s] (inclusive IST calendar dates).
/// Empty map when the snapshot has no leads.json.
pub(super) fn per_ad(snap: &Snapshot, d0s: &str, d1s: &str) -> BTreeMap<String, Segmentation> {
    let Some(leads) = &snap.leads else {
        return BTreeMap::new();
    };

    // ad -> campaign (window ads; a lead on an unknown ad still counts,
    // scoped to an empty campaign id).
    let ad_campaign: BTreeMap<&str, &str> = snap
        .ads_daily
        .iter()
        .map(|r| (r.ad_id.as_str(), r.campaign_id.as_str()))
        .collect();

    // CRM index: per-campaign lead_ids/phones/emails + the global sets.
    let mut cam_ids: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut cam_ph: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut cam_em: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let (mut g_ph, mut g_em) = (HashSet::new(), HashSet::new());
    crate::crm::for_each_ticket(&snap.crm, |t, _ep| {
        let aid = crate::crm::ticket_ad_id(t).unwrap_or_default();
        let cid = ad_campaign
            .get(aid.as_str())
            .copied()
            .unwrap_or("")
            .to_string();
        if let Some(lid) = t.get("lead_id").and_then(|v| v.as_str()) {
            cam_ids.entry(cid.clone()).or_default().insert(lid.into());
        }
        if let Some(p) = t.get("phone").and_then(|v| v.as_str()) {
            let n = norm_phone(p);
            if valid_phone(&n) {
                cam_ph.entry(cid.clone()).or_default().insert(n.clone());
                g_ph.insert(n);
            }
        }
        if let Some(e) = t.get("email").and_then(|v| v.as_str()) {
            let e = e.trim().to_lowercase();
            if !e.is_empty() {
                cam_em.entry(cid).or_default().insert(e.clone());
                g_em.insert(e);
            }
        }
    });
    // Organic tickets never join a campaign but their contacts make a
    // window lead a RE-INQUIRY, not a new person.
    for c in snap.crm_contacts.iter().flat_map(|f| f.rows.iter()) {
        if let Some(p) = c.phone.as_deref() {
            let n = norm_phone(p);
            if valid_phone(&n) {
                g_ph.insert(n);
            }
        }
        if let Some(e) = c.email.as_deref() {
            let e = e.trim().to_lowercase();
            if !e.is_empty() {
                g_em.insert(e);
            }
        }
    }

    // Window leads in submission order (python sorts by created_time).
    let mut window: Vec<&crate::schema::LeadRow> = leads
        .rows
        .iter()
        .filter(|l| {
            let d = l.created_time.get(..10).unwrap_or("");
            d >= d0s && d <= d1s
        })
        .collect();
    window.sort_by(|a, b| a.created_time.cmp(&b.created_time));

    let empty: HashSet<String> = HashSet::new();
    let (mut seen_ph, mut seen_em) = (HashSet::new(), HashSet::new());
    let mut out: BTreeMap<String, Segmentation> = BTreeMap::new();
    for l in window {
        let cid = ad_campaign.get(l.ad_id.as_str()).copied().unwrap_or("");
        let ids = cam_ids.get(cid).unwrap_or(&empty);
        let cph = cam_ph.get(cid).unwrap_or(&empty);
        let cem = cam_em.get(cid).unwrap_or(&empty);
        let ph = norm_phone(l.phone.as_deref().unwrap_or(""));
        let em = l.email.as_deref().unwrap_or("").trim().to_lowercase();

        let seg = out.entry(l.ad_id.clone()).or_default();
        seg.total += 1;
        if !l.lead_id.is_empty() && ids.contains(&l.lead_id) {
            seg.captured += 1;
        } else if !valid_phone(&ph) {
            seg.invalid += 1;
        } else if cph.contains(&ph)
            || seen_ph.contains(&ph)
            || (!em.is_empty() && (cem.contains(&em) || seen_em.contains(&em)))
        {
            seg.duplicate += 1;
        } else if g_ph.contains(&ph) || (!em.is_empty() && g_em.contains(&em)) {
            seg.reinquiry += 1;
        } else {
            seg.uncaptured += 1;
        }
        if valid_phone(&ph) {
            seen_ph.insert(ph);
        }
        if !em.is_empty() {
            seen_em.insert(em);
        }
    }
    out
}

/// python _norm_phone: digits only; strip a 91 country code or a
/// leading 0 (Indian numbers).
pub(super) fn norm_phone(p: &str) -> String {
    let d: String = p.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() == 12 && d.starts_with("91") {
        return d.chars().skip(2).collect();
    }
    if d.len() == 11 && d.starts_with('0') {
        return d.chars().skip(1).collect();
    }
    d
}

/// python _valid_phone: 10 digits starting 6-9.
pub(super) fn valid_phone(d: &str) -> bool {
    d.len() == 10 && d.starts_with(['6', '7', '8', '9'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_normalization_matches_python() {
        assert_eq!(norm_phone("+91 98765 43210"), "9876543210");
        assert_eq!(norm_phone("09876543210"), "9876543210");
        assert_eq!(norm_phone("98765-43210"), "9876543210");
        assert!(valid_phone("9876543210"));
        assert!(!valid_phone("1234567890"), "must start 6-9");
        assert!(!valid_phone("987654321"), "10 digits exactly");
    }
}
