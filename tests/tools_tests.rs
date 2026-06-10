//! End-to-end MCP tests: a real rmcp client talks to the server over an
//! in-memory duplex transport while the Vikunja API is mocked with wiremock.
//! This exercises tool registration, JSON schema generation, argument
//! validation, tool-to-client mapping, structured output and resources.

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use common::test_client;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams, ResourceContents};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

type McpClient = RunningService<RoleClient, ()>;

/// Boots the MCP server against `vikunja_url` and connects a client to it
/// over an in-memory duplex stream.
async fn connect(vikunja_url: &str) -> McpClient {
    let server = vikunja_rust_mcp::mcp::VikunjaMcpServer::new(test_client(vikunja_url));
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(service) = rmcp::serve_server(server, server_io).await {
            let _ = service.waiting().await;
        }
    });
    ().serve(client_io).await.expect("client should connect")
}

async fn call(
    client: &McpClient,
    tool: &str,
    args: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, ServiceError> {
    let arguments = match args {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    };
    let mut params = CallToolRequestParams::new(tool.to_string());
    params.arguments = arguments;
    client.call_tool(params).await
}

fn structured(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    result
        .structured_content
        .clone()
        .expect("tool should return structured content")
}

const EXPECTED_TOOLS: &[&str] = &[
    "vikunja_projects_list",
    "vikunja_projects_get",
    "vikunja_projects_create",
    "vikunja_projects_update",
    "vikunja_projects_delete",
    "vikunja_tasks_list",
    "vikunja_tasks_get",
    "vikunja_tasks_create",
    "vikunja_tasks_update",
    "vikunja_tasks_delete",
    "vikunja_tasks_complete",
    "vikunja_tasks_reopen",
    "vikunja_tasks_assign",
    "vikunja_tasks_unassign",
    "vikunja_tasks_bulk_complete",
    "vikunja_tasks_bulk_reopen",
    "vikunja_tasks_bulk_update",
    "vikunja_tasks_bulk_move",
    "vikunja_tasks_bulk_assign",
    "vikunja_tasks_bulk_unassign",
    "vikunja_task_labels_bulk_add",
    "vikunja_task_labels_bulk_remove",
    "vikunja_labels_list",
    "vikunja_labels_create",
    "vikunja_labels_update",
    "vikunja_labels_delete",
    "vikunja_task_labels_add",
    "vikunja_task_labels_remove",
    "vikunja_task_comments_list",
    "vikunja_task_comments_create",
    "vikunja_task_comments_update",
    "vikunja_task_comments_delete",
    "vikunja_task_attachments_list",
    "vikunja_task_attachments_upload",
    "vikunja_task_attachments_download",
    "vikunja_task_attachments_delete",
    "vikunja_users_search",
    "vikunja_teams_list",
];

#[tokio::test]
async fn all_tools_are_registered_with_schemas() {
    let client = connect("http://127.0.0.1:1").await;
    let tools = client.list_all_tools().await.unwrap();

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in EXPECTED_TOOLS {
        assert!(names.contains(expected), "missing tool {expected}");
    }
    assert_eq!(tools.len(), EXPECTED_TOOLS.len());

    for tool in &tools {
        assert!(
            tool.description.as_ref().is_some_and(|d| !d.is_empty()),
            "tool {} has no description",
            tool.name
        );
        assert!(
            !tool.input_schema.is_empty(),
            "tool {} has no input schema",
            tool.name
        );
        assert!(
            tool.output_schema.is_some(),
            "tool {} has no output schema",
            tool.name
        );
    }

    // Spot-check annotations.
    let list_tool = tools
        .iter()
        .find(|t| t.name == "vikunja_projects_list")
        .unwrap();
    assert_eq!(
        list_tool
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint),
        Some(true)
    );
    let delete_tool = tools
        .iter()
        .find(|t| t.name == "vikunja_projects_delete")
        .unwrap();
    assert_eq!(
        delete_tool
            .annotations
            .as_ref()
            .and_then(|a| a.destructive_hint),
        Some(true)
    );
    for name in [
        "vikunja_tasks_bulk_complete",
        "vikunja_tasks_bulk_reopen",
        "vikunja_tasks_bulk_update",
        "vikunja_tasks_bulk_move",
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert_eq!(
            tool.annotations.as_ref().and_then(|a| a.idempotent_hint),
            Some(true),
            "{name} should be marked idempotent"
        );
    }
    for name in [
        "vikunja_task_labels_bulk_remove",
        "vikunja_tasks_bulk_unassign",
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert_eq!(
            tool.annotations.as_ref().and_then(|a| a.destructive_hint),
            Some(true),
            "{name} should be marked destructive"
        );
    }
    for name in ["vikunja_task_labels_bulk_add", "vikunja_tasks_bulk_assign"] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert_ne!(
            tool.annotations.as_ref().and_then(|a| a.destructive_hint),
            Some(true),
            "{name} should not be marked destructive"
        );
        assert_ne!(
            tool.annotations.as_ref().and_then(|a| a.read_only_hint),
            Some(true),
            "{name} should not be marked read-only"
        );
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn projects_list_returns_structured_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .and(query_param("s", "inbox"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "title": "Inbox"}]))
                .insert_header("x-pagination-total-pages", "2")
                .insert_header("x-pagination-result-count", "1"),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_projects_list", json!({"search": "inbox"}))
        .await
        .unwrap();

    let body = structured(&result);
    assert_eq!(body["projects"][0]["title"], "Inbox");
    assert_eq!(body["pagination"]["total_pages"], 2);
    assert_eq!(body["pagination"]["has_more"], true);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_create_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects/3/tasks"))
        .and(body_json(json!({"title": "Write tests", "priority": 3})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 50, "title": "Write tests", "project_id": 3, "done": false, "priority": 3
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_create",
        json!({"project_id": 3, "title": "Write tests", "priority": 3}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["id"], 50);
    assert_eq!(body["priority"], 3);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_complete_marks_done() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Task", "done": false, "project_id": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Task", "done": true, "project_id": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Task", "done": true, "project_id": 3,
            "done_at": "2026-06-09T10:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_tasks_complete", json!({"task_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["done"], true);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn invalid_ids_are_rejected_before_any_request() {
    let client = connect("http://127.0.0.1:1").await;

    let err = call(&client, "vikunja_tasks_get", json!({"task_id": -1}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("task_id"));

    let err = call(&client, "vikunja_projects_get", json!({"project_id": 0}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("project_id"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn missing_required_arguments_fail_schema_validation() {
    let client = connect("http://127.0.0.1:1").await;
    // title is required for project creation.
    let err = call(&client, "vikunja_projects_create", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::McpError(_)), "got {err:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn blank_title_is_rejected() {
    let client = connect("http://127.0.0.1:1").await;
    let err = call(&client, "vikunja_projects_create", json!({"title": "  "}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("title"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn zero_page_is_rejected() {
    let client = connect("http://127.0.0.1:1").await;
    let err = call(&client, "vikunja_tasks_list", json!({"page": 0}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("page"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn api_errors_surface_with_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/404"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": 4002, "message": "The task does not exist."
        })))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let err = call(&client, "vikunja_tasks_get", json!({"task_id": 404}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("does not exist"));
    let details = data.data.expect("error data");
    assert_eq!(details["http_status"], 404);
    assert_eq!(details["vikunja_error_code"], 4002);
    assert_eq!(details["endpoint"], "tasks.get");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn labels_add_and_remove_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/labels"))
        .and(body_json(json!({"label_id": 2})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"label_id": 2})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/labels/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "removed"})))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let added = call(
        &client,
        "vikunja_task_labels_add",
        json!({"task_id": 9, "label_id": 2}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&added)["ok"], true);

    let removed = call(
        &client,
        "vikunja_task_labels_remove",
        json!({"task_id": 9, "label_id": 2}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&removed)["message"], "removed");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn assign_and_unassign_users() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/assignees"))
        .and(body_json(json!({"user_id": 3})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"user_id": 3})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/assignees/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "unassigned"})))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let assigned = call(
        &client,
        "vikunja_tasks_assign",
        json!({"task_id": 9, "user_id": 3}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&assigned)["ok"], true);

    let unassigned = call(
        &client,
        "vikunja_tasks_unassign",
        json!({"task_id": 9, "user_id": 3}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&unassigned)["message"], "unassigned");

    client.cancel().await.unwrap();
}

// ----- bulk task operations ---------------------------------------------------

#[tokio::test]
async fn bulk_complete_succeeds_for_all_tasks() {
    let server = MockServer::start().await;
    for id in [11, 12] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/tasks/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id, "title": "Task", "done": false, "project_id": 3
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/tasks/{id}")))
            .and(body_json(json!({
                "id": id, "title": "Task", "done": true, "project_id": 3
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id, "title": "Task", "done": true, "project_id": 3,
                "done_at": "2026-06-10T10:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_complete",
        json!({"task_ids": [11, 12]}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["total"], 2);
    assert_eq!(body["succeeded"], 2);
    assert_eq!(body["failed"], 0);
    for (index, id) in [(0, 11), (1, 12)] {
        assert_eq!(body["results"][index]["task_id"], id);
        assert_eq!(body["results"][index]["ok"], true);
        assert_eq!(body["results"][index]["operation"], "complete");
        assert_eq!(body["results"][index]["task"]["done"], true);
        assert!(body["results"][index]["error"].is_null());
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_reopen_marks_tasks_not_done() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/13"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 13, "title": "Task", "done": true, "project_id": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/13"))
        .and(body_json(json!({
            "id": 13, "title": "Task", "done": false, "project_id": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 13, "title": "Task", "done": false, "project_id": 3
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_reopen",
        json!({"task_ids": [13]}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["results"][0]["operation"], "reopen");
    assert_eq!(body["results"][0]["task"]["done"], false);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_update_reports_partial_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 21, "title": "First", "done": false, "project_id": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/21"))
        .and(body_json(json!({
            "id": 21, "title": "First", "done": false, "project_id": 3, "priority": 4
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 21, "title": "First", "done": false, "project_id": 3, "priority": 4
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The second task does not exist: its read fails, no write is issued.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/22"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": 4002, "message": "The task does not exist."
        })))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_update",
        json!({"task_ids": [21, 22], "priority": 4}),
    )
    .await
    .expect("partial failure must not fail the tool call");

    let body = structured(&result);
    assert_eq!(body["ok"], false);
    assert_eq!(body["total"], 2);
    assert_eq!(body["succeeded"], 1);
    assert_eq!(body["failed"], 1);

    assert_eq!(body["results"][0]["ok"], true);
    assert_eq!(body["results"][0]["operation"], "update");
    assert_eq!(body["results"][0]["task"]["priority"], 4);

    let failure = &body["results"][1];
    assert_eq!(failure["task_id"], 22);
    assert_eq!(failure["ok"], false);
    assert!(failure["task"].is_null());
    assert_eq!(failure["error"]["kind"], "not_found");
    assert_eq!(failure["error"]["http_status"], 404);
    assert_eq!(failure["error"]["vikunja_error_code"], 4002);
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not exist")
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_validation_rejects_bad_arguments_before_any_request() {
    // Unreachable Vikunja: any request would fail differently, so an
    // invalid-params error proves validation fired first.
    let client = connect("http://127.0.0.1:1").await;

    // Empty task_ids on every bulk tool.
    for (tool, extra) in [
        ("vikunja_tasks_bulk_complete", json!({})),
        ("vikunja_tasks_bulk_reopen", json!({})),
        ("vikunja_tasks_bulk_update", json!({"priority": 1})),
        ("vikunja_tasks_bulk_move", json!({"project_id": 1})),
        ("vikunja_task_labels_bulk_add", json!({"label_id": 1})),
        ("vikunja_task_labels_bulk_remove", json!({"label_id": 1})),
        ("vikunja_tasks_bulk_assign", json!({"user_id": 1})),
        ("vikunja_tasks_bulk_unassign", json!({"user_id": 1})),
    ] {
        let mut args = extra;
        args["task_ids"] = json!([]);
        let err = call(&client, tool, args).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool}");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{tool}");
        assert!(
            data.message.contains("task_ids"),
            "{tool}: {}",
            data.message
        );
    }

    // Oversized batch: more ids than the documented per-call cap.
    let too_many: Vec<i64> = (1..=101).collect();
    let err = call(
        &client,
        "vikunja_tasks_bulk_complete",
        json!({"task_ids": too_many}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("at most 100"), "{}", data.message);

    // Non-positive task ids.
    for ids in [json!([0]), json!([-1]), json!([3, -7])] {
        let err = call(
            &client,
            "vikunja_tasks_bulk_complete",
            json!({"task_ids": ids}),
        )
        .await
        .unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(data.message.contains("positive"));
    }

    // Non-positive companion ids.
    for (tool, args, field) in [
        (
            "vikunja_task_labels_bulk_add",
            json!({"task_ids": [1], "label_id": 0}),
            "label_id",
        ),
        (
            "vikunja_task_labels_bulk_remove",
            json!({"task_ids": [1], "label_id": -2}),
            "label_id",
        ),
        (
            "vikunja_tasks_bulk_assign",
            json!({"task_ids": [1], "user_id": -3}),
            "user_id",
        ),
        (
            "vikunja_tasks_bulk_unassign",
            json!({"task_ids": [1], "user_id": 0}),
            "user_id",
        ),
        (
            "vikunja_tasks_bulk_move",
            json!({"task_ids": [1], "project_id": 0}),
            "project_id",
        ),
        (
            "vikunja_tasks_bulk_update",
            json!({"task_ids": [1], "project_id": -1}),
            "project_id",
        ),
    ] {
        let err = call(&client, tool, args).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool}");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{tool}");
        assert!(data.message.contains(field), "{tool}: {}", data.message);
    }

    // Empty bulk update patch.
    let err = call(
        &client,
        "vikunja_tasks_bulk_update",
        json!({"task_ids": [1, 2]}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("nothing to update"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_label_add_and_remove_return_per_task_messages() {
    let server = MockServer::start().await;
    for id in [1, 2] {
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/tasks/{id}/labels")))
            .and(body_json(json!({"label_id": 7})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"label_id": 7})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/tasks/{id}/labels/7")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "removed"})))
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;

    let added = call(
        &client,
        "vikunja_task_labels_bulk_add",
        json!({"task_ids": [1, 2], "label_id": 7}),
    )
    .await
    .unwrap();
    let body = structured(&added);
    assert_eq!(body["ok"], true);
    assert_eq!(body["succeeded"], 2);
    assert_eq!(body["results"][0]["operation"], "label_add");
    assert_eq!(body["results"][0]["message"], "label 7 added to task 1");
    assert_eq!(body["results"][1]["message"], "label 7 added to task 2");
    assert!(body["results"][0]["task"].is_null());

    let removed = call(
        &client,
        "vikunja_task_labels_bulk_remove",
        json!({"task_ids": [1, 2], "label_id": 7}),
    )
    .await
    .unwrap();
    let body = structured(&removed);
    assert_eq!(body["ok"], true);
    assert_eq!(body["results"][0]["operation"], "label_remove");
    assert_eq!(body["results"][0]["message"], "removed");
    assert_eq!(body["results"][1]["message"], "removed");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_assign_and_unassign_users() {
    let server = MockServer::start().await;
    for id in [4, 5] {
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/tasks/{id}/assignees")))
            .and(body_json(json!({"user_id": 3})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"user_id": 3})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/tasks/{id}/assignees/3")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"message": "unassigned"})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;

    let assigned = call(
        &client,
        "vikunja_tasks_bulk_assign",
        json!({"task_ids": [4, 5], "user_id": 3}),
    )
    .await
    .unwrap();
    let body = structured(&assigned);
    assert_eq!(body["ok"], true);
    assert_eq!(body["results"][0]["operation"], "assign");
    assert_eq!(body["results"][0]["message"], "user 3 assigned to task 4");
    assert_eq!(body["results"][1]["message"], "user 3 assigned to task 5");

    let unassigned = call(
        &client,
        "vikunja_tasks_bulk_unassign",
        json!({"task_ids": [4, 5], "user_id": 3}),
    )
    .await
    .unwrap();
    let body = structured(&unassigned);
    assert_eq!(body["ok"], true);
    assert_eq!(body["results"][0]["operation"], "unassign");
    assert_eq!(body["results"][0]["message"], "unassigned");
    assert_eq!(body["results"][1]["message"], "unassigned");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_move_preserves_fields_via_read_merge_write() {
    let server = MockServer::start().await;
    // The exact POST body proves read-merge-write: every field from the GET
    // is preserved and only project_id changes.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/31"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 31, "title": "Keep me", "done": false, "project_id": 3, "priority": 2
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/31"))
        .and(body_json(json!({
            "id": 31, "title": "Keep me", "done": false, "project_id": 9, "priority": 2
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 31, "title": "Keep me", "done": false, "project_id": 9, "priority": 2
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_move",
        json!({"task_ids": [31], "project_id": 9}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["results"][0]["operation"], "move");
    assert_eq!(body["results"][0]["task"]["project_id"], 9);
    assert_eq!(body["results"][0]["task"]["title"], "Keep me");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_writes_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/41"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 41, "title": "Task", "done": false, "project_id": 3
        })))
        .expect(1)
        .mount(&server)
        .await;
    // expect(1) verifies on drop that the failing write was attempted
    // exactly once — no automatic retry.
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/41"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"message": "boom"})))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_complete",
        json!({"task_ids": [41]}),
    )
    .await
    .expect("item failure must not fail the tool call");

    let body = structured(&result);
    assert_eq!(body["ok"], false);
    assert_eq!(body["failed"], 1);
    assert_eq!(body["results"][0]["error"]["kind"], "server");
    assert_eq!(body["results"][0]["error"]["http_status"], 500);
    client.cancel().await.unwrap();
}

// ----- attachments through the tool layer ------------------------------------

#[tokio::test]
async fn attachment_upload_from_base64() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "uploaded"})))
        .expect(1)
        .mount(&server)
        .await;
    // The tool re-lists attachments to identify the new one.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 4, "task_id": 9,
            "file": {"id": 1, "name": "notes.txt", "mime": "text/plain", "size": 5}
        }])))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({
            "task_id": 9,
            "file_name": "notes.txt",
            "content_base64": BASE64.encode(b"hello")
        }),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["attachment"]["id"], 4);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachment_upload_argument_validation() {
    let client = connect("http://127.0.0.1:1").await;

    // Neither source provided.
    let err = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("content_base64 or file_path"));

    // Both sources provided.
    let err = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9, "content_base64": "aGk=", "file_path": "/tmp/x", "file_name": "x"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("not both"));

    // Invalid base64.
    let err = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9, "file_name": "x", "content_base64": "!!!not-base64!!!"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("base64"));

    // base64 without a file name.
    let err = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9, "content_base64": "aGk="}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("file_name"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachment_upload_from_file_path() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "uploaded"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("report.txt");
    std::fs::write(&file_path, b"file body").unwrap();

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9, "file_path": file_path.to_str().unwrap()}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    // No matching attachment in the listing: attachment is null but the
    // upload itself succeeded.
    assert!(body["attachment"].is_null());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachment_upload_missing_file_errors() {
    let client = connect("http://127.0.0.1:1").await;
    let err = call(
        &client,
        "vikunja_task_attachments_upload",
        json!({"task_id": 9, "file_path": "/nonexistent/definitely/missing.txt"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("could not read"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachment_download_inline_base64() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"file-content".to_vec())
                .insert_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_attachments_download",
        json!({"task_id": 9, "attachment_id": 4}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["mime"], "text/plain");
    assert_eq!(body["size_bytes"], 12);
    let decoded = BASE64
        .decode(body["content_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded, b"file-content");
    assert!(body["saved_to"].is_null());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachment_download_to_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"saved bytes".to_vec()))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let save_path = dir.path().join("out.bin");

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_attachments_download",
        json!({
            "task_id": 9, "attachment_id": 4,
            "save_path": save_path.to_str().unwrap()
        }),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["saved_to"], save_path.to_str().unwrap());
    assert!(body["content_base64"].is_null());
    assert_eq!(std::fs::read(&save_path).unwrap(), b"saved bytes");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn oversized_inline_download_is_rejected() {
    let server = MockServer::start().await;
    let big = vec![0u8; 2 * 1024 * 1024 + 1];
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(big))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let err = call(
        &client,
        "vikunja_task_attachments_download",
        json!({"task_id": 9, "attachment_id": 4}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("save_path"));
    client.cancel().await.unwrap();
}

// ----- teams & users through the tool layer -------------------------------------

#[tokio::test]
async fn teams_list_switches_to_project_scope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "name": "devs", "permission": 2}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_teams_list", json!({"project_id": 7}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["teams"][0]["permission"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn users_search_returns_users() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users"))
        .and(query_param("s", "ada"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "username": "ada", "name": "Ada Lovelace"}
        ])))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_users_search", json!({"search": "ada"}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["users"][0]["username"], "ada");
    client.cancel().await.unwrap();
}

