//! The transcript as a front-end shows it: one projection from the wire
//! history to view items, shared by every front-end's reopen/replay so
//! they hide and pair the same things.

use serde::Serialize;
use serde_json::Value;

use super::Item;

/// One visible row of a conversation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewItem {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    /// A tool call joined with its output (by call_id).
    Tool {
        call_id: String,
        name: String,
        args: Value,
        status: ToolStatus,
        /// Empty while `status` is pending.
        output: String,
    },
    /// The interrupt marker: the turn before it ended early. Shown as
    /// a note, never as something the user said.
    TurnInterrupted,
    /// The bug-report marker: the user flagged the answer above it.
    BugReportFiled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// No output recorded: still running in a live turn, cut off by an
    /// interrupt in a saved one.
    Pending,
    Ok,
    Failed,
}

/// Project the wire history into what a transcript shows. Markers we
/// author into the history for the model (`TURN_ABORTED_MARKER`, the
/// bug-report marker) come out as notes; each tool call and its output
/// become one row. An output whose call is missing (a session line that
/// no longer parses) still shows, under the name "tool".
pub fn view_items(history: &[Item]) -> Vec<ViewItem> {
    let mut out: Vec<ViewItem> = Vec::new();
    for item in history {
        match item {
            Item::User { text } if text.starts_with("<turn_aborted>") => {
                out.push(ViewItem::TurnInterrupted)
            }
            Item::User { text } if crate::debug::is_bug_report_marker(text) => {
                out.push(ViewItem::BugReportFiled)
            }
            Item::User { text } => out.push(ViewItem::User { text: text.clone() }),
            Item::Assistant { text } => out.push(ViewItem::Assistant { text: text.clone() }),
            Item::ToolCall {
                call_id,
                name,
                args,
            } => out.push(ViewItem::Tool {
                call_id: call_id.clone(),
                name: name.clone(),
                args: args.clone(),
                status: ToolStatus::Pending,
                output: String::new(),
            }),
            Item::ToolOutput {
                call_id,
                output,
                is_error,
            } => answer(&mut out, call_id, output, *is_error),
        }
    }
    out
}

/// Fill in the pending row of `id` with its output.
fn answer(out: &mut Vec<ViewItem>, id: &str, text: &str, is_error: bool) {
    let done = if is_error {
        ToolStatus::Failed
    } else {
        ToolStatus::Ok
    };
    let row = out.iter_mut().rev().find_map(|v| match v {
        ViewItem::Tool {
            call_id,
            status,
            output,
            ..
        } if call_id == id && *status == ToolStatus::Pending => Some((status, output)),
        _ => None,
    });
    match row {
        Some((status, output)) => {
            *status = done;
            *output = text.to_string();
        }
        None => out.push(ViewItem::Tool {
            call_id: id.to_string(),
            name: "tool".into(),
            args: Value::Object(Default::default()),
            status: done,
            output: text.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str) -> Item {
        Item::ToolCall {
            call_id: id.into(),
            name: name.into(),
            args: json!({"product": "Namma Mane"}),
        }
    }

    fn output(id: &str, text: &str, is_error: bool) -> Item {
        Item::ToolOutput {
            call_id: id.into(),
            output: text.into(),
            is_error,
        }
    }

    #[test]
    fn a_call_and_its_output_are_one_row() {
        let v = view_items(&[
            Item::User { text: "hi".into() },
            call("a", "funnel"),
            call("b", "diagnose"),
            output("b", "boom", true),
            output("a", "table", false),
            Item::Assistant { text: "ok".into() },
        ]);
        assert_eq!(v.len(), 4);
        assert!(
            matches!(&v[1], ViewItem::Tool { name, status: ToolStatus::Ok, output, .. }
            if name == "funnel" && output == "table")
        );
        assert!(
            matches!(&v[2], ViewItem::Tool { name, status: ToolStatus::Failed, output, .. }
            if name == "diagnose" && output == "boom")
        );
    }

    /// The model-facing markers are user-role items, but never something
    /// the user said; an interrupted call stays pending.
    #[test]
    fn markers_become_notes_and_a_cut_call_stays_pending() {
        let v = view_items(&[
            call("a", "brief"),
            Item::User {
                text: super::super::TURN_ABORTED_MARKER.into(),
            },
            Item::User {
                text: crate::debug::marker_text("bad math"),
            },
        ]);
        assert!(matches!(
            &v[0],
            ViewItem::Tool {
                status: ToolStatus::Pending,
                ..
            }
        ));
        assert_eq!(
            v[1..],
            [ViewItem::TurnInterrupted, ViewItem::BugReportFiled]
        );
    }

    #[test]
    fn an_output_without_its_call_still_shows() {
        let v = view_items(&[output("gone", "numbers", false)]);
        assert!(
            matches!(&v[0], ViewItem::Tool { name, status: ToolStatus::Ok, .. }
            if name == "tool")
        );
    }

    /// The IPC shape: tagged like `Item`, snake_case throughout.
    #[test]
    fn serializes_tagged_snake_case() {
        let v = view_items(&[call("a", "brief"), output("a", "x", false)]);
        assert_eq!(
            serde_json::to_value(&v[0]).unwrap(),
            json!({"type": "tool", "call_id": "a", "name": "brief",
                   "args": {"product": "Namma Mane"}, "status": "ok", "output": "x"})
        );
        assert_eq!(
            serde_json::to_value(ViewItem::TurnInterrupted).unwrap(),
            json!({"type": "turn_interrupted"})
        );
    }
}
