//! Diagnose presentation: four short sections, the same text for the
//! shell, the TUI cell and the advisor's tool. Split from the compute
//! module.

use serde_json::Value;

use super::{Diagnosis, Fatigue, StageCall, RECENT_DAYS};
use crate::text::{commas, table, truncate_marked};

/// Ads listed by name before the rest are summed up.
const ADS_SHOWN: usize = 10;
const CHANGES_SHOWN: usize = 12;

pub fn text(d: &Diagnosis) -> String {
    format!(
        "DIAGNOSE {} - snapshot {}\n{}{}{}{}",
        d.product,
        d.snapshot_date,
        stages(d),
        ads(d),
        trend(d),
        changes(d)
    )
}

fn name(s: &str, chars: usize) -> String {
    match s.chars().count() > chars {
        true => format!("{}...", s.chars().take(chars - 3).collect::<String>()),
        false => s.to_string(),
    }
}

fn money(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| commas(v.round() as u64))
}

fn stages(d: &Diagnosis) -> String {
    let days = d.recent.days + d.before.days;
    let mut out = format!(
        "\n1. Where its leads die (CRM leads of the last {} days by campaign, as far as each got)\n",
        days
    );
    let Some(rows) = &d.stages else {
        out.push_str("needs CRM data - see the CRM note.\n");
        return out;
    };
    if rows.is_empty() {
        out.push_str("no CRM leads from this product's ads in this snapshot.\n");
        return out;
    }
    let pct = |n: u64, of: u64| format!("{:.0}%", n as f64 / of.max(1) as f64 * 100.0);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                name(&r.campaign, 32),
                r.leads.to_string(),
                pct(r.lost, r.leads),
                pct(r.unreachable, r.leads),
                r.qualified.to_string(),
                r.visited.to_string(),
                money((r.qualified > 0).then(|| r.spend as f64 / r.qualified as f64)),
                match r.call {
                    StageCall::Form => "form",
                    StageCall::Data => "data",
                    StageCall::Sales => "sales",
                    StageCall::Healthy => "ok",
                    StageCall::Stuck => "stuck",
                }
                .into(),
            ]
        })
        .collect();
    out.push_str(&table(
        &[
            "campaign",
            "leads",
            "lost",
            "unreachable",
            "qualified",
            "visited",
            "rs_per_q",
            "call",
        ],
        &cells,
    ));
    out.push_str(
        "form = over half lost before contact (the form, or the audience's intent); \
         data = over 40% unreachable (phone numbers); sales = reached, but under 1 in 5 \
         visit (speed to lead); stuck = none reached yet\n",
    );
    out
}

fn ads(d: &Diagnosis) -> String {
    let mut out = format!("\n2. Ad fatigue (the last {} days, by ad)\n", RECENT_DAYS);
    if d.ads.is_empty() {
        out.push_str("no ad spent in these days.\n");
        return out;
    }
    let cells: Vec<Vec<String>> = d
        .ads
        .iter()
        .take(ADS_SHOWN)
        .map(|a| {
            vec![
                name(&a.name, 32),
                name(&a.adset, 20),
                commas(a.spend),
                a.m.to_string(),
                money((a.m > 0).then(|| a.spend as f64 / a.m as f64)),
                a.ctr.map_or("-".into(), |c| format!("{:.2}", c)),
                a.ctr_before.map_or("-".into(), |c| format!("{:.2}", c)),
                a.freq.map_or("-".into(), |f| format!("{:.1}", f)),
                a.age_days.map_or("-".into(), |n| n.to_string()),
                match a.call {
                    Fatigue::Fatigued => "fatigued",
                    Fatigue::Watch => "watch",
                    Fatigue::Ok => "ok",
                    Fatigue::Unknown => "-",
                }
                .into(),
            ]
        })
        .collect();
    out.push_str(&table(
        &[
            "ad", "ad set", "spend", "meta", "cpl", "ctr%", "was%", "freq", "age_d", "call",
        ],
        &cells,
    ));
    let rest = &d.ads[d.ads.len().min(ADS_SHOWN)..];
    if !rest.is_empty() {
        out.push_str(&format!(
            "+ {} more ads, Rs.{} between them\n",
            rest.len(),
            commas(rest.iter().map(|a| a.spend).sum())
        ));
    }
    out.push_str(
        "ctr% = link clicks per impression, was% = the ad's own before these days; freq = \
         times each person saw it in these 7 days (Meta's count). fatigued = clicked 30%+ \
         less than before while seen 2+ times; watch = clicked 30%+ less, or seen 3+ times; \
         age_d = days since it was made\n",
    );
    out
}

