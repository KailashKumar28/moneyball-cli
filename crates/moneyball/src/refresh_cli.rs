//! `moneyball refresh`: the daily refresh from the shell, and what the
//! schedule runs. One plain line per step as it finishes, then the
//! outcome, the fix and where things landed. Exit code 0 ok, 2 degraded,
//! 1 failed; with MB_AGENT=1 only the run record, as JSON.

use anyhow::Result;

use moneyball_core::refresh::{self, Options};
use moneyball_core::schema::{Outcome, StepStatus, Trigger};
use moneyball_core::AppConfig;

pub(crate) fn run(cfg: &AppConfig, force: bool, days: u32, scheduled: bool) -> Result<()> {
    let token = moneyball_core::secrets::load_meta_token();
    if !cfg.agent {
        println!(
            "refresh for {}  (workspace {})",
            refresh::data_day_label(),
            cfg.data_root.display()
        );
    }
    let opts = Options {
        trigger: if scheduled {
            Trigger::Schedule
        } else {
            Trigger::Manual
        },
        force,
        days,
        cancel: None,
    };
    let agent = cfg.agent;
    let mut n = 0;
    let run = refresh::run(cfg, token.as_deref(), &opts, &mut |p| {
        if let refresh::Progress::Done(step) = p {
            n += 1;
            if !agent {
                println!("{}", refresh::step_line(n, step));
            }
        }
    })?;

    if agent {
        println!("{}", serde_json::to_string(&run)?);
    } else {
        let failed: Vec<&str> = run
            .steps
            .iter()
            .filter(|s| matches!(s.status, StepStatus::Failed | StepStatus::Partial))
            .map(|s| s.name.as_str())
            .collect();
        let outcome = match run.outcome {
            Outcome::Ok => "ok".to_string(),
            o => format!(
                "{} ({})",
                if o == Outcome::Failed {
                    "failed"
                } else {
                    "degraded"
                },
                failed.join(", ")
            ),
        };
        println!("outcome: {}", outcome);
        for step in &run.steps {
            if let Some(fix) = &step.fix {
                println!(
                    "fix: {}   then: moneyball refresh",
                    moneyball_core::connections::fix_in_shell(fix)
                );
            }
        }
        if let Some(report) = run.artifacts.get("report") {
            println!("report: {}", report);
        }
        println!(
            "run record: {}",
            cfg.runs_dir().join(format!("{}.json", run.date)).display()
        );
    }
    match run.outcome {
        Outcome::Ok => Ok(()),
        Outcome::Degraded => std::process::exit(2),
        _ => std::process::exit(1),
    }
}
