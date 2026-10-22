//! Moving recs on without being asked: applied from the change log,
//! expired past their date, settled from the snapshot. Split from mod.rs
//! (size cap).

use chrono::{Duration, NaiveDate};

use super::*;
use crate::diagnose::local_time;
use crate::schema::ChangeRow;

/// Move recs on from what the snapshot shows: proposed ones the change log
/// shows applied, or that passed their date unapplied; applied ones whose
/// check is due, settled by measuring the metric. Records and returns the
/// events.
pub fn sweep(
    cfg: &AppConfig,
    snap: &Snapshot,
    today: NaiveDate,
) -> std::io::Result<Vec<LedgerEvent>> {
    let last = week(snap).1;
    let mut out = Vec::new();
    for e in load(cfg) {
        let Some(by) = e.verify_by() else { continue };
        match e.state {
            RecState::Proposed => {
                if let Some((on, c)) = applied_on(snap, &e) {
                    out.push(event(
                        &e.id,
                        RecState::Applied,
                        on,
                        "moneyball",
                        Some(c.event.clone()),
                    ));
                } else if by < today {
                    out.push(event(&e.id, RecState::Expired, today, "moneyball", None));
                }
            }
            RecState::Applied => {
                let lag = match metric(&e.rec.prediction.metric) {
                    Some((true, _)) => CRM_LAG_DAYS,
                    _ => 0,
                };
                if by + Duration::days(lag) <= last {
                    let (state, note) = settle(snap, cfg, &e, by);
                    out.push(event(&e.id, state, today, "moneyball", Some(note)));
                }
            }
            _ => {}
        }
    }
    append(cfg, &out)?;
    Ok(out)
}

/// The first change the log shows on the rec's entity (or under it) of
/// the lever's kind, since it was proposed.
fn applied_on<'a>(snap: &'a Snapshot, e: &Entry) -> Option<(NaiveDate, &'a ChangeRow)> {
    let rec = &e.rec;
    let rows = snap.changes.as_ref()?.rows.iter();
    rows.filter(|c| kind_of(&rec.lever, &c.event_type) && touches(snap, rec, &c.object_id))
        .filter_map(|c| Some((local_time(&c.time)?, c)))
        .filter(|(t, _)| match e.proposed_at {
            Some(at) => *t >= at,
            None => t.date_naive() >= e.proposed,
        })
        .min_by_key(|(t, _)| *t)
        .map(|(t, c)| (t.date_naive(), c))
}

fn kind_of(lever: &str, event_type: &str) -> bool {
    match lever {
        "budget" => event_type.contains("budget"),
        "targeting" => event_type.contains("target_spec"),
        "creative" => event_type == "create_ad" || event_type.contains("creative"),
        "structure" => {
            event_type.contains("run_status")
                || matches!(
                    event_type,
                    "create_ad" | "create_ad_set" | "create_campaign_group"
                )
        }
        // Forms and the CRM change outside Meta's ad log: marked by hand.
        _ => false,
    }
}

/// The changed object is the entity, or sits under it.
fn touches(snap: &Snapshot, rec: &Rec, object: &str) -> bool {
    !object.is_empty()
        && (object == rec.entity_id
            || snap.ads_daily.iter().any(|r| match rec.level.as_str() {
                "campaign" => {
                    r.campaign_id == rec.entity_id && (r.adset_id == object || r.ad_id == object)
                }
                "adset" => r.adset_id == rec.entity_id && r.ad_id == object,
                "product" => {
                    r._product == rec.product
                        && [
                            r.campaign_id.as_str(),
                            r.adset_id.as_str(),
                            r.ad_id.as_str(),
                        ]
                        .contains(&object)
                }
                _ => false,
            }))
}

/// The verdict over the full days from the change to the check date.
fn settle(snap: &Snapshot, cfg: &AppConfig, e: &Entry, by: NaiveDate) -> (RecState, String) {
    let p = &e.rec.prediction;
    let from = e.applied.unwrap_or(e.proposed) + Duration::days(1);
    let first = snap
        .ads_daily
        .iter()
        .map(|r| r.date_start.as_str())
        .min()
        .and_then(day);
    if from > by {
        return (
            RecState::Inconclusive,
            "applied too late to measure by its date".into(),
        );
    }
    if first.is_none_or(|f| f > from) {
        return (
            RecState::Inconclusive,
            "the snapshot no longer reaches back to the change".into(),
        );
    }
    let span = format!("{} - {}", from.format("%d %b"), by.format("%d %b"));
    let Some(v) = measure(snap, cfg, &e.rec, &p.metric, (from, by)) else {
        return (
            RecState::Inconclusive,
            format!("no {} to measure over {}", p.metric, span),
        );
    };
    let hit = match p.direction.as_str() {
        "up" => v >= p.target,
        _ => v <= p.target,
    };
    let note = format!("{} {} over {}", p.metric, super::print::number(v), span);
    (
        if hit {
            RecState::Confirmed
        } else {
            RecState::Refuted
        },
        note,
    )
}
