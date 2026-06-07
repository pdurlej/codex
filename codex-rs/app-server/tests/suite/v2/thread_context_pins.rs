use anyhow::Context;
use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadContextPinsCreateResponse;
use codex_app_server_protocol::ThreadContextPinsDeleteResponse;
use codex_app_server_protocol::ThreadContextPinsListResponse;
use codex_app_server_protocol::ThreadContextPinsUpdateResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput as V2UserInput;
use codex_state::StateRuntime;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test]
async fn thread_context_pins_crud_round_trips_through_app_server() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let _state_db = init_state_db(codex_home.path()).await?;

    let mut mcp = TestAppServer::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    let thread_id = start_thread(&mut mcp).await?;

    let create_resp =
        send_context_pin_create(&mut mcp, &thread_id, "Remember issue #26889").await?;
    let ThreadContextPinsCreateResponse { pin } =
        to_response::<ThreadContextPinsCreateResponse>(create_resp)?;
    assert_eq!(pin.thread_id, thread_id);
    assert_eq!(pin.text, "Remember issue #26889");
    assert!(!pin.pin_id.is_empty());

    let list_resp = send_context_pin_list(&mut mcp, &thread_id).await?;
    let ThreadContextPinsListResponse { data, next_cursor } =
        to_response::<ThreadContextPinsListResponse>(list_resp)?;
    assert_eq!(next_cursor, None);
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].pin_id, pin.pin_id);
    assert_eq!(data[0].text, "Remember issue #26889");

    let update_resp = send_context_pin_update(
        &mut mcp,
        &thread_id,
        &pin.pin_id,
        "Keep issue #26889 linked",
    )
    .await?;
    let ThreadContextPinsUpdateResponse { pin: updated } =
        to_response::<ThreadContextPinsUpdateResponse>(update_resp)?;
    let updated = updated.context("updated pin should be returned")?;
    assert_eq!(updated.pin_id, pin.pin_id);
    assert_eq!(updated.text, "Keep issue #26889 linked");

    let delete_resp = send_context_pin_delete(&mut mcp, &thread_id, &pin.pin_id).await?;
    let ThreadContextPinsDeleteResponse { deleted } =
        to_response::<ThreadContextPinsDeleteResponse>(delete_resp)?;
    assert!(deleted);

    let list_resp = send_context_pin_list(&mut mcp, &thread_id).await?;
    let ThreadContextPinsListResponse { data, next_cursor } =
        to_response::<ThreadContextPinsListResponse>(list_resp)?;
    assert_eq!(data, Vec::new());
    assert_eq!(next_cursor, None);

    Ok(())
}

#[tokio::test]
async fn thread_context_pins_flow_to_turn_model_input() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let _state_db = init_state_db(codex_home.path()).await?;

    let mut mcp = TestAppServer::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    let thread_id = start_thread(&mut mcp).await?;

    send_context_pin_create(&mut mcp, &thread_id, "Pinned design note").await?;

    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            client_user_message_id: None,
            input: vec![V2UserInput::Text {
                text: "continue".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let requests = server
        .received_requests()
        .await
        .context("failed to fetch received requests")?;
    let request = requests
        .iter()
        .find(|request| request.url.path().ends_with("/responses"))
        .context("expected model request")?;
    let body = request
        .body_json::<Value>()
        .context("request body should be JSON")?;
    let body = body.to_string();
    assert!(body.contains("<external_codex_context_pin_"));
    assert!(body.contains("Pinned design note"));

    Ok(())
}

async fn start_thread(mcp: &mut TestAppServer) -> Result<String> {
    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(response)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/started"),
    )
    .await??;
    Ok(thread.id)
}

async fn send_context_pin_list(
    mcp: &mut TestAppServer,
    thread_id: &str,
) -> Result<JSONRPCResponse> {
    let request_id = mcp
        .send_raw_request(
            "thread/contextPins/list",
            Some(json!({
                "threadId": thread_id,
                "cursor": null,
                "limit": null,
            })),
        )
        .await?;
    read_response(mcp, request_id).await
}

async fn send_context_pin_create(
    mcp: &mut TestAppServer,
    thread_id: &str,
    text: &str,
) -> Result<JSONRPCResponse> {
    let request_id = mcp
        .send_raw_request(
            "thread/contextPins/create",
            Some(json!({
                "threadId": thread_id,
                "text": text,
            })),
        )
        .await?;
    read_response(mcp, request_id).await
}

async fn send_context_pin_update(
    mcp: &mut TestAppServer,
    thread_id: &str,
    pin_id: &str,
    text: &str,
) -> Result<JSONRPCResponse> {
    let request_id = mcp
        .send_raw_request(
            "thread/contextPins/update",
            Some(json!({
                "threadId": thread_id,
                "pinId": pin_id,
                "text": text,
            })),
        )
        .await?;
    read_response(mcp, request_id).await
}

async fn send_context_pin_delete(
    mcp: &mut TestAppServer,
    thread_id: &str,
    pin_id: &str,
) -> Result<JSONRPCResponse> {
    let request_id = mcp
        .send_raw_request(
            "thread/contextPins/delete",
            Some(json!({
                "threadId": thread_id,
                "pinId": pin_id,
            })),
        )
        .await?;
    read_response(mcp, request_id).await
}

async fn read_response(mcp: &mut TestAppServer, request_id: i64) -> Result<JSONRPCResponse> {
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await?
}

async fn init_state_db(codex_home: &Path) -> Result<Arc<StateRuntime>> {
    let state_db = StateRuntime::init(codex_home.to_path_buf(), "mock_provider".into()).await?;
    state_db
        .mark_backfill_complete(/*last_watermark*/ None)
        .await?;
    Ok(state_db)
}

fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"
suppress_unstable_features_warning = true

[features]
sqlite = true

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}
