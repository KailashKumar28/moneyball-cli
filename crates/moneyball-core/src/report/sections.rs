//! Report page sections above/around the product blocks: exec brief,
//! missing-leads banner, portfolio scorecard (+deltas), reconciliation
//! line, products-at-a-glance, and the per-product targeting block.
//! Pure string builders over the aggregate (client + UI specs
//! 2026-08-10).

use std::fmt::Write as _;

use super::html::{commas, esc, rupees, slug};
use crate::schema::*;

/// "Yesterday in brief" panel; empty string when no lines (never an
/// empty panel).
pub(super) fn exec_brief(r: &CreativeReport) -> String {
    if r.exec_brief.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        r##"<div class="pblock" id="brief"><section><div class="seckick">Yesterday in brief</div><div class="insight exec"><ul>"##,
    );
    for l in &r.exec_brief {
        let (class, tag, tagc) = match l.tone.as_str() {
            "win" => ("win", "Win", "tagw"),
            "watch" => ("con", "Watch", "tagc"),
            _ => ("", "Note", "tagi"),
        };
        let _ = write!(
            out,
            r#"<li class="{}"><span class="{}">{}</span>{}</li>"#,
            class,
            tagc,
            tag,
            esc(&l.text)
        );
    }
    out.push_str("</ul></div></section></div>");
    out
}

/// The report's only red element: missing (uncaptured) leads.
pub(super) fn missing_banner(r: &CreativeReport) -> String {
    let mut per: Vec<String> = Vec::new();
    let mut total = 0u64;
    for p in &r.products {
        let mut n = 0u64;
        let mut names: Vec<&str> = Vec::new();
        for c in &p.creatives {
            if let Some(s) = &c.segmentation {
                if s.uncaptured > 0 {
                    n += s.uncaptured;
                    names.push(&c.display_name);
                }
            }
        }
        if n > 0 {
            per.push(format!("{}: {} ({})", p.product, n, names.join(", ")));
            total += n;
        }
    }
    if total == 0 {
        return String::new(); // silence is the reward
    }
    format!(
        r#"<div class="missing"><b>{} lead(s) are missing from your CRM</b><p>They submitted your ad form on {} but never arrived in the CRM. They are paid for and still warm - a same-day call recovers them.</p><div class="mwhere">{}</div></div>"#,
        total,
        esc(&r.window.until),
        esc(&per.join(" &middot; "))
    )
}

fn delta_cell(cur: f64, prior: Option<f64>, money: bool, up_is_good: bool) -> String {
    let Some(p) = prior else {
        return String::new();
    };
    let d = cur - p;
    if d == 0.0 {
        return r#"<div class="okd">unchanged</div>"#.into();
    }
    let good = (d > 0.0) == up_is_good;
    let class = if good { "up" } else { "down" };
    let val = if money {
        format!("Rs {}", commas(d.abs().round() as u64))
    } else {
        commas(d.abs().round() as u64)
    };
    format!(
        r#"<div class="okd {}">{}{} vs last</div>"#,
        class,
        if d > 0.0 { "+" } else { "-" },
        val
    )
}

