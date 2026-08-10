//! HTML renderer: a pure function over report.json + the asset cache -
//! never snapshots, never the network. v2 layout per the 2026-08-10
//! specialist specs: exec brief, missing-leads banner, scorecard with
//! deltas, glance table, per-product targeting block, client-facing
//! labels (Meta leads / CRM leads), dead-tail collapse.

use std::fmt::Write as _;
use std::path::Path;

use crate::schema::*;

const TEMPLATE: &str = include_str!("template.html");

/// Display names for the fixed funnel stage keys (the JSON keeps the
/// stable contract names; only the rendering translates).
pub(super) fn stage_label(stage: &str) -> &str {
    match stage {
        "M-Leads" => "Meta leads",
        "L-Leads" => "CRM leads",
        "Visit" => "Visits",
        "Booking" => "Bookings",
        s => s,
    }
}

/// A card is "dead tail" (collapsed table row, no card) when it has no
/// Meta or CRM leads and spent under 2% of the product's spend.
pub(super) fn is_dead_tail(c: &CreativeCard, product_spend: f64) -> bool {
    c.funnel[2].count == 0 && c.funnel[3].count == 0 && c.delivery.spend < 0.02 * product_spend
}

pub fn render(
    r: &CreativeReport,
    prior: Option<&CreativeReport>,
    brand: &str,
    history_dir: &Path,
) -> String {
    let day = chrono::NaiveDate::parse_from_str(&r.window.until, "%Y-%m-%d").ok();
    let multi = r.window.since != r.window.until;
    let dayline = if multi {
        format!("{} - {}", esc(&r.window.since), esc(&r.window.until)).to_uppercase()
    } else {
        day.map(|d| d.format("%a %d %b %Y").to_string().to_uppercase())
            .unwrap_or_else(|| r.window.until.clone())
    };
    let window = if r.window.since == r.window.until {
        day.map(|d| d.format("%d %b %Y").to_string())
            .unwrap_or_else(|| r.window.until.clone())
    } else {
        format!("{} .. {}", r.window.since, r.window.until)
    };
    let freshness = {
        let pulled = chrono::DateTime::parse_from_rfc3339(&r.generated_at)
            .map(|t| {
                (t + chrono::Duration::minutes(330))
                    .format("%d %b, %I:%M %P IST")
                    .to_string()
            })
            .unwrap_or_else(|_| r.generated_at.clone());
        format!(
            "Data: {}, full day &middot; pulled {} &middot; sources: Meta Ads + your CRM{}",
            esc(&window),
            esc(&pulled),
            prior
                .map(|p| format!(" &middot; compared with {}", esc(&p.window.until)))
                .unwrap_or_else(|| " &middot; first comparable day".into())
        )
    };

    let mut jumpnav = String::new();
    for p in &r.products {
        let _ = write!(
            jumpnav,
            r##"<a href="#p-{}">{}</a>"##,
            slug(&p.product),
            esc(&p.product)
        );
    }
    let products_line = r
        .products
        .iter()
        .map(|p| p.product.as_str())
        .collect::<Vec<_>>()
        .join(" &middot; ");

    let (score1, score2) = super::sections::scorecard(r, prior);
    let mut sections = String::new();
    for p in &r.products {
        render_section(&mut sections, p, &r.report_date, &window, history_dir);
    }
    let note = if r.source.crm_present {
        String::new()
    } else {
        r#"<div class="warnbox">No CRM data in this snapshot - CRM leads/Qualified/Visits/Bookings are zeros, not truths. Run a CRM fetch and regenerate.</div>"#.into()
    };

    TEMPLATE
        .replace(
            "{{TITLE}}",
            &format!("{} Daily - {}", brand, r.window.until),
        )
        .replace("{{DAYLINE}}", &dayline)
        .replace("{{H1}}", &format!("{} Portfolio", esc(brand)))
        .replace("{{PIPELINE}}", "Meta Ads -&gt; CRM -&gt; Qualified")
        .replace("{{PRODUCTS_LINE}}", &products_line)
        .replace("{{FRESHNESS}}", &freshness)
        .replace("{{JUMPNAV}}", &jumpnav)
        .replace("{{EXEC}}", &super::sections::exec_brief(r))
        .replace("{{MISSING}}", &super::sections::missing_banner(r))
        .replace("{{SCORECARD}}", &score1)
        .replace("{{SCORECARD2}}", &score2)
        .replace("{{RECON}}", &super::sections::reconciliation(r))
        .replace("{{PORTFOLIO_NOTE}}", &note)
        .replace("{{GLANCE}}", &super::sections::glance(r))
        .replace("{{SECTIONS}}", &sections)
        .replace("{{WINDOW}}", &window)
}

