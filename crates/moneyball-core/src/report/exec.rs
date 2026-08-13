//! "Yesterday in brief" - 3-5 deterministic one-liners (client-report
//! spec 2026-08-10, generators G1-G6). No LLM anywhere: every line is
//! a formula with minimum-data guards, so a quiet day reads quiet
//! instead of inflated.

use crate::schema::*;

/// Priority-ordered generators; each emits at most one line; cap 5.
pub(super) fn brief(r: &CreativeReport, prior: Option<&CreativeReport>) -> Vec<ExecLine> {
    let mut out: Vec<ExecLine> = Vec::new();
    let p = &r.portfolio;
    let quiet = p.funnel.m_leads < 3;

    // G1 - missing (uncaptured) leads: always first when present.
    let uncap: u64 = each_card(r)
        .filter_map(|(c, _)| c.segmentation.as_ref())
        .map(|s| s.uncaptured)
        .sum();
    if uncap > 0 {
        out.push(line(
            "watch",
            format!(
                "{} lead(s) filled your ad form but never reached the CRM - recoverable, see the red banner below.",
                uncap
            ),
        ));
    }

    if !quiet {
        // G2 - qualified mover vs the prior comparable report.
        if let Some(prev) = prior {
            let mut best: Option<(i64, &ProductSection, u64)> = None;
            for ps in &r.products {
                if let Some(pp) = prev.products.iter().find(|x| x.product == ps.product) {
                    let dq = ps.kpis.funnel.qualified as i64 - pp.kpis.funnel.qualified as i64;
                    if best.map(|(b, _, _)| dq.abs() > b.abs()).unwrap_or(true) {
                        best = Some((dq, ps, pp.kpis.funnel.qualified));
                    }
                }
            }
            if let Some((dq, ps, prev_q)) = best.filter(|(dq, _, _)| dq.abs() >= 2) {
                let q = ps.kpis.funnel.qualified;
                out.push(if dq > 0 {
                    line(
                        "win",
                        format!(
                            "{} produced {} qualified leads, up {} on the last report.",
                            ps.product, q, dq
                        ),
                    )
                } else {
                    line(
                        "watch",
                        format!(
                            "{} fell to {} qualified leads ({} in the last report).",
                            ps.product, q, prev_q
                        ),
                    )
                });
            }
        }
        // G3 - best value creative vs portfolio Rs/qualified.
        if let (Some(port_cpq), true) = (p.cost_per_qualified, p.funnel.qualified >= 2) {
            let winner = each_card(r)
                .filter(|(c, _)| c.funnel[4].count >= 1 && c.delivery.spend >= 1000.0)
                .map(|(c, prod)| (c.delivery.spend / c.funnel[4].count as f64, c, prod))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((cpq, c, prod)) = winner.filter(|(cpq, ..)| *cpq <= 0.75 * port_cpq) {
                out.push(line("win", format!(
                    "Best value: \"{}\" ({}) - {} qualified at Rs {:.0} each vs Rs {:.0} portfolio average.",
                    c.display_name, prod, c.funnel[4].count, cpq, port_cpq
                )));
            }
        }
        // G4 - money leak: big spend, zero qualified.
        let leak = r
            .products
            .iter()
            .flat_map(|ps| {
                let pspend = ps.kpis.spend;
                ps.creatives.iter().map(move |c| (c, &ps.product, pspend))
            })
            .filter(|(c, _, pspend)| {
                c.delivery.spend >= 2000f64.max(0.15 * pspend) && c.funnel[4].count == 0
            })
            .max_by(|a, b| a.0.delivery.spend.total_cmp(&b.0.delivery.spend));
        if let Some((c, prod, _)) = leak.filter(|(c, ..)| c.delivery.spend >= 0.10 * p.spend) {
            out.push(line(
                "watch",
                format!(
                    "Watch: \"{}\" ({}) spent Rs {:.0} for {} CRM leads and 0 qualified.",
                    c.display_name, prod, c.delivery.spend, c.funnel[3].count
                ),
            ));
        }
        // G5 - cheapest Meta leads (filler under 3 lines).
        if out.len() < 3 && p.funnel.m_leads > 0 {
            let port_cpl = p.spend / p.funnel.m_leads as f64;
            let cheap = each_card(r)
                .filter(|(c, _)| c.funnel[2].count >= 3 && c.delivery.spend >= 1000.0)
                .map(|(c, _)| (c.delivery.spend / c.funnel[2].count as f64, c))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((cpl, c)) = cheap.filter(|(cpl, _)| *cpl <= 0.6 * port_cpl) {
                out.push(line(
                    "win",
                    format!(
                        "Cheapest leads: \"{}\" at Rs {:.0} per Meta lead ({} leads).",
                        c.display_name, cpl, c.funnel[2].count
                    ),
                ));
            }
        }
    }

    // G6 - guaranteed baseline.
    let base =
        if p.funnel.qualified == 0 {
            format!(
                "Rs {:.0} across {} projects -> {} CRM leads, none qualified yet.",
                p.spend,
                r.products.len(),
                p.funnel.l_leads
            )
        } else {
            format!(
            "Rs {:.0} across {} projects -> {} CRM leads, {} qualified (Rs {:.0} per qualified).",
            p.spend, r.products.len(), p.funnel.l_leads, p.funnel.qualified,
            p.cost_per_qualified.unwrap_or(0.0)
        )
        };
    out.push(line(
        "info",
        if quiet {
            format!("Quiet day: {}", base)
        } else {
            base
        },
    ));
    out.truncate(5);
    out
}