/// 5 primary tiles with deltas + the muted secondary row.
pub(super) fn scorecard(r: &CreativeReport, prior: Option<&CreativeReport>) -> (String, String) {
    let p = &r.portfolio;
    let q = prior.map(|x| &x.portfolio);
    let f = &p.funnel;
    let tile = |v: String, label: &str, dot: &str, delta: String| {
        format!(
            r#"<div class="ok"><div class="okv tabnum">{}</div><div class="okl"><i class="kdot {}"></i>{}</div>{}</div>"#,
            v, dot, label, delta
        )
    };
    let mut top = String::new();
    top += &tile(
        rupees(p.spend),
        "Spend",
        "m",
        delta_cell(p.spend, q.map(|x| x.spend), true, false),
    );
    top += &tile(
        f.m_leads.to_string(),
        "Meta leads",
        "m",
        delta_cell(
            f.m_leads as f64,
            q.map(|x| x.funnel.m_leads as f64),
            false,
            true,
        ),
    );
    top += &tile(
        f.l_leads.to_string(),
        "CRM leads",
        "c",
        delta_cell(
            f.l_leads as f64,
            q.map(|x| x.funnel.l_leads as f64),
            false,
            true,
        ),
    );
    top += &tile(
        f.qualified.to_string(),
        "Qualified",
        "c",
        delta_cell(
            f.qualified as f64,
            q.map(|x| x.funnel.qualified as f64),
            false,
            true,
        ),
    );
    let cpq = p
        .cost_per_qualified
        .map(rupees)
        .unwrap_or_else(|| "-".into());
    top += &tile(
        cpq,
        "Cost / qualified",
        "c",
        delta_cell(
            p.cost_per_qualified.unwrap_or(0.0),
            q.and_then(|x| x.cost_per_qualified),
            true,
            false,
        ),
    );

    let ctr = if p.impressions > 0 {
        format!("{:.1}%", p.clicks as f64 / p.impressions as f64 * 100.0)
    } else {
        "-".into()
    };
    let cpl = if f.m_leads > 0 {
        rupees(p.spend / f.m_leads as f64)
    } else {
        "-".into()
    };
    let mut low = String::new();
    for (v, l, d) in [
        (commas(p.impressions), "Impressions", "m"),
        (ctr, "CTR", "m"),
        (cpl, "Cost / Meta lead", "m"),
        (visits_label(f.visit), "Visits", "c"),
        (visits_label(f.booking), "Bookings", "c"),
    ] {
        low += &tile(v, l, d, String::new());
    }
    (top, low)
}

fn visits_label(v: u64) -> String {
    if v == 0 {
        "none".into()
    } else {
        v.to_string()
    }
}

