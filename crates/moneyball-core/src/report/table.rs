//! The per-product comparison table: one dense row per creative,
//! client-facing labels (2026-08-10 spec), the gap expander
//! (already-in-CRM / repeat form-fill / invalid / missing), and the
//! dead-tail collapse (zero-lead micro-spend creatives fold into one
//! honest line instead of six rows of dashes).

use std::fmt::Write as _;

use super::html::{commas, esc, is_dead_tail, rupees, slug};
use crate::schema::*;

pub(super) fn comparison_table(p: &ProductSection, report_date: &str) -> String {
    let has_seg = p.creatives.iter().any(|c| c.segmentation.is_some());
    let seg_id = format!("segx-{}", slug(&p.product));
    let tail_id = format!("tailx-{}", slug(&p.product));
    let mut out = format!(
        r##"<div class="ttable"><input type="checkbox" id="{}" class="segx"><input type="checkbox" id="{}" class="tailx"><table><thead><tr><th>creative (ranked)</th><th>live since</th><th>status</th><th>spend</th><th>impressions</th><th>ctr</th><th title="People who submitted the instant form on Facebook/Instagram.">meta leads</th><th title="Spend divided by Meta leads.">cost/lead</th><th title="Share of Meta leads that arrived in your CRM.">reached crm</th><th title="Leads that arrived in your CRM.">crm leads</th><th title="Meta leads that did not become new CRM leads. Expand for why.">gap{}</th><th class="seg" title="This person was in your CRM before this ad - counted once, not lost.">already in crm</th><th class="seg" title="Same person submitted the form more than once.">repeat</th><th class="seg" title="Phone number your CRM rejected.">invalid</th><th class="seg" title="Submitted the form but never arrived in the CRM - recoverable.">missing</th><th title="Marked genuine by your sales team in the CRM.">qualified</th><th>visits</th><th>bookings</th></tr></thead><tbody>"##,
        seg_id,
        tail_id,
        if has_seg {
            format!(r##" <label for="{}" class="segl">+</label>"##, seg_id)
        } else {
            String::new()
        }
    );
    let mut tail: Vec<usize> = Vec::new();
    for (i, c) in p.creatives.iter().enumerate() {
        if is_dead_tail(c, p.kpis.spend) {
            tail.push(i);
        }
    }
    let tail_spend: f64 = tail.iter().map(|&i| p.creatives[i].delivery.spend).sum();
    for (i, c) in p.creatives.iter().enumerate() {
        let is_tail = tail.contains(&i);
        let f = |n: usize| c.funnel[n].count;
        let cpl = if c.delivery.m_leads > 0 {
            rupees(c.delivery.spend / c.delivery.m_leads as f64)
        } else {
            "-".into()
        };
        let ctr = if c.delivery.impressions > 0 {
            format!(
                "{:.1}%",
                c.delivery.clicks as f64 / c.delivery.impressions as f64 * 100.0
            )
        } else {
            "-".into()
        };
        let m_to_l = if f(2) > 0 {
            format!("{:.0}%", f(3) as f64 / f(2) as f64 * 100.0)
        } else {
            "-".into()
        };
        let st_class = match c.status.code {
            StatusCode::Live => "st-live",
            StatusCode::Learn => "st-learn",
            StatusCode::Stop => "st-stop",
        };
        let dim = |v: u64| {
            if v == 0 {
                r#" class="dim""#
            } else {
                ""
            }
        };
        let _ = write!(
            out,
            r##"<tr{}><td class="nm">{:02} &middot; <a href="#c-{}-{}">{}</a></td><td>{}</td><td><span class="stx {}">{}</span></td><td>{}</td><td>{}</td><td>{}</td><td{}>{}</td><td>{}</td><td>{}</td><td{}>{}</td>{}{}<td{}>{}</td><td{}>{}</td><td{}>{}</td></tr>"##,
            if is_tail { r#" class="tail""# } else { "" },
            i + 1,
            slug(&p.product),
            i + 1,
            esc(&c.display_name),
            live_since(c.created.as_deref(), report_date),
            st_class,
            esc(&c.status.label),
            rupees(c.delivery.spend),
            commas(c.delivery.impressions),
            ctr,
            dim(f(2)),
            f(2),
            cpl,
            m_to_l,
            dim(f(3)),
            f(3),
            gap_cell(c, f(2), f(3)),
            seg_cells(c),
            dim(f(4)),
            f(4),
            dim(f(5)),
            f(5),
            dim(f(6)),
            f(6),
        );
    }
    if !tail.is_empty() {
        let _ = write!(
            out,
            r##"<tr><td class="nm" colspan="18"><label for="{}" class="taill">+ {} more creative(s) in early delivery &middot; {} total &middot; no leads yet</label></td></tr>"##,
            tail_id,
            tail.len(),
            rupees(tail_spend)
        );
    }
    out.push_str("</tbody></table></div>");
    out
}

/// Gap: with segmentation = Meta submissions that did NOT become new
/// CRM leads (total - captured); else the M-L fallback.
fn gap_cell(c: &CreativeCard, m: u64, l: u64) -> String {
    let d = match &c.segmentation {
        Some(s) => (s.total - s.captured) as i64,
        None => m as i64 - l as i64,
    };
    if d == 0 {
        r#"<td class="dim">0</td>"#.into()
    } else {
        format!("<td>{:+}</td>", d)
    }
}

/// The expanded split: already-in-CRM / repeat / invalid / missing.
/// Missing is the report's red - recoverable people.
fn seg_cells(c: &CreativeCard) -> String {
    let Some(s) = &c.segmentation else {
        return r#"<td class="seg dim">-</td><td class="seg dim">-</td><td class="seg dim">-</td><td class="seg dim">-</td>"#
            .into();
    };
    let cell = |v: u64, red: bool| {
        if v == 0 {
            r#"<td class="seg dim">0</td>"#.to_string()
        } else if red {
            format!(r#"<td class="seg red">{}</td>"#, v)
        } else {
            format!(r#"<td class="seg">{}</td>"#, v)
        }
    };
    format!(
        "{}{}{}{}",
        cell(s.reinquiry, false),
        cell(s.duplicate, false),
        cell(s.invalid, false),
        cell(s.uncaptured, true)
    )
}

/// "17 Jul + age in days" from the creative's earliest created date.
fn live_since(created: Option<&str>, report_date: &str) -> String {
    let Some(c) = created else {
        return "-".into();
    };
    let parse = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    match (parse(c), parse(report_date)) {
        (Some(cd), Some(rd)) => {
            let days = (rd - cd).num_days().max(0);
            format!("{} &middot; {}d", cd.format("%d %b"), days)
        }
        _ => esc(c),
    }
}
