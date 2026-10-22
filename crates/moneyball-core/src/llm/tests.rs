//! llm tests: wire bodies, the stream parsers, auth errors, requests,
//! and the agent-turn stream. Split from llm.rs (size cap).

use super::*;

#[test]
fn responses_body_uses_instructions_and_input() {
    let v = build_responses_body(
        "gpt-5",
        Some("you are a moneyball advisor"),
        "show me namma",
    );
    assert_eq!(v["model"], "gpt-5");
    assert_eq!(v["instructions"], "you are a moneyball advisor");
    assert_eq!(v["input"], "show me namma");
}

#[test]
fn chat_body_separates_system_message() {
    let v = build_chat_body("gpt-oss", Some("be terse"), "hello");
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "be terse");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "hello");
}

#[test]
fn chat_body_no_system() {
    let v = build_chat_body("gpt-oss", None, "hello");
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
}

#[test]
fn messages_body_top_level_system_and_max_tokens() {
    let v = build_messages_body("claude-sonnet-5", Some("be brief"), "hi");
    assert_eq!(v["model"], "claude-sonnet-5");
    assert_eq!(v["system"], "be brief");
    assert_eq!(v["max_tokens"], DEFAULT_MAX_TOKENS);
    assert_eq!(v["messages"][0]["role"], "user");
}

// ---------- tool-calling body builders ----------

// ---------- tool-calling response parsers ----------

// ---------- streaming delta extraction ----------

#[test]
fn stream_delta_messages_wire() {
    let v: Value = serde_json::from_str(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
    )
    .unwrap();
    assert_eq!(
        extract_stream_delta(WireApi::Messages, &v).as_deref(),
        Some("Hel")
    );
    // Non-delta events yield nothing.
    let v2: Value = serde_json::from_str(r#"{"type":"message_start"}"#).unwrap();
    assert!(extract_stream_delta(WireApi::Messages, &v2).is_none());
}

#[test]
fn stream_delta_responses_wire() {
    let v: Value =
        serde_json::from_str(r#"{"type":"response.output_text.delta","delta":"world"}"#).unwrap();
    assert_eq!(
        extract_stream_delta(WireApi::Responses, &v).as_deref(),
        Some("world")
    );
    let v2: Value = serde_json::from_str(r#"{"type":"response.completed"}"#).unwrap();
    assert!(extract_stream_delta(WireApi::Responses, &v2).is_none());
}

#[test]
fn status_error_keeps_llm_auth_contract() {
    assert!(matches!(
        status_error("p", reqwest::StatusCode::UNAUTHORIZED, "no"),
        Error::LlmAuth(_)
    ));
    assert!(matches!(
        status_error("p", reqwest::StatusCode::FORBIDDEN, "no"),
        Error::LlmAuth(_)
    ));
    assert!(matches!(
        status_error("p", reqwest::StatusCode::INTERNAL_SERVER_ERROR, "boom"),
        Error::Llm(_)
    ));
}

#[test]
fn each_wire_reports_its_usage() {
    use crate::agent::Usage;
    let mut u = Usage::default();
    for ev in [
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 100, "cache_read_input_tokens": 900}}}),
        json!({"type": "message_delta", "usage": {"output_tokens": 42}}),
    ] {
        usage_in(WireApi::Messages, &ev, &mut u);
    }
    assert_eq!(
        u,
        Usage {
            input: 1000,
            output: 42
        }
    );
    // MiniMax: input arrives with the delta, cache reads too.
    let mut u = Usage::default();
    for ev in [
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 0, "output_tokens": 0}}}),
        json!({"type": "message_delta", "usage": {"input_tokens": 46, "output_tokens": 5, "cache_read_input_tokens": 128}}),
    ] {
        usage_in(WireApi::Messages, &ev, &mut u);
    }
    assert_eq!(
        u,
        Usage {
            input: 174,
            output: 5
        }
    );
    let mut u = Usage::default();
    usage_in(
        WireApi::ChatCompletions,
        &json!({"choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 3}}),
        &mut u,
    );
    assert_eq!(
        u,
        Usage {
            input: 7,
            output: 3
        }
    );
    let mut u = Usage::default();
    usage_in(
        WireApi::Responses,
        &json!({"type": "response.completed", "response": {"usage": {"input_tokens": 5, "output_tokens": 1}}}),
        &mut u,
    );
    assert_eq!(
        u,
        Usage {
            input: 5,
            output: 1
        }
    );
}

mod turn_tests {
    use super::*;

    fn hist() -> Vec<Item> {
        vec![
            Item::User { text: "q".into() },
            Item::ToolCall {
                call_id: "c1".into(),
                name: "funnel".into(),
                args: json!({"product": "P"}),
            },
            Item::ToolOutput {
                call_id: "c1".into(),
                output: "table".into(),
                is_error: false,
            },
            Item::Assistant { text: "ans".into() },
        ]
    }

    #[test]
    fn messages_history_coalesces_roles_and_maps_tool_blocks() {
        let m = messages_history(&hist());
        // user(q) | assistant(tool_use) | user(tool_result) | assistant(text)
        assert_eq!(m.len(), 4);
        assert_eq!(m[1]["content"][0]["type"], "tool_use");
        assert_eq!(m[1]["content"][0]["input"]["product"], "P");
        assert_eq!(m[2]["content"][0]["type"], "tool_result");
        assert_eq!(m[2]["content"][0]["tool_use_id"], "c1");
    }

    #[test]
    fn chat_history_uses_tool_role_and_stringified_args() {
        let m = chat_history("sys", &hist());
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "funnel");
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[3]["tool_call_id"], "c1");
    }

    #[test]
    fn responses_history_emits_typed_items() {
        let m = responses_history(&hist());
        assert_eq!(m[1]["type"], "function_call");
        assert_eq!(m[2]["type"], "function_call_output");
    }

    #[test]
    fn acc_assembles_messages_wire_tool_call_from_deltas() {
        let mut acc = ToolCallAcc::default();
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_start","index":1,
                "content_block":{"type":"tool_use","id":"c9","name":"brief"}}),
        );
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_delta","index":1,
                "delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}),
        );
        acc.feed(
            WireApi::Messages,
            &json!({"type":"content_block_delta","index":1,
                "delta":{"type":"input_json_delta","partial_json":"1}"}}),
        );
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c9");
        assert_eq!(calls[0].arguments["a"], 1);
    }

    #[test]
    fn bad_args_become_empty_object_not_panic() {
        assert_eq!(parse_args("{broken"), json!({}));
        assert_eq!(parse_args(""), json!({}));
    }

    #[test]
    fn a_call_carries_the_providers_query_params_and_auth() {
        let p = ModelProviderInfo {
            base_url: "https://x.openai.azure.com/openai".into(),
            wire_api: WireApi::ChatCompletions,
            query_params: Some(std::collections::HashMap::from([(
                "api-version".into(),
                "2025-01-01".into(),
            )])),
            ..Default::default()
        };
        let req = request(&reqwest::blocking::Client::new(), &p, "k")
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://x.openai.azure.com/openai/chat/completions?api-version=2025-01-01"
        );
        assert_eq!(req.headers()["authorization"], "Bearer k");
    }
}
