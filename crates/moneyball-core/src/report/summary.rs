//! The report as plain text: the CLI's printout and the TUI /report cell,
//! and the dry run for a bot renderer (it needs nothing beyond
//! report.json). Split from report/mod.rs (size cap).

use crate::schema::CreativeReport;

/// Plain-text rendering of the aggregate - the CLI output and the dry
/// run for a future bot renderer (contract goal: a bot needs nothing
/// beyond report.json).
pub fn text_summary(r: &CreativeReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let f = &r.portfolio.funnel;
    let _ = writeln!(
        out,
        "CREATIVE REPORT  {}  (window {}..{})",
        r.report_date, r.window.since, r.window.until
    );
    // Without CRM data every count past Meta leads is unknown, never 0.
    let crm = |n: u64| {
        if r.source.crm_present {
            n.to_string()
        } else {
            "-".into()
        }
    };
    let _ = writeln!(
        out,
        "portfolio: spend Rs.{:.0}  impr {}  clicks {}  M {}  L {}  Q {}  V {}  B {}{}",
        r.portfolio.spend,
        r.portfolio.impressions,
        r.portfolio.clicks,
        f.m_leads,
        crm(f.l_leads),
        crm(f.qualified),
        crm(f.visit),
        crm(f.booking),
        if r.portfolio.unattributed_l_leads > 0 {
            format!("  (+{} unattributed L)", r.portfolio.unattributed_l_leads)
        } else {
            String::new()
        }
    );
    for p in &r.products {
        let _ = writeln!(
            out,
            "\n{} - {}",
            p.product,
            crate::text::count(p.creatives.len() as u64, "creative", "creatives")
        );
        for c in p.creatives.iter().take(3) {
            let fun: Vec<String> = c
                .funnel
                .iter()
                .skip(2)
                .enumerate()
                .map(|(i, s)| {
                    if i == 0 {
                        s.count.to_string()
                    } else {
                        crm(s.count)
                    }
                })
                .collect();
            let _ = writeln!(
                out,
                "  [{}] {}  spend Rs.{:.0}  M/L/Q/V/B {}  {}",
                c.status.label,
                c.display_name,
                c.delivery.spend,
                fun.join("/"),
                if c.image.is_some() { "" } else { "(no image)" }
            );
        }
        if p.creatives.len() > 3 {
            let _ = writeln!(out, "  ... +{} more", p.creatives.len() - 3);
        }
    }
    out
}
