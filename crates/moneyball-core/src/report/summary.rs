//! The report as plain text: the CLI's printout and the TUI /report cell,
//! and the dry run for a bot renderer (it needs nothing beyond
//! report.json). Split from report/mod.rs (size cap).

use crate::schema::CreativeReport;
use crate::text::commas;

/// Plain-text rendering of the aggregate - the CLI output and the dry
/// run for a future bot renderer (contract goal: a bot needs nothing
/// beyond report.json).
pub fn text_summary(r: &CreativeReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let f = &r.portfolio.funnel;
    let day = |d: &str| super::html::day(d, "%a %d %b %Y");
    let _ = writeln!(
        out,
        "CREATIVE REPORT  {}",
        match r.window.since == r.window.until {
            true => day(&r.window.until),
            false => format!("{} - {}", day(&r.window.since), day(&r.window.until)),
        }
    );
    let rs = |v: f64| format!("Rs.{}", commas(v.round().max(0.0) as u64));
    // Without CRM data every count past Meta leads is unknown: left out,
    // never shown as 0.
    let crm = |l: u64, q: u64, v: u64, b: u64| match r.source.crm_present {
        true => format!(
            "{d}CRM leads {}{d}qualified {}{d}visits {}{d}bookings {}",
            commas(l),
            commas(q),
            commas(v),
            commas(b),
            d = DOT
        ),
        false => String::new(),
    };
    let _ = writeln!(
        out,
        "portfolio: spend {}{d}impressions {}{d}clicks {}{d}Meta leads {}{}{}",
        rs(r.portfolio.spend),
        commas(r.portfolio.impressions),
        commas(r.portfolio.clicks),
        commas(f.m_leads),
        crm(f.l_leads, f.qualified, f.visit, f.booking),
        if r.portfolio.unattributed_l_leads > 0 {
            format!(
                "  (+{} from outside these ads)",
                crate::text::count(r.portfolio.unattributed_l_leads, "CRM lead", "CRM leads")
            )
        } else {
            String::new()
        },
        d = DOT
    );
    for p in &r.products {
        let _ = writeln!(
            out,
            "\n{} - {}",
            p.product,
            crate::text::count(p.creatives.len() as u64, "creative", "creatives")
        );
        for c in p.creatives.iter().take(3) {
            // Funnel steps: Impressions, Clicks, Meta leads, CRM leads,
            // qualified, visits, bookings.
            let n = |i: usize| c.funnel.get(i).map_or(0, |s| s.count);
            let _ = writeln!(
                out,
                "  [{}] {}  spend {}{d}Meta leads {}{}{}",
                c.status.label,
                c.display_name,
                rs(c.delivery.spend),
                commas(n(2)),
                crm(n(3), n(4), n(5), n(6)),
                if c.image.is_some() {
                    ""
                } else {
                    "  (no image)"
                },
                d = DOT
            );
        }
        if p.creatives.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", p.creatives.len() - 3);
        }
    }
    out
}

const DOT: &str = "  \u{00B7}  ";
