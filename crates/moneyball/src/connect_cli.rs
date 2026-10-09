//! `moneyball connect`: where every source stands (core
//! connections::status_all) with the one command that fixes it, as text
//! or, with MB_AGENT=1, JSON. `moneyball connect crm` runs the guided CRM
//! flow. Meta and the model connect inside moneyball (/setup meta,
//! /setup) until one-click sign-in exists.

use anyhow::Result;
use clap::Subcommand;

use moneyball_core::connections::{self, Link};
use moneyball_core::AppConfig;

#[derive(Subcommand, Debug)]
pub(crate) enum Connect {
    /// Connect a CRM: pick it, give its keys, a test pull gates the save.
    Crm,
}

pub(crate) fn run(cfg: &AppConfig, what: Option<Connect>) -> Result<()> {
    if let Some(Connect::Crm) = what {
        return crate::connect_flow::run(cfg);
    }
    let sources = connections::status_all(cfg);
    if cfg.agent {
        println!("{}", serde_json::to_string(&sources)?);
        return Ok(());
    }
    for s in &sources {
        let state = match s.link {
            Link::Connected => "connected",
            Link::Refused => "refused",
            Link::Missing => "missing",
        };
        println!("{:<9}{:<12}{:<11}{}", s.group, s.name, state, s.detail);
        if let Some(fix) = &s.fix {
            println!("{:<32}fix: {}", "", connections::fix_in_shell(fix));
        }
    }
    Ok(())
}
