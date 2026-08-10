//! The per-product comparison table (one dense row per creative).
//! Split from html.rs (size cap); pure over the report aggregate.

use std::fmt::Write as _;

use super::html::{commas, esc, slug};
use crate::schema::*;

/// The dense one-row-per-creative compare (python comparison_table):
/// rank + name (anchors to its card), live-since, status, delivery,
/// CPL, M->L, and the CRM funnel counts. The Diff breakdown column
/// needs lead segmentation - a documented v1 exclusion.
pub(super) fn comparison_table(p: &ProductSection, report_date: &str) -> String {
    let has_seg = p.creatives.iter().any(|c| c.segmentation.is_some());
    let seg_id = format!("segx-{}", slug(&p.product));
    let mut out = format!(
        r##"<div class="ttable"><input type="checkbox" id="{}" class="segx"><table><thead><tr><th>creative (ranked)</th><th>live since</th><th>status</th><th>spend</th><th>impr</th><th>ctr</th><th>m-leads</th><th>cpl</th><th>m&gt;l</th><th>l-leads</th><th title="Meta leads that never became CRM leads. Expand for the split.">diff{}</th><th class="seg" title="contact already in the CRM under another campaign or as an organic lead - a returning person">re-inq</th><th class="seg" title="same contact submitted again in the window or campaign">dup</th><th class="seg" title="invalid phone (CRM rejects) or a genuine sync gap to recover">other</th><th>qual</th><th>visits</th><th>book</th></tr></thead><tbody>"##,
        seg_id,
        if has_seg {
            format!(r##" <label for="{}" class="segl">+</label>"##, seg_id)
        } else {
            String::new()
        }
    );
    for (i, c) in p.creatives.iter().enumerate() {
        let f = |n: usize| c.funnel[n].count;
        let cpl = if c.delivery.m_leads > 0 {
            format!(
                "\u{20B9}{}",
                commas((c.delivery.spend / c.delivery.m_leads as f64).round() as u64)
            )
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
        let dim = |v: u64| if v == 0 { r#" class="dim""# } else { "" };
        let _ = write!(
            out,
            r##"<tr><td class="nm">{:02} &middot; <a href="#c-{}-{}">{}</a></td><td>{}</td><td><span class="stx {}">{}</span></td><td>{}{}</td><td>{}</td><td>{}</td><td{}>{}</td><td>{}</td><td>{}</td><td{}>{}</td>{}{}<td{}>{}</td><td{}>{}</td><td{}>{}</td></tr>"##,
            i + 1,
            slug(&p.product),
            i + 1,
            esc(&c.display_name),
            live_since(c.created.as_deref(), report_date),
            st_class,
            esc(&c.status.label),
            "\u{20B9}",
            commas(c.delivery.spend.round() as u64),
            commas(c.delivery.impressions),
            ctr,
            dim(f(2)),
            f(2),
            cpl,
            m_to_l,
            dim(f(3)),
            f(3),
            diff_cell(c, f(2), f(3)),
            seg_cells(c),
            dim(f(4)),
            f(4),
            dim(f(5)),
            f(5),
            dim(f(6)),
            f(6),
        );
    }
    out.push_str("</tbody></table></div>");
    out
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

/// Diff: with segmentation = window Meta submissions that did NOT
/// become CRM leads (total - captured, the sum the expander splits);
/// without it, the M-L fallback (negative possible when CRM delivery
/// lands in-window but the Meta count fell the day before).
fn diff_cell(c: &CreativeCard, m: u64, l: u64) -> String {
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

/// The hidden-until-expanded split: re-inquiry, duplicate, and
/// other = invalid + uncaptured (the actionable drops).
fn seg_cells(c: &CreativeCard) -> String {
    let Some(s) = &c.segmentation else {
        return r#"<td class="seg dim">-</td><td class="seg dim">-</td><td class="seg dim">-</td>"#
            .into();
    };
    let cell = |v: u64| {
        if v == 0 {
            r#"<td class="seg dim">0</td>"#.to_string()
        } else {
            format!(r#"<td class="seg">{}</td>"#, v)
        }
    };
    format!(
        "{}{}{}",
        cell(s.reinquiry),
        cell(s.duplicate),
        cell(s.invalid + s.uncaptured)
    )
}
