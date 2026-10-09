//! Where every source stands - Meta (the ads), the CRM (the leads), the
//! model (the advisor) - read from auth.json, crm.toml, config.json and
//! the newest run record only, so it is instant. What `moneyball connect`
//! prints and the TUI's /connect shows; each row names the one command
//! that connects or fixes it today.

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::config::AppConfig;
use crate::schema::{RunRecord, RunStep, StepStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Meta,
    Crm,
    Model,
}

/// A source's state: connected, refused (its last pull was turned away -
/// expired or removed), or missing (never set up, or a key not stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    Connected,
    Refused,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub provider: Provider,
    /// "ads", "leads" or "advisor".
    pub group: &'static str,
    /// "Meta", the CRM's name, the model provider.
    pub name: String,
    pub link: Link,
    pub detail: String,
    /// The one command that connects or fixes it ("/..." runs in the app).
    pub fix: Option<String>,
}

pub fn status_all(cfg: &AppConfig) -> Vec<Source> {
    let run = crate::runs::latest(cfg);
    vec![meta(cfg, run.as_ref()), crm(cfg, run.as_ref()), model(cfg)]
}

/// A fix as the shell says it: an in-app command needs moneyball open.
pub fn fix_in_shell(fix: &str) -> String {
    match fix.strip_prefix('/') {
        Some(cmd) => format!("open moneyball and run /{}", cmd),
        None => fix.to_string(),
    }
}

fn meta(cfg: &AppConfig, run: Option<&RunRecord>) -> Source {
    let products = cfg.workspace.as_ref().map_or(0, |w| w.products.len());
    let has_token = crate::secrets::load_meta_token().is_some();
    let step = last_step(cfg, run, "meta");
    let (link, detail, fix) = match step {
        _ if products == 0 => (Link::Missing, "no products yet".to_string(), "/setup"),
        _ if !has_token => (Link::Missing, "no token saved".to_string(), "/setup meta"),
        Some((s, at)) if s.error_kind.as_deref() == Some("meta_auth") => (
            Link::Refused,
            format!(
                "last pull refused {}: {}",
                at,
                s.message.clone().unwrap_or_default()
            ),
            "/setup meta",
        ),
        Some((s, at)) => {
            let how = match s.status {
                StepStatus::Partial => format!("{} products", crate::refresh::status_word(s)),
                _ => "ok".into(),
            };
            (
                Link::Connected,
                format!("{} products, last pull {} {}", products, how, at),
                "",
            )
        }
        None => (
            Link::Connected,
            format!("{} products, token saved", products),
            "",
        ),
    };
    source(Provider::Meta, "ads", "Meta".into(), link, detail, fix)
}

fn crm(cfg: &AppConfig, run: Option<&RunRecord>) -> Source {
    let spec = std::fs::read_to_string(crate::crm::fetch::spec_path(cfg))
        .ok()
        .and_then(|raw| crate::crm::source::parse(&raw).ok());
    let Some(spec) = spec else {
        return source(
            Provider::Crm,
            "leads",
            "CRM".into(),
            Link::Missing,
            "no CRM connected".into(),
            "moneyball crm connect",
        );
    };
    let unstored: Vec<String> = spec
        .request
        .as_ref()
        .map(|r| r.secret_names())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| crate::secrets::load_crm_key(cfg.workspace_id(), n).is_none())
        .collect();
    let (link, detail, fix) = match last_step(cfg, run, "crm") {
        _ if !unstored.is_empty() => (
            Link::Missing,
            format!("{} not stored", unstored.join(", ")),
            format!("moneyball crm secret {}", unstored[0]),
        ),
        Some((s, at)) if s.status == StepStatus::Failed => (
            if s.error_kind.as_deref() == Some("crm_auth") {
                Link::Refused
            } else {
                Link::Connected
            },
            format!(
                "last pull failed {}: {}",
                at,
                s.message.clone().unwrap_or_default()
            ),
            s.fix.clone().unwrap_or_default(),
        ),
        Some((s, at)) => (
            Link::Connected,
            format!("{} {}", crate::refresh::counts_detail(s), at),
            String::new(),
        ),
        None => (
            Link::Connected,
            "key saved, no pull yet".into(),
            String::new(),
        ),
    };
    source(Provider::Crm, "leads", spec.name, link, detail, &fix)
}

fn model(cfg: &AppConfig) -> Source {
    let w = cfg.workspace.as_ref();
    let provider = w.and_then(|w| w.model_provider.clone()).unwrap_or_default();
    let name = w.and_then(|w| w.model.clone()).unwrap_or_default();
    let (link, detail) = if provider.is_empty() || name.is_empty() {
        (Link::Missing, "no model chosen".to_string())
    } else if crate::secrets::load_llm_key(&provider).is_none() {
        (Link::Missing, format!("{}: no key saved", name))
    } else {
        (Link::Connected, format!("{}, key saved", name))
    };
    let fix = if link == Link::Connected {
        ""
    } else {
        "/setup"
    };
    let shown = if provider.is_empty() {
        "model".to_string()
    } else {
        provider
    };
    source(Provider::Model, "advisor", shown, link, detail, fix)
}

/// The newest pull of `name` (not a skip), and when, in words: a reused
/// Meta pull is dated by the snapshot it wrote, as /status does.
fn last_step<'a>(
    cfg: &AppConfig,
    run: Option<&'a RunRecord>,
    name: &str,
) -> Option<(&'a RunStep, String)> {
    let run = run?;
    let s = run
        .steps
        .iter()
        .find(|s| s.name == name && s.status != StepStatus::Skipped)?;
    let at: Option<DateTime<Local>> = if s.reused {
        let ads = cfg.snap_dir().join(&run.date).join("ads_daily.json");
        std::fs::metadata(ads)
            .ok()?
            .modified()
            .ok()
            .map(DateTime::from)
    } else {
        DateTime::parse_from_rfc3339(&run.finished_at)
            .ok()
            .map(|t| t.with_timezone(&Local))
    };
    let at = at
        .map(|t| crate::status::when(t, Local::now().date_naive()))
        .unwrap_or_default();
    Some((s, at))
}

fn source(
    provider: Provider,
    group: &'static str,
    name: String,
    link: Link,
    detail: String,
    fix: &str,
) -> Source {
    Source {
        provider,
        group,
        name,
        link,
        detail,
        fix: (!fix.is_empty()).then(|| fix.to_string()),
    }
}