fn render_section(
    out: &mut String,
    p: &ProductSection,
    report_date: &str,
    window: &str,
    history_dir: &Path,
) {
    let _ = write!(
        out,
        r#"<div class="pblock" id="p-{}"><section><div class="seckick">{} &middot; {}</div><h2>{}</h2><div class="okpis">{}</div>{}{}<div class="subhead"><span>Creatives</span></div><div class="cboard">"#,
        slug(&p.product),
        esc(&p.product),
        esc(window),
        esc(&p.product),
        product_kpis(&p.kpis),
        super::table::comparison_table(p, report_date),
        super::sections::targeting_block(p),
    );
    for (i, c) in p.creatives.iter().enumerate() {
        if is_dead_tail(c, p.kpis.spend) {
            continue; // lives only in the table's collapsed tail
        }
        render_card(out, i + 1, &slug(&p.product), c, history_dir);
    }
    let _ = write!(out, "</div></section></div>");
}

fn render_card(out: &mut String, rank: usize, pslug: &str, c: &CreativeCard, history_dir: &Path) {
    let img = c
        .image
        .as_ref()
        .and_then(|i| std::fs::read(history_dir.join(&i.path)).ok())
        .map(|bytes| {
            let (mime, payload) = super::img::card_image(&bytes, super::img::ext_of(c));
            format!(
                r#"<img src="data:{};base64,{}" alt="" loading="lazy">"#,
                mime,
                super::img::b64(&payload)
            )
        })
        .unwrap_or_else(|| r#"<div class="noimg">no preview</div>"#.into());
    // Click the creative -> open the live Instagram/FB post.
    let img = match c.permalink.as_deref() {
        Some(url) => format!(
            r#"<a class="cc-link" href="{}" target="_blank" rel="noopener" title="Open the live post">{}<span class="golive">view post</span></a>"#,
            esc(url),
            img
        ),
        None => img,
    };
    let vtag = if c.is_video {
        r#"<span class="vtag">VIDEO</span>"#
    } else {
        ""
    };
    let st_class = match c.status.code {
        StatusCode::Live => "st-live",
        StatusCode::Learn => "st-learn",
        StatusCode::Stop => "st-stop",
    };
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
    let missing = c
        .segmentation
        .as_ref()
        .filter(|s| s.uncaptured > 0)
        .map(|s| {
            format!(
                r#"<div class="cc-miss">{} lead(s) missing from CRM</div>"#,
                s.uncaptured
            )
        })
        .unwrap_or_default();
    let ads = if c.ad_ids.len() == 1 {
        "1 ad".to_string()
    } else {
        format!("{} ads", c.ad_ids.len())
    };
    let _ = write!(
        out,
        r#"<div class="ccard" id="c-{}-{}"><div class="cc-fig"><span class="crank">{}</span>{}{}</div><div class="cc-body"><div class="cc-name" title="{}">{}</div><div class="cc-camp">{}</div><div class="cc-kpis">{}{}{}{}</div>{}{}<div class="cc-meta"><span class="stx {}">{}</span><span>{}{}</span></div>{}</div></div>"#,
        pslug,
        rank,
        rank,
        vtag,
        img,
        esc(&c.display_name),
        esc(&c.display_name),
        esc(&c.campaigns.join(" / ")),
        kpi(&rupees(c.delivery.spend), "spend"),
        kpi(&commas(c.delivery.impressions), "impressions"),
        kpi(&cpl, "cost / meta lead"),
        kpi(&ctr, "ctr"),
        funnel_bars(c),
        missing,
        st_class,
        esc(&c.status.label),
        ads,
        c.created
            .as_deref()
            .map(|d| format!(" &middot; since {}", esc(d)))
            .unwrap_or_default(),
        targeting_chips(c),
    );
}

/// The 5 lead stages as horizontal bars; zero paints NO ink.
fn funnel_bars(c: &CreativeCard) -> String {
    let max = c.funnel.iter().skip(2).map(|s| s.count).max().unwrap_or(0);
    let mut out = String::from(r#"<div class="hfunnel">"#);
    for (i, s) in c.funnel.iter().enumerate().skip(2) {
        let zero = s.count == 0;
        let pct = if max > 0 && !zero {
            (s.count as f64 / max as f64 * 100.0).max(2.0)
        } else {
            0.0
        };
        let color = if i == 2 { "var(--meta)" } else { "var(--crm)" };
        let _ = write!(
            out,
            r#"<div class="hf-row"><span class="hf-lab">{}</span><span class="hf-track"><span class="hf-bar{}" style="width:{:.0}%;background:{}"></span></span><span class="hf-v">{}</span></div>"#,
            esc(stage_label(&s.stage)),
            if zero { " zero" } else { "" },
            pct,
            color,
            s.count
        );
    }
    out.push_str("</div>");
    out
}

fn targeting_chips(c: &CreativeCard) -> String {
    if c.targetings.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<div class="capirow"><span class="capi-lbl">audience:</span>"#);
    for t in c.targetings.iter().take(4) {
        let _ = write!(
            out,
            r#"<span class="capi-chip">{}<b>{}</b>M</span>"#,
            esc(&t.targeting),
            t.delivery.m_leads
        );
    }
    out.push_str("</div>");
    out
}

fn kpi(v: &str, l: &str) -> String {
    format!(
        r#"<div class="cc-k"><div class="cc-kv">{}</div><div class="cc-kl">{}</div></div>"#,
        v, l
    )
}

fn product_kpis(k: &Kpis) -> String {
    let f = &k.funnel;
    let cpq = k
        .cost_per_qualified
        .map(rupees)
        .unwrap_or_else(|| "-".into());
    let mut out = String::new();
    for (v, l, dot) in [
        (rupees(k.spend), "Spend", "m"),
        (f.m_leads.to_string(), "Meta leads", "m"),
        (f.l_leads.to_string(), "CRM leads", "c"),
        (f.qualified.to_string(), "Qualified", "c"),
        (cpq, "Cost / qualified", "c"),
    ] {
        let _ = write!(
            out,
            r#"<div class="ok"><div class="okv tabnum">{}</div><div class="okl"><i class="kdot {}"></i>{}</div></div>"#,
            v, dot, l
        );
    }
    out
}

/// Minimal HTML escape for data text nodes and attributes.
pub(super) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

pub(super) fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Rupee amount with the sign entity (authored Rust stays ASCII).
pub(super) fn rupees(v: f64) -> String {
    format!("&#8377;{}", commas(v.round().max(0.0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_matches_known_vectors() {
        assert_eq!(super::super::img::b64(b""), "");
        assert_eq!(super::super::img::b64(b"f"), "Zg==");
        assert_eq!(super::super::img::b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn commas_slug_esc_rupees() {
        assert_eq!(commas(82132), "82,132");
        assert_eq!(slug("A/B (test)"), "a-b-test");
        assert_eq!(esc(r#"<b x="1">&"#), "&lt;b x=&quot;1&quot;&gt;&amp;");
        assert_eq!(rupees(954.4), "&#8377;954");
    }

    #[test]
    fn stage_labels_translate_for_clients() {
        assert_eq!(stage_label("M-Leads"), "Meta leads");
        assert_eq!(stage_label("L-Leads"), "CRM leads");
        assert_eq!(stage_label("Qualified"), "Qualified");
    }
}