fn trend(d: &Diagnosis) -> String {
    let (now, then) = (&d.recent, &d.before);
    if then.days == 0 {
        return format!(
            "\n3. Trend\nno days before the last {} in this snapshot to compare with.\n",
            RECENT_DAYS
        );
    }
    let mut out = format!(
        "\n3. The last {} days against the {} before\n",
        now.days, then.days
    );
    let change = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) if b > 0.0 => format!("{:+.0}%", (a / b - 1.0) * 100.0),
        _ => "-".into(),
    };
    let mut rows: Vec<(&str, Option<f64>, Option<f64>, bool)> = vec![
        (
            "spend per day (Rs)",
            Some(now.per_day(now.spend)),
            Some(then.per_day(then.spend)),
            true,
        ),
        (
            "Meta leads per day",
            Some(now.per_day(now.m as f64)),
            Some(then.per_day(then.m as f64)),
            false,
        ),
        ("cost per Meta lead (Rs)", now.cpl(), then.cpl(), true),
    ];
    if let (Some(a), Some(b)) = (now.q, then.q) {
        rows.push((
            "qualified per day",
            Some(now.per_day(a as f64)),
            Some(then.per_day(b as f64)),
            false,
        ));
        rows.push((
            "cost per qualified (Rs)",
            now.rs_per_q(),
            then.rs_per_q(),
            true,
        ));
    }
    let shown = |v: Option<f64>, whole: bool| match (v, whole) {
        (Some(v), true) => commas(v.round() as u64),
        (Some(v), false) => format!("{:.1}", v),
        (None, _) => "-".into(),
    };
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|(label, a, b, whole)| {
            vec![
                label.to_string(),
                shown(*a, *whole),
                shown(*b, *whole),
                change(*a, *b),
            ]
        })
        .collect();
    out.push_str(&table(&["", "last", "before", "change"], &cells));
    // The verdict rides on what a lead that matters costs - but the last
    // 3 days' leads are still being qualified, and that count only rises:
    // trust a drop in cost per qualified, never a rise.
    let (what, a, b) = match (now.rs_per_q(), then.rs_per_q(), now.cpl(), then.cpl()) {
        (Some(a), Some(b), ..) if b > 0.0 && a <= b => ("cost per qualified", a, b),
        (.., Some(a), Some(b)) if b > 0.0 => ("cost per Meta lead", a, b),
        _ => return out,
    };
    let ratio = a / b;
    let pct = ((ratio - 1.0) * 100.0).abs().round();
    out.push_str(&if ratio >= 1.5 {
        format!(
            "{} is up {:.1}x on the days before: something changed - see 4.\n",
            what, ratio
        )
    } else if ratio >= 1.2 {
        format!(
            "{} is up {}% on the days before: worth a look at 4.\n",
            what, pct
        )
    } else if ratio <= 0.8 {
        format!(
            "{} is down {}% on the days before: note what worked.\n",
            what, pct
        )
    } else {
        format!("{} held steady (within 20%).\n", what)
    });
    out
}

fn changes(d: &Diagnosis) -> String {
    let mut out = format!(
        "\n4. What changed in its ad account (the last {} days)\n",
        RECENT_DAYS
    );
    let Some(rows) = &d.changes else {
        out.push_str("this snapshot has no change log - the next /refresh pulls it.\n");
        return out;
    };
    if rows.is_empty() {
        out.push_str("nothing was changed.\n");
        return out;
    }
    // One line per object and minute: creating a post's campaign, ad set
    // and ad is one change to a reader.
    let mut lines: Vec<(String, String, Vec<String>)> = Vec::new();
    for r in rows {
        let when = super::local_time(&r.time)
            .map_or(r.time.clone(), |t| t.format("%a %d %b %H:%M").to_string());
        let old = r.old.as_ref().and_then(|v| logged_value(v, "old_value"));
        let new = r.new.as_ref().and_then(|v| logged_value(v, "new_value"));
        let what = match (old, new) {
            (Some(o), Some(n)) => format!("{} ({} -> {})", r.event, o, n),
            (None, Some(n)) => format!("{} (now {})", r.event, n),
            _ => r.event.clone(),
        };
        match lines.last_mut() {
            Some((w, o, whats)) if *w == when && *o == r.object_name => whats.push(what),
            _ => lines.push((when, r.object_name.clone(), vec![what])),
        }
    }
    for (when, object, whats) in lines.iter().take(CHANGES_SHOWN) {
        // The same change logged again in that minute: once, counted.
        let mut said: Vec<(String, usize)> = Vec::new();
        for w in whats {
            match said.iter_mut().find(|(s, _)| s == w) {
                Some((_, n)) => *n += 1,
                None => said.push((w.clone(), 1)),
            }
        }
        let said: Vec<String> = said
            .into_iter()
            .map(|(w, n)| if n > 1 { format!("{} (x{})", w, n) } else { w })
            .collect();
        out.push_str(&format!(
            "{}  {}: {}\n",
            when,
            truncate_marked(object, 40, "..."),
            said.join("; ")
        ));
    }
    if lines.len() > CHANGES_SHOWN {
        out.push_str(&format!(
            "+ {} earlier changes\n",
            lines.len() - CHANGES_SHOWN
        ));
    }
    out
}
/// A logged value in words: a status as is, money in rupees (Meta logs
/// minor units), on/off; a list or spec (some arrive as JSON text) is
/// too long to say here.
fn logged_value(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::String(s) if s.starts_with(['{', '[']) => None,
        Value::String(s) => Some(truncate_marked(s, 40, "...")),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "on" } else { "off" }.into()),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("payment_amount") => {
            let major = (o.get(key)?.as_f64()? / 100.0).round() as u64;
            let per = match o.get("additional_value").and_then(Value::as_str) {
                Some("Per day") => "/day",
                _ => "",
            };
            Some(match o.get("currency").and_then(Value::as_str) {
                Some("INR") | None => format!("Rs.{}{}", commas(major), per),
                Some(c) => format!("{} {}{}", c, commas(major), per),
            })
        }
        _ => None,
    }
}

/// Headless runner: load the snapshot, diagnose, print.
pub fn run(
    cfg: &crate::config::AppConfig,
    product: &str,
    date: Option<&str>,
) -> crate::error::Result<()> {
    let snap = crate::snapshot::load(&cfg.snap_for(date)?)?;
    let product =
        crate::product::resolve_configured(cfg, product).map_err(crate::error::Error::Config)?;
    print!("{}", text(&super::compute(&snap, cfg, &product)));
    if let Some(note) = crate::crm::status::CrmHealth::of(cfg, &snap).note() {
        println!("!! {}", note);
    }
    Ok(())
}
