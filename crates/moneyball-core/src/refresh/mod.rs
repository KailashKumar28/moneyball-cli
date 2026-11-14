//! The daily refresh: Meta, then CRM anchored to the same snapshot day,
//! then yesterday's report - under one lock, with one run record written
//! whatever happens. Orchestration only: every network call lives in the
//! modules it calls (fetch/, crm/fetch.rs), so the boundary is unchanged.
//!
//! Outcome: `failed` when Meta failed (nothing new was written and CRM is
//! skipped, so it never lands on a different day than Meta); `degraded`
//! when something is missing (CRM failed or not connected, a product's
//! account, the creatives/leads/adsets capture, a cancel); `ok` otherwise.

mod finish;
mod show;
mod worker;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub(crate) use finish::REUSES_META;
pub use finish::{cancelled, fixes, gaps, rerun, verdict, RunVerdict};
pub use show::*;
pub use worker::{spawn, Ev};

use crate::config::AppConfig;
use crate::error::Result;
use crate::schema::{Outcome, RunRecord, RunStep, StepStatus, Trigger, RUN_SCHEMA};

/// What the caller hears while a refresh runs: a step starting (one of
/// STEPS) and a step finished (its record).
pub enum Progress<'a> {
    Begin(&'static str),
    Done(&'a RunStep),
}

/// A step skipped because Esc stopped the run (its error_kind).
pub const CANCELLED: &str = "cancelled";

pub struct Options {
    pub trigger: Trigger,
    /// Pull Meta again even when today's snapshot already has it.
    pub force: bool,
    /// Trailing days to pull, ending yesterday (Meta and CRM alike).
    pub days: u32,
    /// Set to stop after the step in flight (a Meta pull can't be cut
    /// mid-request); the steps left are recorded as skipped.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Run every step, telling `on_step` as each starts and finishes, and return the
/// record (also written to `runs/<date>.json`). Err only when the run
/// cannot start: no workspace, or another refresh holds the lock.
pub fn run(
    cfg: &AppConfig,
    meta_token: Option<&str>,
    opts: &Options,
    on_step: &mut dyn FnMut(Progress),
) -> Result<RunRecord> {
    let _lock = crate::runs::lock(cfg)?;
    let date = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    // Today's earlier run, read before this one replaces its record: a
    // reused Meta pull carries that run's steps forward, gaps included.
    let earlier = crate::runs::read(cfg, &date);
    let mut run = RunRecord {
        schema: RUN_SCHEMA.into(),
        workspace_id: cfg
            .workspace
            .as_ref()
            .and_then(|w| w.workspace_id.clone())
            .unwrap_or_default(),
        date: date.clone(),
        trigger: opts.trigger,
        started_at: now(),
        finished_at: String::new(),
        outcome: Outcome::Running,
        steps: Vec::new(),
        artifacts: BTreeMap::new(),
    };
    crate::runs::write(cfg, &run)?;

    // 1 + 2. Meta insights, then creatives/leads/adsets (one fetch call).
    let snap_dir = cfg.snap_dir().join(&date);
    let reused = !opts.force && snap_dir.join("ads_daily.json").is_file();
    if reused {
        let carried = |name: &str| {
            let s = earlier.as_ref()?.steps.iter().find(|s| s.name == name)?;
            matches!(s.status, StepStatus::Done | StepStatus::Partial).then(|| RunStep {
                ms: 0,
                reused: true,
                ..s.clone()
            })
        };
        let meta = carried("meta").unwrap_or_else(|| {
            // No record of the pull (an older binary, or `moneyball fetch`):
            // say what the snapshot holds.
            let rows = crate::snapshot::load(&snap_dir)
                .map(|s| s.ads_daily.len() as u64)
                .unwrap_or(0);
            RunStep {
                reused: true,
                ..done("meta", 0, [("rows", rows)])
            }
        });
        push(cfg, &mut run, on_step, meta);
        let assets = carried("assets").unwrap_or_else(|| RunStep {
            reused: true,
            ..skipped("assets", "captured with that Meta pull")
        });
        push(cfg, &mut run, on_step, assets);
    } else {
        on_step(Progress::Begin("meta"));
        let started = Instant::now();
        let fetched = match meta_token {
            Some(token) => crate::fetch::fetch_snapshot(cfg, token, opts.days),
            None => Err(crate::error::Error::MetaAuth(
                "no Meta connection stored for this workspace".into(),
            )),
        };
        match fetched {
            Err(e) => {
                push(cfg, &mut run, on_step, failed("meta", started, &e));
                push(cfg, &mut run, on_step, skipped("assets", "Meta failed"));
                push(
                    cfg,
                    &mut run,
                    on_step,
                    skipped("crm", "Meta failed; CRM waits so both stay on one day"),
                );
                push(cfg, &mut run, on_step, skipped("report", "no new data"));
                return finish(cfg, run, Outcome::Failed);
            }
            Ok(f) => {
                let ms = elapsed(started);
                let landed: Vec<usize> = f
                    .per_product
                    .iter()
                    .filter_map(|(_, r)| r.as_ref().ok().copied())
                    .collect();
                let missing: Vec<String> = f
                    .per_product
                    .iter()
                    .filter_map(|(name, r)| r.as_ref().err().map(|e| format!("{}: {}", name, e)))
                    .collect();
                let mut step = done(
                    "meta",
                    ms,
                    [
                        ("rows", landed.iter().sum::<usize>() as u64),
                        ("products", landed.len() as u64),
                    ],
                );
                if !missing.is_empty() {
                    step.counts
                        .insert("products_failed".into(), missing.len() as u64);
                    step.status = StepStatus::Partial;
                    step.error_kind = Some("meta_product".into());
                    step.message = Some(format!("no data for {}", missing.join("; ")));
                }
                push(cfg, &mut run, on_step, step);
                run.artifacts
                    .insert("snapshot".into(), f.path.display().to_string());

                let errors: Vec<String> = [
                    ("creatives", &f.creatives_error),
                    ("leads", &f.leads_error),
                    ("adsets", &f.adsets_error),
                    ("changes", &f.changes_error),
                    ("7-day reach", &f.ads_7d_error),
                ]
                .into_iter()
                .filter_map(|(what, e)| e.as_ref().map(|e| format!("{}: {}", what, e)))
                .collect();
                let mut assets = done(
                    "assets",
                    0,
                    [
                        ("creatives", f.creatives as u64),
                        ("leads", f.leads as u64),
                        ("adsets", f.adsets as u64),
                        ("changes", f.changes as u64),
                        ("ads_7d", f.ads_7d as u64),
                    ],
                );
                if !errors.is_empty() {
                    assets.status = StepStatus::Partial;
                    assets.error_kind = Some("capture".into());
                    assets.message = Some(errors.join("; "));
                }
                push(cfg, &mut run, on_step, assets);
            }
        }
    }

    // 3. CRM, into this same snapshot (crm fetch anchors to it).
    let cancelled = |run: &mut RunRecord, on_step: &mut dyn FnMut(Progress), from: usize| {
        for name in &STEPS[from..] {
            let mut step = skipped(name, "cancelled");
            step.error_kind = Some(CANCELLED.into());
            push(cfg, run, on_step, step);
        }
    };
    if is_cancelled(opts) {
        cancelled(&mut run, on_step, 2);
        return finish(cfg, run, Outcome::Degraded);
    }
    if !crate::crm::fetch::spec_path(cfg).is_file() {
        let mut step = skipped("crm", "no CRM connected for this workspace");
        step.fix = Some("/connect crm".into());
        push(cfg, &mut run, on_step, step);
    } else {
        on_step(Progress::Begin("crm"));
        let started = Instant::now();
        let quiet = AppConfig {
            agent: true,
            ..cfg.clone()
        };
        match crate::crm::fetch::fetch_crm(&quiet, opts.days) {
            Ok(r) if r.path.is_some() => {
                push(
                    cfg,
                    &mut run,
                    on_step,
                    done(
                        "crm",
                        elapsed(started),
                        [("tickets", r.tickets as u64), ("pages", r.pages as u64)],
                    ),
                );
            }
            Ok(r) => {
                let mut step = done("crm", elapsed(started), [("tickets", r.tickets as u64)]);
                step.status = StepStatus::Failed;
                step.error_kind = Some("crm_invalid".into());
                step.message = Some(format!(
                    "{} CRM data failed validation, so crm.json was not written ({})",
                    r.name,
                    r.check.errors.first().cloned().unwrap_or_default()
                ));
                push(cfg, &mut run, on_step, step);
            }
            Err(e) => push(cfg, &mut run, on_step, failed("crm", started, &e)),
        }
    }

    // 4. Yesterday's report from this snapshot. Without CRM it still goes
    //    out: the verdict gate withholds every verdict CRM data would need.
    if is_cancelled(opts) {
        cancelled(&mut run, on_step, 3);
        return finish(cfg, run, Outcome::Degraded);
    }
    on_step(Progress::Begin("report"));
    let started = Instant::now();
    match crate::report::generate(cfg, Some(&date), 1) {
        Ok(out) => {
            let creatives: usize = out.report.products.iter().map(|p| p.creatives.len()).sum();
            let mut step = done(
                "report",
                elapsed(started),
                [("creatives", creatives as u64)],
            );
            if !out.report.source.crm_present {
                step.status = StepStatus::Partial;
                step.error_kind = Some("no_crm".into());
                step.message = Some(
                    "written without CRM data: CRM figures left out, verdicts withheld".into(),
                );
            }
            push(cfg, &mut run, on_step, step);
            run.artifacts
                .insert("report".into(), out.html_path.display().to_string());
            run.artifacts
                .insert("report_json".into(), out.json_path.display().to_string());
        }
        Err(e) => push(cfg, &mut run, on_step, failed("report", started, &e)),
    }

    // Recs move on with the new data; a ledger it cannot write shows its
    // error when it is next opened.
    let _ = crate::ledger::catch_up(cfg, chrono::Local::now().date_naive());

    let missing = run
        .steps
        .iter()
        .any(|s| matches!(s.status, StepStatus::Failed | StepStatus::Partial));
    finish(
        cfg,
        run,
        if missing {
            Outcome::Degraded
        } else {
            Outcome::Ok
        },
    )
}

/// Record a finished step: tell the caller, and rewrite the run record so
/// a run that dies midway still says how far it got.
fn push(cfg: &AppConfig, run: &mut RunRecord, on_step: &mut dyn FnMut(Progress), step: RunStep) {
    on_step(Progress::Done(&step));
    run.steps.push(step);
    let _ = crate::runs::write(cfg, run);
}

fn is_cancelled(opts: &Options) -> bool {
    opts.cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::SeqCst))
}

fn finish(cfg: &AppConfig, mut run: RunRecord, outcome: Outcome) -> Result<RunRecord> {
    run.outcome = outcome;
    run.finished_at = now();
    crate::runs::write(cfg, &run)?;
    Ok(run)
}

fn done<const N: usize>(name: &str, ms: u64, counts: [(&str, u64); N]) -> RunStep {
    RunStep {
        name: name.into(),
        status: StepStatus::Done,
        ms,
        counts: counts
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        error_kind: None,
        message: None,
        fix: None,
        reused: false,
    }
}

fn skipped(name: &str, why: &str) -> RunStep {
    RunStep {
        status: StepStatus::Skipped,
        message: Some(why.into()),
        ..done(name, 0, [])
    }
}

fn failed(name: &str, started: Instant, e: &crate::error::Error) -> RunStep {
    RunStep {
        status: StepStatus::Failed,
        error_kind: Some(e.kind().into()),
        message: Some(e.reason()),
        fix: e.fix(),
        ..done(name, elapsed(started), [])
    }
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