/// The reconciliation sentence - the buckets must sum or we say so.
pub(super) fn reconciliation(r: &CreativeReport) -> String {
    let f = &r.portfolio.funnel;
    let mut seg = Segmentation::default();
    let mut have_seg = false;
    for p in &r.products {
        for c in &p.creatives {
            if let Some(s) = &c.segmentation {
                have_seg = true;
                seg.reinquiry += s.reinquiry;
                seg.duplicate += s.duplicate;
                seg.invalid += s.invalid;
                seg.uncaptured += s.uncaptured;
            }
        }
    }
    if !have_seg {
        return String::new();
    }
    let gap = f.m_leads as i64 - f.l_leads as i64;
    let explained = (seg.reinquiry + seg.duplicate + seg.invalid + seg.uncaptured) as i64;
    let mut s = format!(
        "The trail: {} Meta leads -> {} arrived in CRM -> {} qualified.",
        f.m_leads, f.l_leads, f.qualified
    );
    if gap > 0 {
        let mut parts: Vec<String> = Vec::new();
        if seg.reinquiry > 0 {
            parts.push(format!("{} already in your CRM", seg.reinquiry));
        }
        if seg.duplicate > 0 {
            parts.push(format!("{} repeat form-fills", seg.duplicate));
        }
        if seg.invalid > 0 {
            parts.push(format!("{} invalid numbers", seg.invalid));
        }
        if seg.uncaptured > 0 {
            parts.push(format!("{} never arrived (see banner)", seg.uncaptured));
        }
        let _ = write!(s, " Of the gap of {}: {}.", gap, parts.join(", "));
        if explained != gap {
            let _ = write!(
                s,
                " {} lead(s) are still syncing and not yet classified.",
                (gap - explained).abs()
            );
        }
    }
    if r.portfolio.unattributed_l_leads > 0 {
        let _ = write!(
            s,
            " Separately, {} CRM lead(s) came from outside these ads (portals, walk-ins, organic).",
            r.portfolio.unattributed_l_leads
        );
    }
    format!(r#"<p class="recon">{}</p>"#, esc(&s))
}

/// Products at a glance - the 60-second layer.
pub(super) fn glance(r: &CreativeReport) -> String {
    if r.products.len() < 2 {
        return String::new();
    }
    let mut out = String::from(
        r#"<div class="ttable"><table><thead><tr><th>project</th><th>spend</th><th>crm leads</th><th>qualified</th><th>cost / qualified</th><th>missing</th></tr></thead><tbody>"#,
    );
    for p in &r.products {
        let k = &p.kpis;
        let miss: u64 = p
            .creatives
            .iter()
            .filter_map(|c| c.segmentation.as_ref())
            .map(|s| s.uncaptured)
            .sum();
        let cpq = k
            .cost_per_qualified
            .map(rupees)
            .unwrap_or_else(|| "-".into());
        let _ = write!(
            out,
            r##"<tr><td class="nm"><a href="#p-{}">{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td{}>{}</td></tr>"##,
            slug(&p.product),
            esc(&p.product),
            rupees(k.spend),
            k.funnel.l_leads,
            k.funnel.qualified,
            cpq,
            if miss > 0 {
                r#" class="red""#
            } else {
                r#" class="dim""#
            },
            miss
        );
    }
    out.push_str("</tbody></table></div>");
    out
}

/// The per-product targeting block: subhead + open ledger + insight.
pub(super) fn targeting_block(p: &ProductSection) -> String {
    if p.targetings.is_empty() {
        return String::new();
    }
    let mut out = format!(
        r#"<div class="subhead" id="t-{}"><span>Targeting</span></div><div class="tgwrap"><table class="tgtable"><thead><tr><th>targeting (by spend)</th><th>spend</th><th>cost / meta lead</th><th>meta leads</th><th>crm leads</th><th>qualified</th><th>cost / qualified</th><th>7d: spend &middot; q &middot; cost/q</th><th>verdict</th></tr></thead><tbody>"#,
        slug(&p.product)
    );
    for t in &p.targetings {
        let w = &t.window;
        let c = &t.window_crm;
        let s7 = &t.window_7d;
        let cpl = if w.m_leads > 0 {
            rupees(w.spend / w.m_leads as f64)
        } else {
            "-".into()
        };
        let cpq = if c.qualified > 0 {
            rupees(w.spend / c.qualified as f64)
        } else {
            "-".into()
        };
        let s7q = if s7.qualified > 0 {
            format!(
                "{} &middot; {} &middot; {}",
                rupees(s7.spend),
                s7.qualified,
                rupees(s7.spend / s7.qualified as f64)
            )
        } else {
            format!("{} &middot; 0 &middot; -", rupees(s7.spend))
        };
        let mut facts = String::new();
        if let Some(sp) = &t.specs {
            for f in [sp.age.as_deref(), sp.geo.as_deref(), sp.genders.as_deref()]
                .into_iter()
                .flatten()
            {
                if f != "-" && f != "all" {
                    let _ = write!(facts, r#"<span class="fact">{}</span>"#, esc(f));
                }
            }
            if let Some(l) = sp.learning.as_deref() {
                if l != "SUCCESS" {
                    let _ = write!(facts, r#"<span class="fact">{}</span>"#, esc(l));
                }
            }
        }
        let chips: String = t
            .verdicts
            .iter()
            .map(|v| {
                format!(
                    r#"<span class="vchip {}" title="{}">{}</span> "#,
                    esc(&v.code),
                    esc(&v.detail),
                    esc(&v.label)
                )
            })
            .collect();
        let dim = |v: u64| if v == 0 { r#" class="dim""# } else { "" };
        let _ = write!(
            out,
            r#"<tr><td class="tg-nm">{}<span class="tg-arch">{}</span><span class="tg-facts">{}</span></td><td>{}</td><td>{}</td><td{}>{}</td><td{}>{}</td><td{}>{}</td><td>{}</td><td>{}</td><td class="tg-vd">{}</td></tr>"#,
            esc(&t.targeting),
            esc(&t.archetype),
            facts,
            rupees(w.spend),
            cpl,
            dim(w.m_leads),
            w.m_leads,
            dim(c.l_leads),
            c.l_leads,
            dim(c.qualified),
            c.qualified,
            cpq,
            s7q,
            chips.trim_end()
        );
    }
    out.push_str("</tbody></table></div>");
    if !p.cross_reads.is_empty() {
        out.push_str(r#"<div class="insight"><div class="ins-h">Cross-read</div><ul>"#);
        for line in &p.cross_reads {
            let _ = write!(out, "<li>{}</li>", esc(line));
        }
        out.push_str("</ul></div>");
    }
    out
}