fn each_card(r: &CreativeReport) -> impl Iterator<Item = (&CreativeCard, &String)> {
    r.products
        .iter()
        .flat_map(|p| p.creatives.iter().map(move |c| (c, &p.product)))
}

fn line(tone: &str, text: String) -> ExecLine {
    ExecLine {
        tone: tone.into(),
        text,
    }
}

/// The prior comparable report: latest reports/<d>/creative-report*.json
/// with d < report_date and the same window length. Multi-day reports
/// carry a range suffix in the file name, so every json in the day's
/// dir is a candidate; the window-length check picks the comparable one.
pub(super) fn load_prior(
    reports_dir: &std::path::Path,
    report_date: &str,
    window_days: i64,
) -> Option<CreativeReport> {
    let mut dates: Vec<String> = std::fs::read_dir(reports_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|d| d.as_str() < report_date)
        .collect();
    dates.sort();
    for d in dates.iter().rev() {
        let Ok(entries) = std::fs::read_dir(reports_dir.join(d)) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("creative-report") || !name.ends_with(".json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(r) = serde_json::from_str::<CreativeReport>(&raw) else {
                continue;
            };
            let len = chrono::NaiveDate::parse_from_str(&r.window.until, "%Y-%m-%d")
                .and_then(|u| {
                    chrono::NaiveDate::parse_from_str(&r.window.since, "%Y-%m-%d")
                        .map(|s| (u - s).num_days() + 1)
                })
                .unwrap_or(-1);
            if len == window_days {
                return Some(r);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty-but-valid report for a given window; enough for the
    /// loader's window-length matching.
    fn report(since: &str, until: &str, report_date: &str) -> CreativeReport {
        let snap = crate::snapshot::Snapshot {
            path: std::path::PathBuf::from("/t"),
            date: report_date.into(),
            ads_daily: vec![],
            adsets: serde_json::json!({}),
            creatives: None,
            crm: serde_json::json!({}),
            leads: None,
            crm_contacts: None,
            regions: serde_json::json!([]),
            changes: serde_json::json!([]),
            campaigns: serde_json::json!([]),
        };
        let days = (chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d").unwrap()
            - chrono::NaiveDate::parse_from_str(since, "%Y-%m-%d").unwrap())
        .num_days()
            + 1;
        crate::report::build(&snap, "ws", "t", days as u32, None, None).unwrap()
    }

    #[test]
    fn prior_matches_window_length_across_suffixed_files() {
        let tmp = std::env::temp_dir().join(format!("mb-exec-prior-{}", std::process::id()));
        let day_dir = tmp.join("2026-08-01");
        std::fs::create_dir_all(&day_dir).unwrap();
        let daily = report("2026-07-31", "2026-07-31", "2026-08-01");
        let weekly = report("2026-07-25", "2026-07-31", "2026-08-01");
        std::fs::write(
            day_dir.join("creative-report.json"),
            serde_json::to_string(&daily).unwrap(),
        )
        .unwrap();
        std::fs::write(
            day_dir.join("creative-report-2026-07-25-to-2026-07-31.json"),
            serde_json::to_string(&weekly).unwrap(),
        )
        .unwrap();

        // A weekly run finds the range-suffixed weekly, not the daily.
        let p = load_prior(&tmp, "2026-08-02", 7).expect("weekly prior");
        assert_eq!(p.window.since, "2026-07-25");
        // A daily run still finds the daily.
        let p = load_prior(&tmp, "2026-08-02", 1).expect("daily prior");
        assert_eq!(p.window.since, "2026-07-31");
        // No comparable window length: none.
        assert!(load_prior(&tmp, "2026-08-02", 30).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
