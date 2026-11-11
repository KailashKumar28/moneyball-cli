//! A refresh on its own thread, for a front-end that must stay
//! responsive: owned events over a channel, and the flag that stops it.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde::Serialize;

use super::{Options, Progress};
use crate::config::AppConfig;
use crate::schema::{RunRecord, RunStep};

/// What the worker tells its front-end, in order: Begin and Done for each
/// step, then exactly one of Finished or CouldNotStart. A channel that
/// closes without either means the worker died.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ev {
    /// A step (one of STEPS) started.
    Begin { step: &'static str },
    /// A step finished: its record.
    Done(Box<RunStep>),
    /// Every step ran or was skipped; the run record is written.
    Finished { run: Box<RunRecord>, ms: u64 },
    /// No workspace, or another refresh holds the lock.
    CouldNotStart { error: String },
}

/// Start a refresh on a worker thread. Returns its events and the flag
/// that stops it after the step in flight (`opts.cancel` when the caller
/// brought one). The Meta token is resolved by the caller, as for `run`.
pub fn spawn(
    cfg: AppConfig,
    meta_token: Option<String>,
    mut opts: Options,
) -> (Receiver<Ev>, Arc<AtomicBool>) {
    let (tx, rx) = std::sync::mpsc::channel::<Ev>();
    let cancel = opts.cancel.get_or_insert_default().clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let step_tx = tx.clone();
        let result = super::run(&cfg, meta_token.as_deref(), &opts, &mut |p| {
            let _ = step_tx.send(match p {
                Progress::Begin(step) => Ev::Begin { step },
                Progress::Done(step) => Ev::Done(Box::new(step.clone())),
            });
        });
        let _ = tx.send(match result {
            Ok(run) => Ev::Finished {
                run: Box::new(run),
                ms: started.elapsed().as_millis() as u64,
            },
            Err(e) => Ev::CouldNotStart { error: e.reason() },
        });
    });
    (rx, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Trigger;

    fn opts() -> Options {
        Options {
            trigger: Trigger::Manual,
            force: false,
            days: 28,
            cancel: None,
        }
    }

    /// A second refresh of one workspace cannot start: one event saying
    /// why, then the channel closes.
    #[test]
    fn a_refresh_that_cannot_start_says_so_once() {
        let root = std::env::temp_dir().join(format!("mb-worker-{}", std::process::id()));
        let cfg = AppConfig {
            data_root: root.clone(),
            date: None,
            workspace: None,
            agent: true,
        };
        let _held = crate::runs::lock(&cfg).unwrap();
        let (rx, cancel) = spawn(cfg, None, opts());
        assert!(!cancel.load(std::sync::atomic::Ordering::SeqCst));
        match rx.recv().unwrap() {
            Ev::CouldNotStart { error } => assert!(error.contains("already running"), "{}", error),
            other => panic!("expected CouldNotStart, got {:?}", other),
        }
        assert!(rx.recv().is_err(), "nothing follows the last event");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn events_serialize_tagged_by_type() {
        let begin = serde_json::to_value(Ev::Begin { step: "meta" }).unwrap();
        assert_eq!(begin, serde_json::json!({"type": "begin", "step": "meta"}));
        let step: RunStep =
            serde_json::from_value(serde_json::json!({"name": "crm", "status": "done", "ms": 7}))
                .unwrap();
        let done = serde_json::to_value(Ev::Done(Box::new(step))).unwrap();
        assert_eq!(done["type"], "done");
        assert_eq!(done["name"], "crm");
        let failed = serde_json::to_value(Ev::CouldNotStart { error: "x".into() }).unwrap();
        assert_eq!(
            failed,
            serde_json::json!({"type": "could_not_start", "error": "x"})
        );
    }
}