// ----- resources ------------------------------------------------------------------

#[tokio::test]
async fn resources_are_listed_and_readable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "title": "Inbox"}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1, "title": "Inbox"})))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let resources = client.list_resources(None).await.unwrap();
    let uris: Vec<&str> = resources
        .resources
        .iter()
        .map(|r| r.raw.uri.as_str())
        .collect();
    assert!(uris.contains(&"vikunja://status"));
    assert!(uris.contains(&"vikunja://projects"));
    assert!(uris.contains(&"vikunja://tasks"));

    let templates = client.list_resource_templates(None).await.unwrap();
    assert_eq!(templates.resource_templates.len(), 2);

    // Status resource: reports config without leaking the token.
    let status = client
        .read_resource(ReadResourceRequestParams::new("vikunja://status"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents {
        text, mime_type, ..
    } = &status.contents[0]
    else {
        panic!("expected text contents");
    };
    assert_eq!(mime_type.as_deref(), Some("application/json"));
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["connectivity"]["ok"], true);
    assert!(!text.contains(common::TEST_TOKEN));

    // Project list resource.
    let projects = client
        .read_resource(ReadResourceRequestParams::new("vikunja://projects"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &projects.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["projects"][0]["title"], "Inbox");

    // Individual project via URI template.
    let one = client
        .read_resource(ReadResourceRequestParams::new("vikunja://projects/1"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &one.contents[0] else {
        panic!("expected text contents");
    };
    assert!(text.contains("Inbox"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn status_resource_reports_unreachable_instance() {
    let client = connect("http://127.0.0.1:1").await;
    let status = client
        .read_resource(ReadResourceRequestParams::new("vikunja://status"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &status.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["connectivity"]["ok"], false);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_resource_template_reads_one_task() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/42"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": 42, "title": "Ship it", "project_id": 1})),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = client
        .read_resource(ReadResourceRequestParams::new("vikunja://tasks/42"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
        panic!("expected text contents");
    };
    assert!(text.contains("Ship it"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn unknown_resource_uri_errors() {
    let client = connect("http://127.0.0.1:1").await;
    let err = client
        .read_resource(ReadResourceRequestParams::new("vikunja://bogus"))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::McpError(_)), "got {err:?}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn server_info_advertises_capabilities() {
    let client = connect("http://127.0.0.1:1").await;
    let info = client.peer_info().expect("server info");
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.resources.is_some());
    assert!(
        info.instructions
            .as_ref()
            .is_some_and(|i| i.contains("Vikunja"))
    );
    client.cancel().await.unwrap();
}

// ----- full tool surface ------------------------------------------------------

/// Drives every remaining tool through the MCP loop once, against mocks
/// that encode the exact Vikunja endpoint contract.
#[tokio::test]
async fn remaining_tool_surface_round_trips() {
    let server = MockServer::start().await;

    // Projects.
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "title": "Work", "description": "d"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/projects/7"))
        .and(body_json(
            json!({"id": 7, "title": "Renamed", "description": "d"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "title": "Renamed", "description": "d"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/projects/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "project gone"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects"))
        .and(body_json(json!({"title": "Fresh"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 8, "title": "Fresh"})))
        .mount(&server)
        .await;

    // Tasks.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("filter", "(priority >= 3) && project_id = 7"))
        .and(query_param("sort_by", "due_date"))
        .and(query_param("order_by", "asc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 9, "title": "T", "project_id": 7}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "T", "done": true, "project_id": 7
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "T", "done": false, "project_id": 7
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "task gone"})))
        .mount(&server)
        .await;

    // Labels.
    Mock::given(method("GET"))
        .and(path("/api/v1/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 5, "title": "bug"}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/labels"))
        .and(body_json(json!({"title": "bug", "hex_color": "ff0000"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 5, "title": "bug"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/labels/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 5, "title": "bug"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/labels/5"))
        .and(body_json(json!({"id": 5, "title": "defect"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 5, "title": "defect"})))
        .mount(&server)
        .await;
    // Label deletion answers with the deleted label, not a message.
    Mock::given(method("DELETE"))
        .and(path("/api/v1/labels/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 5, "title": "bug"})))
        .mount(&server)
        .await;

    // Comments.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "comment": "hi"}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/comments"))
        .and(body_json(json!({"comment": "new"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 2, "comment": "new"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9/comments/2"))
        .and(body_json(json!({"comment": "edited"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": 2, "comment": "edited"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/comments/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "comment gone"})))
        .mount(&server)
        .await;

    // Attachments.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 4, "task_id": 9,
            "file": {"id": 1, "name": "a.txt", "mime": "text/plain", "size": 1}
        }])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"message": "attachment gone"})),
        )
        .mount(&server)
        .await;

    // Teams (global scope).
    Mock::given(method("GET"))
        .and(path("/api/v1/teams"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "name": "devs"}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let result = call(&client, "vikunja_projects_get", json!({"project_id": 7}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["title"], "Work");

    let result = call(
        &client,
        "vikunja_projects_update",
        json!({"project_id": 7, "title": "Renamed"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["title"], "Renamed");

    let result = call(
        &client,
        "vikunja_projects_create",
        json!({"title": "Fresh"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["id"], 8);

    let result = call(&client, "vikunja_projects_delete", json!({"project_id": 7}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["message"], "project gone");

    let result = call(
        &client,
        "vikunja_tasks_list",
        json!({
            "project_id": 7, "filter": "priority >= 3",
            "sort_by": "due_date", "order_by": "asc"
        }),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["tasks"][0]["id"], 9);

    let result = call(&client, "vikunja_tasks_get", json!({"task_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["id"], 9);

    let result = call(
        &client,
        "vikunja_tasks_update",
        json!({"task_id": 9, "priority": 4}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["id"], 9);

    let result = call(&client, "vikunja_tasks_reopen", json!({"task_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["done"], false);

    let result = call(&client, "vikunja_tasks_delete", json!({"task_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["message"], "task gone");

    let result = call(&client, "vikunja_labels_list", json!({}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["labels"][0]["title"], "bug");

    let result = call(
        &client,
        "vikunja_labels_create",
        json!({"title": "bug", "hex_color": "ff0000"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["id"], 5);

    let result = call(
        &client,
        "vikunja_labels_update",
        json!({"label_id": 5, "title": "defect"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["title"], "defect");

    let result = call(&client, "vikunja_labels_delete", json!({"label_id": 5}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["message"], "label 5 (\"bug\") deleted");

    let result = call(&client, "vikunja_task_comments_list", json!({"task_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["comments"][0]["comment"], "hi");

    let result = call(
        &client,
        "vikunja_task_comments_create",
        json!({"task_id": 9, "comment": "new"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["id"], 2);

    let result = call(
        &client,
        "vikunja_task_comments_update",
        json!({"task_id": 9, "comment_id": 2, "comment": "edited"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["comment"], "edited");

    let result = call(
        &client,
        "vikunja_task_comments_delete",
        json!({"task_id": 9, "comment_id": 2}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["message"], "comment gone");

    let result = call(
        &client,
        "vikunja_task_attachments_list",
        json!({"task_id": 9}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["attachments"][0]["id"], 4);

    let result = call(
        &client,
        "vikunja_task_attachments_delete",
        json!({"task_id": 9, "attachment_id": 4}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["message"], "attachment gone");

    let result = call(&client, "vikunja_teams_list", json!({}))
        .await
        .unwrap();
    assert_eq!(structured(&result)["teams"][0]["name"], "devs");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn more_argument_validation_paths() {
    let client = connect("http://127.0.0.1:1").await;

    // per_page = 0.
    let err = call(&client, "vikunja_labels_list", json!({"per_page": 0}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("per_page"));

    // Negative label id on update.
    let err = call(&client, "vikunja_labels_update", json!({"label_id": -2}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("label_id"));

    // Blank comment.
    let err = call(
        &client,
        "vikunja_task_comments_create",
        json!({"task_id": 1, "comment": "   "}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("comment"));

    // Blank task title.
    let err = call(
        &client,
        "vikunja_tasks_create",
        json!({"project_id": 1, "title": ""}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("title"));

    // Invalid project_id on tasks_list / tasks_update / teams_list.
    for (tool, args) in [
        ("vikunja_tasks_list", json!({"project_id": -1})),
        (
            "vikunja_tasks_update",
            json!({"task_id": 1, "project_id": 0}),
        ),
        ("vikunja_teams_list", json!({"project_id": -5})),
        (
            "vikunja_tasks_create",
            json!({"project_id": -1, "title": "x"}),
        ),
    ] {
        let err = call(&client, tool, args).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool}");
        };
        assert!(
            data.message.contains("project_id"),
            "{tool}: {}",
            data.message
        );
    }

    // Empty update patch reaches the client-level guard.
    let err = call(&client, "vikunja_tasks_update", json!({"task_id": 1}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("nothing to update"));

    client.cancel().await.unwrap();
}

/// Oversized uploads are rejected before any network traffic. Called
/// directly (not through the MCP loop) to avoid pushing 28 MB of JSON
/// through the transport.
#[tokio::test]
async fn oversized_upload_from_file_is_rejected_before_reading() {
    use rmcp::handler::server::wrapper::Parameters;
    use vikunja_rust_mcp::mcp::tools::{AttachmentsUploadArgs, MAX_UPLOAD_BYTES};

    let dir = tempfile::tempdir().unwrap();
    let big_path = dir.path().join("big.bin");
    std::fs::write(&big_path, vec![0u8; MAX_UPLOAD_BYTES + 1]).unwrap();

    let server = vikunja_rust_mcp::mcp::VikunjaMcpServer::new(test_client("http://127.0.0.1:1"));
    let result = server
        .task_attachments_upload(Parameters(AttachmentsUploadArgs {
            task_id: 9,
            file_name: None,
            content_base64: None,
            file_path: Some(big_path.to_string_lossy().into_owned()),
        }))
        .await;
    let Err(err) = result else {
        panic!("expected oversized upload to be rejected");
    };
    assert!(err.message.contains("maximum supported upload"));
}

#[tokio::test]
async fn oversized_base64_upload_is_rejected_before_decoding() {
    use rmcp::handler::server::wrapper::Parameters;
    use vikunja_rust_mcp::mcp::tools::{AttachmentsUploadArgs, MAX_UPLOAD_BYTES};

    // A base64 string whose decoded size estimate exceeds the cap. Built
    // from valid base64 characters so only the size check can reject it.
    let encoded = "A".repeat((MAX_UPLOAD_BYTES / 3 + 1) * 4);

    let server = vikunja_rust_mcp::mcp::VikunjaMcpServer::new(test_client("http://127.0.0.1:1"));
    let result = server
        .task_attachments_upload(Parameters(AttachmentsUploadArgs {
            task_id: 9,
            file_name: Some("big.bin".into()),
            content_base64: Some(encoded),
            file_path: None,
        }))
        .await;
    let Err(err) = result else {
        panic!("expected oversized upload to be rejected");
    };
    assert!(err.message.contains("maximum supported upload"));
}

#[tokio::test]
async fn tasks_resource_lists_tasks() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "title": "Only task", "project_id": 1}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = client
        .read_resource(ReadResourceRequestParams::new("vikunja://tasks"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["tasks"][0]["title"], "Only task");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn resource_reads_propagate_api_errors() {
    // Vikunja unreachable: list resources must fail, not hang or panic.
    let client = connect("http://127.0.0.1:1").await;
    for uri in [
        "vikunja://projects",
        "vikunja://tasks",
        "vikunja://projects/3",
    ] {
        let err = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap_err();
        assert!(matches!(err, ServiceError::McpError(_)), "{uri}: {err:?}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn malformed_template_ids_are_not_found() {
    let client = connect("http://127.0.0.1:1").await;
    for uri in [
        "vikunja://tasks/abc",
        "vikunja://tasks/-2",
        "vikunja://projects/0",
    ] {
        let err = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {uri}");
        };
        assert_eq!(
            data.code,
            rmcp::model::ErrorCode::RESOURCE_NOT_FOUND,
            "{uri}"
        );
    }
    client.cancel().await.unwrap();
}
