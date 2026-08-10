//! HTML renderer (slice B2): a pure function over report.json + the
//! asset cache - it never reads snapshots and never touches the
//! network. Board layout adapted from the hand-built Fincity report;
//! images inlined base64 so the file is fully self-contained.

use std::fmt::Write as _;
use std::path::Path;

use crate::schema::*;

const TEMPLATE: &str = include_str!("template.html");
/// Meta-side vs CRM-side KPI dot classes (visual language of the
/// original report: blue = Meta delivery, green = CRM truth).
const M: &str = "m";
const C: &str = "c";

/// Render the full report. `history_dir` resolves `image.path` refs
/// (relative to `<workspace>/.moneyball/history/`); a missing file
/// degrades to the placeholder, never an error.
pub fn render(r: &CreativeReport, history_dir: &Path) -> String {
    let window = if r.window.since == r.window.until {
        r.window.since.clone()
    } else {
        format!("{} .. {}", r.window.since, r.window.until)
    };

    let mut jumpnav = String::from(r##"<a href="#portfolio">Portfolio</a>"##);
    for p in &r.products {
        let _ = write!(
            jumpnav,
            r##"<a href="#p-{}">{}</a>"##,
            slug(&p.product),
            esc(&p.product)
        );
    }

    let mut sections = String::new();
    for p in &r.products {
        render_section(&mut sections, p, &r.report_date, history_dir);
    }

    let note = if r.source.crm_present {
        String::new()
    } else {
        r#"<div class="warnbox">No CRM data in this snapshot - L-Leads/Qualified/Visit/Booking are zeros, not truths. Run a CRM fetch and regenerate.</div>"#.into()
    };

    TEMPLATE
        .replace("{{TITLE}}", &format!("Creative Report - {}", r.report_date))
        .replace("{{WINDOW}}", &esc(&window))
        .replace("{{DATE}}", &esc(&r.report_date))
        .replace("{{JUMPNAV}}", &jumpnav)
        .replace("{{PORTFOLIO_KPIS}}", &kpi_cells(&r.portfolio))
        .replace("{{PORTFOLIO_NOTE}}", &note)
        .replace("{{SECTIONS}}", &sections)
        .replace("{{GENERATED}}", &esc(&r.generated_at))
}

fn render_section(out: &mut String, p: &ProductSection, report_date: &str, history_dir: &Path) {
    let _ = write!(
        out,
        r#"<div class="pblock" id="p-{}"><section><div class="seckick">{}</div><h2>{}</h2><div class="okpis">{}</div>{}<div class="cboard">"#,
        slug(&p.product),
        esc(&p.product),
        esc(&p.product),
        kpi_cells(&p.kpis),
        super::table::comparison_table(p, report_date)
    );
    for (i, c) in p.creatives.iter().enumerate() {
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
            let (mime, payload) = card_image(&bytes, ext_of(c));
            format!(
                r#"<img src="data:{};base64,{}" alt="" loading="lazy">"#,
                mime,
                b64(&payload)
            )
        })
        .unwrap_or_else(|| r#"<div class="noimg">no preview</div>"#.into());
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
    let _ = write!(
        out,
        r#"<div class="ccard" id="c-{}-{}"><div class="cc-fig"><span class="crank">{}</span>{}{}</div><div class="cc-body"><div class="cc-name" title="{}">{}</div><div class="cc-camp">{}</div><div class="cc-kpis">{}{}{}{}</div>{}<div class="cc-meta"><span class="stx {}">{}</span><span>{} ad(s){}</span></div>{}</div></div>"#,
        pslug,
        rank,
        rank,
        vtag,
        img,
        esc(&c.display_name),
        esc(&c.display_name),
        esc(&c.campaigns.join(" / ")),
        kpi(
            &format!("\u{20B9}{}", commas(c.delivery.spend.round() as u64)),
            "spend"
        ),
        kpi(&commas(c.delivery.impressions), "impr"),
        kpi(&cpl, "cpl"),
        kpi(&ctr, "ctr"),
        funnel_bars(c),
        st_class,
        esc(&c.status.label),
        c.ad_ids.len(),
        c.created
            .as_deref()
            .map(|d| format!(" &middot; since {}", esc(d)))
            .unwrap_or_default(),
        targeting_chips(c),
    );
}

fn ext_of(c: &CreativeCard) -> &str {
    c.image
        .as_ref()
        .and_then(|i| i.path.rsplit('.').next())
        .unwrap_or("jpg")
}

/// Cards render ~300px wide; inlining full-res originals bloated the
/// file to 20MB. Downscale to card width and re-encode JPEG; anything
/// that fails to decode (or is already small) inlines verbatim.
fn card_image(bytes: &[u8], ext: &str) -> (&'static str, Vec<u8>) {
    const MAX_W: u32 = 720;
    const KEEP_UNDER: usize = 120 * 1024;
    let mime = match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    };
    if bytes.len() < KEEP_UNDER {
        return (mime, bytes.to_vec());
    }
    let Ok(img) = image::load_from_memory(bytes) else {
        return (mime, bytes.to_vec());
    };
    let img = if img.width() > MAX_W {
        img.resize(MAX_W, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78);
    match img.to_rgb8().write_with_encoder(enc) {
        Ok(()) => ("image/jpeg", out.into_inner()),
        Err(_) => (mime, bytes.to_vec()),
    }
}

/// The 5 lead stages as horizontal bars, scaled to M-Leads (the widest
/// lead-stage count; impressions/clicks stay numeric in the KPI grid).
fn funnel_bars(c: &CreativeCard) -> String {
    let max = c.funnel.iter().skip(2).map(|s| s.count).max().unwrap_or(0);
    let mut out = String::from(r#"<div class="hfunnel">"#);
    for (i, s) in c.funnel.iter().enumerate().skip(2) {
        let pct = if max > 0 {
            (s.count as f64 / max as f64 * 100.0).max(2.0)
        } else {
            2.0
        };
        let color = if i == 2 { "var(--meta)" } else { "var(--crm)" };
        let _ = write!(
            out,
            r#"<div class="hf-row"><span class="hf-lab">{}</span><span class="hf-track"><span class="hf-bar" style="width:{:.0}%;background:{}"></span></span><span class="hf-v">{}</span></div>"#,
            esc(&s.stage),
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
    let mut out = String::from(r#"<div class="capirow">"#);
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

fn kpi_cells(k: &Kpis) -> String {
    let f = &k.funnel;
    let cpl = if f.m_leads > 0 {
        format!(
            "\u{20B9}{}",
            commas((k.spend / f.m_leads as f64).round() as u64)
        )
    } else {
        "-".into()
    };
    let ctr = if k.impressions > 0 {
        format!("{:.1}%", k.clicks as f64 / k.impressions as f64 * 100.0)
    } else {
        "-".into()
    };
    let mut out = String::new();
    for (v, l, dot) in [
        (
            format!("\u{20B9}{}", commas(k.spend.round() as u64)),
            "spend",
            M,
        ),
        (commas(k.impressions), "impr", M),
        (f.m_leads.to_string(), "M-Leads", M),
        (cpl, "CPL", M),
        (ctr, "CTR", M),
        (f.l_leads.to_string(), "L-Leads", C),
        (f.qualified.to_string(), "Qualified", C),
        (f.visit.to_string(), "Visits", C),
        (f.booking.to_string(), "Bookings", C),
    ] {
        let _ = write!(
            out,
            r#"<div class="ok"><div class="okv tabnum">{}</div><div class="okl"><i class="kdot {}"></i>{}</div></div>"#,
            v, dot, l
        );
    }
    if k.unattributed_l_leads > 0 {
        let _ = write!(
            out,
            r#"<div class="ok"><div class="okv tabnum">{}</div><div class="okl"><i class="kdot c"></i>unattributed L</div></div>"#,
            k.unattributed_l_leads
        );
    }
    out
}

fn kpi(v: &str, l: &str) -> String {
    format!(
        r#"<div class="cc-k"><div class="cc-kv">{}</div><div class="cc-kl">{}</div></div>"#,
        v, l
    )
}

/// Minimal HTML escape for authored-data text nodes and attributes.
pub(super) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Anchor-safe slug (ASCII lowercase + dashes), python _slug parity.
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

/// Western 3-digit grouping (matches the python report's number style).
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

/// Dependency-free base64 (standard alphabet, padded).
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_matches_known_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn commas_and_slug() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(1234), "1,234");
        assert_eq!(commas(82132), "82,132");
        assert_eq!(commas(1234567), "1,234,567");
        assert_eq!(
            slug("Purva Sparkling Spring by Fincity"),
            "purva-sparkling-spring-by-fincity"
        );
        assert_eq!(slug("A/B (test)"), "a-b-test");
    }

    #[test]
    fn esc_neutralizes_markup() {
        assert_eq!(esc(r#"<b x="1">&"#), "&lt;b x=&quot;1&quot;&gt;&amp;");
    }
}
