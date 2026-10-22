//! Submitting a recommendation: every check mb.py's `rec submit` made -
//! the evidence against the data, the prediction's sense, the guards.
//! Split from mod.rs (size cap).

use chrono::{DateTime, Duration, Local};

use super::*;
use crate::funnel::{self, By};

/// Check a recommendation against the data and the guards; record it and
/// return its id, or every reason it was refused (for the model to fix).
pub fn submit(
    cfg: &AppConfig,
    snap: &Snapshot,
    mut rec: Rec,
    now: DateTime<Local>,
) -> Result<String, Vec<String>> {
    let today = now.date_naive();
    let mut why = Vec::new();
    match crate::product::resolve_configured(cfg, &rec.product) {
        Ok(p) => rec.product = p,
        Err(e) => why.push(e),
    }
    if rec.level == "product" {
        rec.entity_id = rec.product.clone();
        rec.entity_name = rec.product.clone();
    } else if !["campaign", "adset", "ad"].contains(&rec.level.as_str()) {
        why.push("level must be campaign, adset, ad or product".into());
    } else {
        let by = By::parse(&rec.level).expect("checked above");
        let rows = funnel::compute_between(snap, cfg, &rec.product, week(snap), by);
        match rows.iter().find(|r| r.id == rec.entity_id) {
            Some(r) => rec.entity_name = r.name.clone(),
            None => why.push(format!(
                "{} {} did not deliver for {} in the last 7 days - use an id from the funnel table",
                rec.level, rec.entity_id, rec.product
            )),
        }
    }
    let Some(cooldown) = LEVERS.iter().find(|l| l.0 == rec.lever).map(|l| l.1) else {
        why.push(format!(
            "unknown lever \"{}\" (budget, creative, targeting, structure, form, crm)",
            rec.lever
        ));
        return Err(why);
    };
    if !why.is_empty() {
        return Err(why);
    }
    let crm = !crate::crm::is_empty(&snap.crm);

    if rec.evidence.is_empty() {
        why.push("give at least one evidence number from the last 7 days".into());
    }
    for e in &rec.evidence {
        let Some((needs_crm, summed)) = metric(&e.metric) else {
            why.push(format!("unknown evidence metric \"{}\"", e.metric));
            continue;
        };
        if needs_crm && !crm {
            why.push(format!(
                "{} needs CRM data, and this snapshot has none",
                e.metric
            ));
            continue;
        }
        let actual = measure(snap, cfg, &rec, &e.metric, week(snap));
        let ok = actual.is_some_and(|a| match summed && e.metric != "spend" {
            true => a.round() == e.value.round(),
            false => (a - e.value).abs() <= e.value.abs() * EVIDENCE_TOLERANCE,
        });
        if !ok {
            why.push(format!(
                "evidence {}={} does not match the last 7 days ({})",
                e.metric,
                e.value,
                actual.map_or("no value".into(), |a| format!("{:.1}", a))
            ));
        }
    }

    let p = &rec.prediction;
    match metric(&p.metric) {
        None => why.push(format!("unknown prediction metric \"{}\"", p.metric)),
        Some((true, _)) if !crm => why.push(format!(
            "{} needs CRM data, and this snapshot has none - predict spend, meta_leads or cpl",
            p.metric
        )),
        Some(_) => {
            let now = measure(snap, cfg, &rec, &p.metric, week(snap));
            match (p.direction.as_str(), now) {
                ("up", Some(n)) if p.target <= n => {
                    why.push(format!("target {} is not above today's {:.1}", p.target, n))
                }
                ("down", Some(n)) if p.target >= n => {
                    why.push(format!("target {} is not below today's {:.1}", p.target, n))
                }
                ("up" | "down", _) => {}
                _ => why.push("direction must be up or down".into()),
            }
        }
    }
    match day(&p.verify_by) {
        Some(d) if d > today && d <= today + Duration::days(MAX_HORIZON_DAYS) => {}
        _ => why.push(format!(
            "verify_by must be a YYYY-MM-DD after today and at most {} days out",
            MAX_HORIZON_DAYS
        )),
    }

    if rec.lever == "budget" {
        match &rec.budget {
            Some(b) if b.from > 0.0 && b.to > 0.0 => {
                if (b.to - b.from).abs() > b.from * BUDGET_STEP && !b.relearn {
                    why.push(
                        "a budget move over 20% restarts learning - step it, or set relearn".into(),
                    );
                }
            }
            _ => why.push("a budget rec needs budget.from and budget.to (Rs a day)".into()),
        }
    }

    let ledger = load(cfg);
    let same = |e: &&Entry| e.rec.entity_id == rec.entity_id;
    if ledger
        .iter()
        .filter(same)
        .any(|e| e.open() && e.rec.lever == rec.lever)
    {
        why.push(format!(
            "an open {} rec already targets this {}",
            rec.lever, rec.level
        ));
    }
    for e in ledger.iter().filter(same) {
        let lever_days = LEVERS
            .iter()
            .find(|l| l.0 == e.rec.lever)
            .map_or(cooldown, |l| l.1);
        if let Some(until) = e
            .applied
            .map(|a| a + Duration::days(lever_days))
            .filter(|u| *u >= today)
        {
            why.push(format!(
                "{} is cooling down after {} until {}",
                rec.level, e.id, until
            ));
        }
    }
    let todays = ledger.iter().filter(|e| e.proposed == today).count();
    if todays >= MAX_PER_DAY {
        why.push(format!(
            "{} recs already today - the limit is {}",
            todays, MAX_PER_DAY
        ));
    }
    if !why.is_empty() {
        return Err(why);
    }

    let id = format!("mb-{}-{}", today.format("%Y%m%d"), todays + 1);
    let mut e = event(&id, RecState::Proposed, today, "advisor", None);
    e.at = Some(now.to_rfc3339());
    e.rec = Some(rec);
    append(cfg, &[e]).map_err(|e| vec![format!("cannot write the ledger: {}", e)])?;
    Ok(id)
}
