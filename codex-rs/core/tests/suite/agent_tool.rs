#![allow(clippy::unwrap_used)]

use codex_core::protocol::Op;
use core_test_support::load_sse_fixture_with_id;
use core_test_support::responses;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;

fn sse_completed(id: &str) -> String {
    load_sse_fixture_with_id("tests/fixtures/completed_template.json", id)
}

#[allow(clippy::expect_used)]
fn tool_identifiers(body: &serde_json::Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(|v| v.as_str())
                .or_else(|| tool.get("type").and_then(|v| v.as_str()))
                .map(std::string::ToString::to_string)
                .expect("tool should have either name or type")
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sub_agent_inherits_tools() {
    let server = start_mock_server().await;

    // 1. Mock the response for the *parent* invocation (the "general" agent tool call)
    let agent_tool_call_args = serde_json::json!({
        "name": "general",
        "task": "do something",
    })
    .to_string();

    let sse_parent = responses::sse(vec![
        responses::ev_response_created("resp-parent"),
        responses::ev_function_call("call-1", "agent", &agent_tool_call_args),
        responses::ev_completed("resp-parent"),
    ]);

    let mock_parent = responses::mount_sse_once(&server, sse_parent).await;

    // 2. Mock the response for the *sub-agent* execution
    // This will be called by the AgentHandler
    let sse_sub_agent = sse_completed("resp-sub");
    let mock_sub_agent = responses::mount_sse_once(&server, sse_sub_agent).await;

    // 3. Set up the test Codex session
    let mut builder = test_codex().with_model("gpt-5-codex");
    let test = builder
        .build(&server)
        .await
        .expect("create test Codex conversation");

    // 4. Submit the user input to trigger the parent flow
    test.codex
        .submit(Op::UserInput {
            items: vec![codex_protocol::user_input::UserInput::Text {
                text: "use the general agent".to_string(),
            }],
        })
        .await
        .expect("submit parent turn");

    // Wait for the agent tool to finish (this implies the sub-agent ran)
    core_test_support::wait_for_event(&test.codex, |ev| {
        matches!(ev, codex_core::protocol::EventMsg::AgentEnd(_))
    })
    .await;

    let _ = mock_parent; // keep alive until here

    // 5. Verify the sub-agent received the correct tools
    // The *second* request to the mock server corresponds to the sub-agent invocation.
    // We need to wait for the sub-agent to actually make its request.
    // The `submit` call returns when the *parent* turn is processing, but the AgentHandler
    // spawns its own internal turn. Since `submit` waits for the turn to complete, and
    // the agent tool call is part of that turn, it should wait for the agent to finish.

    let sub_agent_request = mock_sub_agent.single_request();
    let sub_agent_body = sub_agent_request.body_json();
    let tools = tool_identifiers(&sub_agent_body);

    // 6. Assertions:
    // - Should contain standard tools (e.g., shell_command, apply_patch)
    // - Should NOT contain "agent" (recursion prevention)
    assert!(
        tools.contains(&"shell_command".to_string()),
        "sub-agent should inherit shell_command"
    );
    assert!(
        tools.contains(&"apply_patch".to_string()),
        "sub-agent should inherit apply_patch"
    );
    assert!(
        !tools.contains(&"agent".to_string()),
        "sub-agent should NOT have the agent tool (recursion prevention)"
    );
}
