//! `moneyball refresh`: the daily refresh from the shell, and what the
//! schedule runs. One plain line per step as it finishes, then the
//! outcome, the fix and where things landed. Exit code 0 ok, 2 degraded,
//! 1 failed; with MB_AGENT=1 only the run record, as JSON.

use anyhow::Result;

use moneyball_core::refresh::{self, Options};
use moneyball_core::schema::{Outcome, RunStep, StepStatus, Trigger};
use moneyball_core::AppConfig;

pub(crate) fn run(cfg: &AppConfig, force: bool, days: u32) -> Result<()> {
    let token = moneyball_core::secrets::load_meta_token();
    if !cfg.agent {
        println!(
            "refresh for {}  (workspace {})",
            refresh::data_day_label(),
            cfg.data_root.display()
        );
    }
    let opts = Options {
        trigger: Trigger::Manual,
        force,
        days,
    };
    let agent = cfg.agent;
    let mut n = 0;
    let run = refresh::run(cfg, token.as_deref(), &opts, &mut |step| {
        n += 1;
        if !agent {
            println!("{}", step_line(n, step));
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
                println!("fix: {}   then: moneyball refresh", fix);
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

/// `  1 Meta insights        done     5 products, 1,104 rows      18.2s`
fn step_line(n: usize, s: &RunStep) -> String {
    let label = match s.name.as_str() {
        "meta" => "Meta insights",
        "assets" => "creatives and leads",
        "crm" => "CRM",
        "report" => "daily report",
        other => other,
    };
    let status = match s.status {
        StepStatus::Done => "done",
        StepStatus::Partial => "partial",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
    };
    let counts = s
        .counts
        .iter()
        .map(|(k, v)| format!("{} {}", commas(*v), k))
        .collect::<Vec<_>>()
        .join(", ");
    // A step that went wrong gets its whole reason on the next line; a
    // reason cut short is exactly the part the user needed.
    let trouble = matches!(s.status, StepStatus::Failed | StepStatus::Partial);
    let detail = match (&s.message, counts.is_empty()) {
        (Some(m), true) if !trouble => m.clone(),
        (Some(m), false) if !trouble => format!("{}; {}", counts, m),
        _ => counts,
    };
    let time = if s.ms > 0 {
        format!("{:.1}s", s.ms as f64 / 1000.0)
    } else {
        String::new()
    };
    let line = format!(
        "  {} {:<20} {:<8} {:<70} {}",
        n,
        label,
        status,
        moneyball_core::text::truncate_marked(&detail, 70, "..."),
        time
    );
    let line = line.trim_end().to_string();
    match (&s.message, trouble) {
        (Some(m), true) => format!("{}\n      {}", line, m),
        _ => line,
    }
}

fn commas(n: u64) -> String {
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
