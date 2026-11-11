//! Agent loop tests, hermetic through the StreamFn seam. Split from agent.rs (size cap).

use super::*;

/// Scripted stream: pops the next TurnResponse per request; errors
/// when the script runs dry.
#[allow(clippy::type_complexity)]
fn scripted(
    responses: std::cell::RefCell<Vec<TurnResponse>>,
) -> impl Fn(&[Item], &mut dyn FnMut(&str)) -> crate::error::Result<TurnResponse> {
    move |_h, _d| {
        responses
            .borrow_mut()
            .pop()
            .ok_or_else(|| crate::error::Error::Llm("script exhausted".into()))
    }
}

struct FakeExec {
    /// Err = the test asserts the tool must NOT run.
    allow: bool,
}
impl ToolExec for FakeExec {
    fn run(&self, name: &str, _args: &Value) -> std::result::Result<String, String> {
        assert!(
            self.allow,
            "tool {} must not execute in this scenario",
            name
        );
        Ok(format!("{} output", name))
    }
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

fn drain(rx: std::sync::mpsc::Receiver<Ev>) -> Vec<Ev> {
    rx.try_iter().collect()
}

#[test]
fn loop_runs_tools_and_emits_items_as_they_complete() {
    // Script (popped back-to-front): tool round, then final text.
    let script = std::cell::RefCell::new(vec![
        TurnResponse {
            text: "answer".into(),
            usage: Usage {
                input: 300,
                output: 20,
            },
            ..Default::default()
        },
        TurnResponse {
            tool_calls: vec![call("c1", "brief")],
            usage: Usage {
                input: 200,
                output: 10,
            },
            ..Default::default()
        },
    ]);
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    run_turn_with(
        &scripted(script),
        "test",
        vec![Item::User { text: "q".into() }],
        &FakeExec { allow: true },
        &cancel,
        &tx,
    );
    let evs = drain(rx);
    // The turn's size: the last call's context, both calls' output.
    assert!(matches!(evs.last(), Some(Ev::TurnComplete { usage, .. })
            if *usage == Usage { input: 300, output: 30 }));
    // Items emitted in order, each the moment it completed.
    let items: Vec<&Item> = evs
        .iter()
        .filter_map(|e| match e {
            Ev::ItemDone { item } => Some(item),
            _ => None,
        })
        .collect();
    assert!(matches!(items[0], Item::ToolCall { call_id, .. } if call_id == "c1"));
    assert!(
        matches!(items[1], Item::ToolOutput { call_id, output, is_error: false }
            if call_id == "c1" && output == "brief output")
    );
    assert!(matches!(items[2], Item::Assistant { text } if text == "answer"));
    assert!(matches!(evs.last(), Some(Ev::TurnComplete { .. })));
}

#[test]
fn length_stop_fails_tool_calls_without_executing() {
    let script = std::cell::RefCell::new(vec![
        TurnResponse {
            text: "retry answer".into(),
            ..Default::default()
        },
        TurnResponse {
            tool_calls: vec![call("c1", "funnel")],
            truncated: true,
            ..Default::default()
        },
    ]);
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    // FakeExec{allow:false} panics if the truncated call executes.
    run_turn_with(
        &scripted(script),
        "test",
        vec![Item::User { text: "q".into() }],
        &FakeExec { allow: false },
        &cancel,
        &tx,
    );
    let evs = drain(rx);
    let out = evs
        .iter()
        .find_map(|e| match e {
            Ev::ItemDone {
                item: Item::ToolOutput {
                    output, is_error, ..
                },
            } => Some((output.clone(), *is_error)),
            _ => None,
        })
        .expect("a ToolOutput item");
    assert!(out.1, "truncated call must be an error output");
    assert!(out.0.contains("max_tokens"));
    assert!(matches!(evs.last(), Some(Ev::TurnComplete { .. })));
}

#[test]
fn cancel_mid_stream_emits_marker_and_aborts() {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    // Stream "interrupted": sets the flag and errors, like a
    // dropped SSE stream after Esc.
    let stream = move |_h: &[Item], _d: &mut dyn FnMut(&str)| {
        c2.store(true, Ordering::SeqCst);
        Err(crate::error::Error::Llm("interrupted".into()))
    };
    run_turn_with(
        &stream,
        "test",
        vec![Item::User { text: "q".into() }],
        &FakeExec { allow: true },
        &cancel,
        &tx,
    );
    let evs = drain(rx);
    assert!(evs.iter().any(|e| matches!(
        e,
        Ev::ItemDone { item: Item::User { text } } if text.starts_with("<turn_aborted>")
    )));
    assert!(matches!(evs.last(), Some(Ev::TurnAborted)));
}

#[test]
fn stream_error_without_cancel_is_failed() {
    let script = std::cell::RefCell::new(vec![]);
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    run_turn_with(
        &scripted(script),
        "test",
        vec![Item::User { text: "q".into() }],
        &FakeExec { allow: true },
        &cancel,
        &tx,
    );
    let evs = drain(rx);
    assert!(matches!(evs.last(), Some(Ev::Failed { .. })));
    // A failed request emits no items.
    assert!(!evs.iter().any(|e| matches!(e, Ev::ItemDone { .. })));
}

#[test]
fn heal_inserts_aborted_outputs_after_dangling_calls() {
    let mut h = vec![
        Item::User { text: "q".into() },
        Item::ToolCall {
            call_id: "c1".into(),
            name: "brief".into(),
            args: Value::Null,
        },
    ];
    heal_history(&mut h);
    assert_eq!(h.len(), 3);
    match &h[2] {
        Item::ToolOutput {
            call_id, is_error, ..
        } => {
            assert_eq!(call_id, "c1");
            assert!(is_error);
        }
        other => panic!("expected ToolOutput, got {:?}", other),
    }
    // Idempotent.
    heal_history(&mut h);
    assert_eq!(h.len(), 3);
}

#[test]
fn truncate_output_caps_lines_with_hint() {
    let big = "x\n".repeat(3000);
    let t = truncate_output(&big);
    assert!(t.lines().count() <= MAX_OUTPUT_LINES + 1);
    assert!(t.contains("[truncated: kept first 2000 of 3000 lines]"));
    assert_eq!(truncate_output("small"), "small");
}

#[test]
fn item_serde_round_trips() {
    let items = vec![
        Item::User { text: "hi".into() },
        Item::ToolCall {
            call_id: "c".into(),
            name: "funnel".into(),
            args: serde_json::json!({"product": "P"}),
        },
        Item::ToolOutput {
            call_id: "c".into(),
            output: "table".into(),
            is_error: false,
        },
        Item::Assistant { text: "ans".into() },
    ];
    for i in &items {
        let s = serde_json::to_string(i).unwrap();
        let back: Item = serde_json::from_str(&s).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            s,
            "round trip changed {:?}",
            i
        );
    }
}
