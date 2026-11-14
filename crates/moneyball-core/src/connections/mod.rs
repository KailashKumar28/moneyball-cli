//! Where every source stands - Meta (the ads), the CRM (the leads), the
//! model (the advisor) - read from auth.json, crm.toml, config.json and
//! the newest run record only, so it is instant. What `moneyball connect`
//! prints and the TUI's /connect shows; each row names the one command
//! that connects or fixes it today.

pub mod crm;

use chrono::Local;
use serde::Serialize;

use crate::config::AppConfig;
use crate::schema::{RunRecord, RunStep, StepStatus};

/// What a source is for: the ads (Meta), the leads (the CRM), the
/// advisor (the model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Ads,
    Leads,
    Advisor,
}

impl Group {
    pub fn word(self) -> &'static str {
        match self {
            Group::Ads => "ads",
            Group::Leads => "leads",
            Group::Advisor => "advisor",
        }
    }
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
    pub group: Group,
    /// "Meta", the CRM's name, the model provider.
    pub name: String,
    pub link: Link,
    pub detail: String,
    /// The one command that connects or fixes it ("/..." runs in the app).
    pub fix: Option<String>,
}

/// One connect flow, two front ends: the shell prints and reads stdin;
/// the TUI sends each line and question to its worker channel and waits
/// for the composer.
pub trait Prompter {
    fn show(&mut self, line: &str);
    /// The answer, trimmed; `secret` hides what is typed. None: cancelled.
    fn ask(&mut self, prompt: &str, secret: bool) -> Option<String>;
}

pub fn status_all(cfg: &AppConfig) -> Vec<Source> {
    let run = crate::runs::latest(cfg);
    vec![
        meta_row(cfg, run.as_ref()),
        crm_row(cfg, run.as_ref()),
        model_row(cfg),
    ]
}

/// A fix as the shell says it: its own twin when there is one, else
/// "open moneyball and run /...".
pub fn fix_in_shell(fix: &str) -> String {
    match fix {
        "/connect crm" => "moneyball connect crm".into(),
        f => match f.strip_prefix('/') {
            Some(cmd) => format!("open moneyball and run /{}", cmd),
            None => f.to_string(),
        },
    }
}

fn meta_row(cfg: &AppConfig, run: Option<&RunRecord>) -> Source {
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
        Some((s, at)) if s.status == StepStatus::Failed => (
            Link::Connected,
            format!(
                "last pull failed {}: {}",
                at,
                s.message.clone().unwrap_or_default()
            ),
            "/refresh",
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
    source(Group::Ads, "Meta".into(), link, detail, fix)
}

fn crm_row(cfg: &AppConfig, run: Option<&RunRecord>) -> Source {
    let spec = std::fs::read_to_string(crate::crm::fetch::spec_path(cfg))
        .ok()
        .and_then(|raw| crate::crm::source::parse(&raw).ok());
    let Some(spec) = spec else {
        // A CRM's keys belong to a client: with none set up yet,
        // `/connect crm` would refuse, so setup comes first.
        let set_up = cfg
            .workspace
            .as_ref()
            .is_some_and(|w| !w.products.is_empty());
        let (detail, fix) = if set_up {
            ("no CRM connected", "/connect crm")
        } else {
            ("no CRM connected; set the client up first", "/setup")
        };
        return source(
            Group::Leads,
            "CRM".into(),
            Link::Missing,
            detail.into(),
            fix,
        );
    };
    let unstored = crate::crm::status::unstored_secrets(cfg, spec.request.as_ref());
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
    source(Group::Leads, spec.name, link, detail, &fix)
}

fn model_row(cfg: &AppConfig) -> Source {
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
    source(Group::Advisor, shown, link, detail, fix)
}

/// The newest pull of `name` (not a skip), and when, in words - dated as
/// /status dates it (status::last_answer).
fn last_step<'a>(
    cfg: &AppConfig,
    run: Option<&'a RunRecord>,
    name: &str,
) -> Option<(&'a RunStep, String)> {
    let (s, at) = crate::status::last_answer(cfg, run?, name)?;
    let at = at.map(|t| crate::status::when(t, Local::now().date_naive()));
    Some((s, at.unwrap_or_default()))
}

fn source(group: Group, name: String, link: Link, detail: String, fix: &str) -> Source {
    Source {
        group,
        name,
        link,
        detail,
        fix: (!fix.is_empty()).then(|| fix.to_string()),
    }
}
