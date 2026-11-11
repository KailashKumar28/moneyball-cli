//! The agent and /report worker's events, applied to the app each tick:
//! streamed text, tool cells, items persisted as they complete, and how
//! a turn ends. Split from event.rs (size cap).

use crate::*;

/// Drain any pending worker events (LLM stream deltas, /report) into the
/// app. Runs every tick, so text appears as it arrives (~10 redraws/sec).
/// The receiver borrow is released before each event is handled, since a
/// handler may start a NEW stream.
pub(crate) fn drain(app: &mut App) {
    use std::sync::mpsc::TryRecvError;
    loop {
        let ev = {
            let Some(rx) = &app.stream else { return };
            rx.try_recv()
        };
        match ev {
            Ok(StreamEvent::Agent(aev)) => {
                use moneyball_core::agent::Ev;
                // The session records each item the moment it is final
                // (codex's rollout rule: a crash or quit mid-turn loses
                // at most the item in flight) and ends the turn on a
                // final event.
                if let Err(e) = app.session.apply(&aev) {
                    app.status = Some(e.to_string());
                }
                match aev {
                    Ev::AssistantDelta { text } => app.chat.append_assistant(&text),
                    Ev::AssistantDone { .. } => app.chat.finish_streaming(),
                    Ev::ToolBegin {
                        call_id,
                        name,
                        args,
                    } => {
                        app.chat.finish_streaming();
                        app.running_tool = Some((call_id, name.clone(), std::time::Instant::now()));
                        app.chat.push(chat::Cell::ToolCall(chat::cells::ToolCall {
                            name,
                            args: crate::app::compact_args(&args),
                            status: chat::cells::ToolStatus::Running,
                        }));
                    }
                    Ev::ToolEnd {
                        call_id,
                        output,
                        ok,
                    } => {
                        // Finalize the Running cell (6b contract) and
                        // carry the real name + elapsed time into the
                        // result instead of "tool"/0ms.
                        app.chat.finalize_tool(ok);
                        let (name, ms) = match app.running_tool.take() {
                            Some((id, name, started)) if id == call_id => {
                                // Sub-ms tools round up so a measured
                                // duration is never rendered as "none".
                                (name, (started.elapsed().as_millis() as u64).max(1))
                            }
                            other => {
                                app.running_tool = other;
                                ("tool".into(), 0)
                            }
                        };
                        app.chat
                            .push(chat::Cell::ToolResult(chat::cells::ToolResult {
                                name,
                                output: output.lines().map(String::from).collect(),
                                success: ok,
                                duration_ms: ms,
                            }));
                    }
                    Ev::ItemDone { .. } => {}
                    Ev::TurnComplete {
                        ms,
                        provider,
                        usage,
                    } => {
                        app.chat.finish_streaming();
                        // Latency, size and provider are status metadata,
                        // never part of the persisted message text.
                        // Each part only when the provider reported it.
                        let commas = moneyball_core::text::commas;
                        let parts: Vec<String> = [
                            moneyball_core::refresh::duration(ms),
                            match usage.input {
                                0 => String::new(),
                                n => format!("context {} tokens", commas(n)),
                            },
                            match usage.output {
                                0 => String::new(),
                                n => format!("{} out", commas(n)),
                            },
                            format!("via {}", provider),
                        ]
                        .into_iter()
                        .filter(|p| !p.is_empty())
                        .collect();
                        app.status = Some(parts.join("  \u{00B7}  "));
                        app.stream = None;
                        return;
                    }
                    Ev::TurnAborted => {
                        app.chat.finish_streaming();
                        app.chat.push(chat::Cell::System(chat::cells::System(
                            "(turn interrupted)".into(),
                        )));
                        app.stream = None;
                        return;
                    }
                    Ev::Failed { error } => {
                        // Context overflow gets an actionable hint -
                        // without it every later turn fails the same
                        // way and the session looks bricked.
                        let hint = if moneyball_core::agent::looks_like_context_overflow(&error) {
                            "\nthe conversation may have outgrown the model's context \
                             window - /clear starts a fresh session (the old one stays \
                             on disk)"
                        } else {
                            ""
                        };
                        app.chat
                            .append_assistant(&format!("llm call failed: {}{}", error, hint));
                        app.chat.finish_streaming();
                        app.stream = None;
                        return;
                    }
                }
            }
            Ok(StreamEvent::ReportDone { out, ms }) => {
                app.stream = None;
                crate::creative_flow::on_report_done(app, out, ms);
                return;
            }
            Ok(StreamEvent::ReportFailed { err, ms }) => {
                app.stream = None;
                crate::creative_flow::on_report_failed(app, err, ms);
                return;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // The worker died without a final event (panic in a
                // tool or the forwarder). Never a silent dead turn:
                // say so, and release the turn so Esc/submit work.
                app.chat.finish_streaming();
                if app.session.turn_active() {
                    app.chat.push(chat::Cell::System(chat::cells::System(
                        "agent worker stopped unexpectedly - resend your message".into(),
                    )));
                }
                app.session.stream_closed();
                app.stream = None;
                return;
            }
        }
    }
}
