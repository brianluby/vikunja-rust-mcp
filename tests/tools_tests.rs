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
    "vikunja_task_relations_list",
    "vikunja_task_relations_create",
    "vikunja_task_relations_delete",
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
    "vikunja_filters_list",
    "vikunja_filters_get",
    "vikunja_filters_create",
    "vikunja_filters_update",
    "vikunja_filters_delete",
    "vikunja_filters_tasks",
    "vikunja_project_views_list",
    "vikunja_buckets_list",
    "vikunja_dates_resolve",
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

/// Strict MCP clients log "unknown format ignored" warnings for schemas
/// using schemars' Rust-specific unsigned formats (`uint`, `uint32`, ...).
/// Every published input and output schema must be free of them.
#[tokio::test]
async fn schemas_contain_no_nonstandard_unsigned_formats() {
    fn collect_formats(value: &serde_json::Value, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(format)) = map.get("format") {
                    found.push(format.clone());
                }
                for nested in map.values() {
                    collect_formats(nested, found);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    collect_formats(nested, found);
                }
            }
            _ => {}
        }
    }

    let client = connect("http://127.0.0.1:1").await;
    let tools = client.list_all_tools().await.unwrap();
    assert!(!tools.is_empty());

    for tool in &tools {
        let mut formats = Vec::new();
        collect_formats(
            &serde_json::to_value(&tool.input_schema).unwrap(),
            &mut formats,
        );
        if let Some(output) = &tool.output_schema {
            collect_formats(&serde_json::to_value(output).unwrap(), &mut formats);
        }
        for format in formats {
            assert!(
                !format.starts_with("uint"),
                "tool {} publishes schemars-specific format {format:?}",
                tool.name
            );
        }
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

// ----- date shortcuts ---------------------------------------------------------

/// RFC 3339 string for a local date at HH:MM, exactly as the server's
/// resolver renders it (the e2e server runs with the default 09:00/23:59
/// date config in the machine's local timezone).
fn local_rfc3339(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> String {
    use chrono::TimeZone as _;
    let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap();
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[tokio::test]
async fn dates_resolve_previews_shortcuts_without_calling_vikunja() {
    // Unreachable Vikunja: success proves the tool never talks to it.
    let client = connect("http://127.0.0.1:1").await;

    let result = call(
        &client,
        "vikunja_dates_resolve",
        json!({
            "expression": "in 2 days",
            "reference_time": "2026-06-10T12:00:00Z",
            "target": "due_date"
        }),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["expression"], "in 2 days");
    assert_eq!(body["reference_time"], "2026-06-10T12:00:00Z");
    assert_eq!(body["resolved"], "2026-06-12T09:00:00Z");
    assert_eq!(body["clears_date"], false);
    assert_eq!(body["default_time_used"], "09:00");
    assert!(
        body["timezone_description"]
            .as_str()
            .unwrap()
            .contains("+00:00")
    );

    // Non-UTC reference offsets are preserved in the resolution.
    let result = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "next friday", "reference_time": "2026-06-10T12:00:00-04:00"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&result)["resolved"], "2026-06-12T09:00:00-04:00");

    // End of week applies the end-of-day time.
    let result = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "end of week", "reference_time": "2026-06-10T12:00:00Z"}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["resolved"], "2026-06-14T23:59:00Z");
    assert_eq!(body["default_time_used"], "23:59");

    // Clear words report clears_date and resolve to nothing; without a
    // reference_time the server local timezone is used.
    let result = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "no due date"}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["clears_date"], true);
    assert!(body["resolved"].is_null());
    assert!(body["default_time_used"].is_null());
    assert!(
        body["timezone_description"]
            .as_str()
            .unwrap()
            .contains("server local timezone")
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn dates_resolve_rejects_invalid_input() {
    let client = connect("http://127.0.0.1:1").await;

    let err = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "someday"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("unsupported date shortcut"));
    assert!(data.message.contains("in N days"));

    let err = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "today", "reference_time": "yesterday"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("RFC 3339"));

    let err = call(
        &client,
        "vikunja_dates_resolve",
        json!({"expression": "today", "target": "title"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("target"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_create_resolves_shortcuts_before_sending() {
    let expected_due = local_rfc3339(2026, 7, 1, 9, 0);
    let server = MockServer::start().await;
    // Exact body: the shortcut arrives as a resolved RFC 3339 due_date and
    // the `none` start date shortcut omits the field entirely.
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects/3/tasks"))
        .and(body_json(
            json!({"title": "Ship", "due_date": expected_due}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 60, "title": "Ship", "project_id": 3, "due_date": expected_due
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_create",
        json!({
            "project_id": 3,
            "title": "Ship",
            "due_date_shortcut": "2026-07-01",
            "start_date_shortcut": "none"
        }),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["id"], 60);
    assert_eq!(body["due_date"], expected_due);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_update_shortcut_preserves_fields_via_read_merge_write() {
    let expected_due = local_rfc3339(2026, 7, 1, 9, 0);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3, "priority": 2
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The merged write carries the resolved RFC 3339 value and every field
    // from the GET.
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3, "priority": 2,
            "due_date": expected_due
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3, "priority": 2,
            "due_date": expected_due
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_update",
        json!({"task_id": 9, "due_date_shortcut": "2026-07-01"}),
    )
    .await
    .unwrap();

    assert_eq!(structured(&result)["due_date"], expected_due);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_update_clear_shortcut_sends_zero_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "T", "done": false, "project_id": 3,
            "due_date": "2026-07-01T09:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "T", "done": false, "project_id": 3,
            "due_date": "0001-01-01T00:00:00Z"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "T", "done": false, "project_id": 3
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    call(
        &client,
        "vikunja_tasks_update",
        json!({"task_id": 9, "due_date_shortcut": "clear"}),
    )
    .await
    .unwrap();
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bulk_update_applies_date_shortcut_per_task() {
    let server = MockServer::start().await;
    for id in [7, 8] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/tasks/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id, "title": "Task", "done": false, "project_id": 3,
                "due_date": "2026-07-01T09:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/tasks/{id}")))
            .and(body_json(json!({
                "id": id, "title": "Task", "done": false, "project_id": 3,
                "due_date": "0001-01-01T00:00:00Z"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id, "title": "Task", "done": false, "project_id": 3
            })))
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_bulk_update",
        json!({"task_ids": [7, 8], "due_date_shortcut": "clear"}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["ok"], true);
    assert_eq!(body["succeeded"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn date_and_shortcut_together_are_rejected() {
    let client = connect("http://127.0.0.1:1").await;
    for (tool, args) in [
        (
            "vikunja_tasks_create",
            json!({
                "project_id": 3, "title": "X",
                "due_date": "2026-07-01T09:00:00Z", "due_date_shortcut": "tomorrow"
            }),
        ),
        (
            "vikunja_tasks_update",
            json!({
                "task_id": 9,
                "start_date": "2026-07-01T09:00:00Z", "start_date_shortcut": "today"
            }),
        ),
        (
            "vikunja_tasks_bulk_update",
            json!({
                "task_ids": [1],
                "end_date": "2026-07-01T09:00:00Z", "end_date_shortcut": "friday"
            }),
        ),
    ] {
        let err = call(&client, tool, args).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool}");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{tool}");
        assert!(
            data.message.contains("not both"),
            "{tool}: {}",
            data.message
        );
    }
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
    assert_eq!(templates.resource_templates.len(), 5);

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

/// Reads a resource and parses its JSON body, asserting the MIME type and
/// that the API token never leaks into the output.
async fn read_json_resource(client: &McpClient, uri: &str) -> serde_json::Value {
    let result = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents {
        text, mime_type, ..
    } = &result.contents[0]
    else {
        panic!("expected text contents for {uri}");
    };
    assert_eq!(mime_type.as_deref(), Some("application/json"), "{uri}");
    assert!(!text.contains(common::TEST_TOKEN), "{uri} leaks the token");
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn task_view_resources_are_advertised() {
    let client = connect("http://127.0.0.1:1").await;
    let resources = client.list_resources(None).await.unwrap();
    for uri in [
        "vikunja://tasks/today",
        "vikunja://tasks/overdue",
        "vikunja://tasks/upcoming",
        "vikunja://tasks/high-priority",
        "vikunja://tasks/inbox",
        "vikunja://tasks/recently-updated",
    ] {
        let resource = resources
            .resources
            .iter()
            .find(|r| r.raw.uri == uri)
            .unwrap_or_else(|| panic!("{uri} not advertised"));
        assert_eq!(resource.raw.mime_type.as_deref(), Some("application/json"));
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_view_resources_send_exact_filters() {
    // (uri, view, expected filter or "" for none, sort_by, order_by)
    let cases: &[(&str, &str, &str, &str, &str)] = &[
        (
            "vikunja://tasks/today",
            "today",
            "done = false && due_date >= now/d && due_date < now/d+1d",
            "due_date",
            "asc",
        ),
        (
            "vikunja://tasks/overdue",
            "overdue",
            "done = false && due_date < now/d && due_date != null",
            "due_date",
            "asc",
        ),
        (
            "vikunja://tasks/upcoming",
            "upcoming",
            "done = false && due_date >= now/d && due_date < now/d+7d",
            "due_date",
            "asc",
        ),
        (
            "vikunja://tasks/high-priority",
            "high-priority",
            "done = false && priority >= 3",
            "priority",
            "desc",
        ),
        (
            "vikunja://tasks/inbox",
            "inbox",
            "done = false && due_date = null",
            "updated",
            "desc",
        ),
        (
            "vikunja://tasks/recently-updated",
            "recently-updated",
            "",
            "updated",
            "desc",
        ),
    ];

    for (uri, view, filter, sort_by, order_by) in cases {
        let server = MockServer::start().await;
        let mut mock = Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", "1"))
            .and(query_param("sort_by", *sort_by))
            .and(query_param("order_by", *order_by));
        if filter.is_empty() {
            mock = mock.and(wiremock::matchers::query_param_is_missing("filter"));
        } else {
            mock = mock.and(query_param("filter", *filter));
        }
        mock.respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "title": "Plan week", "project_id": 1}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .expect(1)
        .mount(&server)
        .await;

        let client = connect(&server.uri()).await;
        let parsed = read_json_resource(&client, uri).await;
        assert_eq!(parsed["view"], *view, "{uri}");
        if filter.is_empty() {
            assert_eq!(parsed["filter"], serde_json::Value::Null, "{uri}");
        } else {
            assert_eq!(parsed["filter"], *filter, "{uri}");
        }
        assert_eq!(parsed["sort_by"], *sort_by, "{uri}");
        assert_eq!(parsed["order_by"], *order_by, "{uri}");
        assert_eq!(parsed["page_cap"], 10, "{uri}");
        assert_eq!(parsed["pages_read"], 1, "{uri}");
        assert_eq!(parsed["truncated"], false, "{uri}");
        assert_eq!(parsed["count"], 1, "{uri}");
        assert_eq!(parsed["tasks"][0]["title"], "Plan week", "{uri}");
        assert!(
            parsed["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "{uri}"
        );
        client.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn task_view_resource_reports_truncation_at_page_cap() {
    let server = MockServer::start().await;
    for page in 1..=10u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", page.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(
                        json!([{"id": page, "title": format!("T{page}"), "project_id": 1}]),
                    )
                    .insert_header("x-pagination-total-pages", "25"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;
    let parsed = read_json_resource(&client, "vikunja://tasks/recently-updated").await;
    assert_eq!(parsed["page_cap"], 10);
    assert_eq!(parsed["pages_read"], 10);
    assert_eq!(parsed["truncated"], true);
    assert_eq!(parsed["count"], 10);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_view_reads_propagate_api_errors() {
    let client = connect("http://127.0.0.1:1").await;
    let err = client
        .read_resource(ReadResourceRequestParams::new("vikunja://tasks/today"))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::McpError(_)), "got {err:?}");
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

// ----- saved filters ------------------------------------------------------------

fn saved_filter_json(id: i64, title: &str, filter: &str) -> serde_json::Value {
    json!({
        "id": id, "title": title, "description": "open work",
        "filters": {
            "sort_by": ["due_date", "id"],
            "order_by": ["asc", "desc"],
            "filter": filter,
            "filter_timezone": "America/Los_Angeles",
            "filter_include_nulls": false
        },
        "owner": {"id": 1, "username": "ada"},
        "is_favorite": false,
        "created": "2026-01-01T00:00:00Z", "updated": "2026-01-02T00:00:00Z"
    })
}

#[tokio::test]
async fn filters_crud_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/filters"))
        .and(body_json(json!({
            "title": "Open work",
            "filters": {"filter": "done = false", "sort_by": ["due_date"], "order_by": ["asc"]}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Renamed",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/filters/9"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"message": "Successfully deleted."})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let created = call(
        &client,
        "vikunja_filters_create",
        json!({
            "title": "Open work", "filter": "done = false",
            "sort_by": ["due_date"], "order_by": ["asc"]
        }),
    )
    .await
    .unwrap();
    assert_eq!(structured(&created)["id"], 9);

    let fetched = call(&client, "vikunja_filters_get", json!({"filter_id": 9}))
        .await
        .unwrap();
    let body = structured(&fetched);
    assert_eq!(body["title"], "Open work");
    assert_eq!(body["filters"]["filter"], "done = false");

    let updated = call(
        &client,
        "vikunja_filters_update",
        json!({"filter_id": 9, "title": "Renamed"}),
    )
    .await
    .unwrap();
    assert_eq!(structured(&updated)["title"], "Renamed");

    let deleted = call(&client, "vikunja_filters_delete", json!({"filter_id": 9}))
        .await
        .unwrap();
    assert_eq!(structured(&deleted)["ok"], true);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn filters_list_derives_from_pseudo_projects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "title": "Inbox"},
            {"id": -2, "title": "Open work", "description": "open", "is_favorite": true},
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_filters_list", json!({}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["filters"].as_array().unwrap().len(), 1);
    assert_eq!(body["filters"][0]["filter_id"], 1);
    assert_eq!(body["filters"][0]["pseudo_project_id"], -2);
    assert_eq!(body["filters"][0]["title"], "Open work");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn filters_tasks_executes_stored_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("filter", "done = false"))
        .and(query_param("sort_by", "due_date"))
        .and(query_param("order_by", "asc"))
        .and(query_param("filter_timezone", "America/Los_Angeles"))
        .and(query_param("filter_include_nulls", "false"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 4, "title": "Pay rent", "project_id": 1}]))
                .insert_header("x-pagination-total-pages", "1")
                .insert_header("x-pagination-result-count", "1"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_filters_tasks", json!({"filter_id": 9}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["filter_id"], 9);
    assert_eq!(body["title"], "Open work");
    assert_eq!(body["filter"], "done = false");
    assert_eq!(body["tasks"][0]["title"], "Pay rent");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn filters_validation_rejects_bad_arguments_before_any_request() {
    // Unreachable address: a request would fail loudly, proving validation
    // rejected the call first.
    let client = connect("http://127.0.0.1:1").await;
    let cases: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "vikunja_filters_create",
            json!({"title": "   ", "filter": "done = false"}),
            "title",
        ),
        (
            "vikunja_filters_create",
            json!({"title": "Open", "filter": "   "}),
            "filter",
        ),
        (
            "vikunja_filters_create",
            json!({"title": "Open", "filter": "(done = false"}),
            "parenthes",
        ),
        (
            "vikunja_filters_create",
            json!({"title": "Open", "filter": "done = false)"}),
            "parenthes",
        ),
        (
            "vikunja_filters_create",
            json!({"title": "Open", "filter": "title ~ 'unterminated"}),
            "quote",
        ),
        (
            "vikunja_filters_create",
            json!({
                "title": "Open", "filter": "done = false",
                "sort_by": ["due_date", "id"], "order_by": ["asc"]
            }),
            "same number",
        ),
        (
            "vikunja_filters_create",
            json!({
                "title": "Open", "filter": "done = false",
                "sort_by": ["due_date"], "order_by": ["upward"]
            }),
            "'asc' or 'desc'",
        ),
        (
            "vikunja_filters_create",
            json!({"title": "Open", "filter": "done = false", "filter_timezone": "  "}),
            "filter_timezone",
        ),
        ("vikunja_filters_get", json!({"filter_id": 0}), "positive"),
        (
            "vikunja_filters_update",
            json!({"filter_id": 9, "filter": "((done = false)"}),
            "parenthes",
        ),
        (
            "vikunja_filters_update",
            json!({"filter_id": 9}),
            "nothing to update",
        ),
        (
            "vikunja_filters_delete",
            json!({"filter_id": -3}),
            "positive",
        ),
        (
            "vikunja_filters_tasks",
            json!({"filter_id": 9, "page": 0}),
            "page",
        ),
    ];
    for (tool, args, expected) in cases {
        let err = call(&client, tool, args.clone()).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool} {args}");
        };
        assert!(
            data.message.contains(expected),
            "{tool} {args}: expected '{expected}' in '{}'",
            data.message
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn filter_resources_are_listed_and_readable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([
                    {"id": 1, "title": "Inbox"},
                    {"id": -10, "title": "Open work"},
                ]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("filter", "done = false"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 4, "title": "Pay rent", "project_id": 1}]))
                .insert_header("x-pagination-total-pages", "1"),
        )
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let resources = client.list_resources(None).await.unwrap();
    let uris: Vec<&str> = resources
        .resources
        .iter()
        .map(|r| r.raw.uri.as_str())
        .collect();
    assert!(uris.contains(&"vikunja://filters"));

    let templates = client.list_resource_templates(None).await.unwrap();
    let template_uris: Vec<&str> = templates
        .resource_templates
        .iter()
        .map(|t| t.raw.uri_template.as_str())
        .collect();
    assert!(template_uris.contains(&"vikunja://filters/{id}"));
    assert!(template_uris.contains(&"vikunja://filters/{id}/tasks"));

    // Saved filter list resource (from pseudo-projects).
    let list = client
        .read_resource(ReadResourceRequestParams::new("vikunja://filters"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &list.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["filters"][0]["filter_id"], 9);
    assert_eq!(parsed["filters"][0]["pseudo_project_id"], -10);

    // One saved filter definition.
    let one = client
        .read_resource(ReadResourceRequestParams::new("vikunja://filters/9"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &one.contents[0] else {
        panic!("expected text contents");
    };
    assert!(text.contains("done = false"));

    // Tasks matching the saved filter.
    let tasks = client
        .read_resource(ReadResourceRequestParams::new("vikunja://filters/9/tasks"))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &tasks.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["filter_id"], 9);
    assert_eq!(parsed["filter"], "done = false");
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["tasks"][0]["title"], "Pay rent");

    client.cancel().await.unwrap();
}

// ----- task relations ---------------------------------------------------------

#[tokio::test]
async fn task_relations_create_and_delete_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/5/relations"))
        .and(body_json(json!({
            "task_id": 5, "other_task_id": 9, "relation_kind": "blocking"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "task_id": 5, "other_task_id": 9, "relation_kind": "blocking",
            "created": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/5/relations/blocking/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "The task relation was successfully deleted."
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let created = call(
        &client,
        "vikunja_task_relations_create",
        json!({"task_id": 5, "other_task_id": 9, "relation_kind": "blocking"}),
    )
    .await
    .unwrap();
    let body = structured(&created);
    assert_eq!(body["task_id"], 5);
    assert_eq!(body["other_task_id"], 9);
    assert_eq!(body["relation_kind"], "blocking");

    let deleted = call(
        &client,
        "vikunja_task_relations_delete",
        json!({"task_id": 5, "other_task_id": 9, "relation_kind": "blocking"}),
    )
    .await
    .unwrap();
    let body = structured(&deleted);
    assert_eq!(body["ok"], true);
    assert!(body["message"].as_str().unwrap().contains("deleted"));

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_list_groups_relations_by_kind() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "title": "parent", "project_id": 3,
            "related_tasks": {
                "subtask": [
                    {"id": 11, "title": "child a", "project_id": 3},
                    {"id": 12, "title": "child b", "project_id": 3, "done": true}
                ],
                "blocking": [
                    {"id": 9, "title": "blocker", "project_id": 3}
                ]
            }
        })))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_relations_list",
        json!({"task_id": 7}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["task_id"], 7);
    let relations = body["relations"].as_array().unwrap();
    assert_eq!(relations.len(), 2);
    // BTreeMap ordering: "blocking" sorts before "subtask".
    assert_eq!(relations[0]["relation_kind"], "blocking");
    assert_eq!(relations[0]["tasks"][0]["id"], 9);
    assert_eq!(relations[1]["relation_kind"], "subtask");
    assert_eq!(relations[1]["tasks"].as_array().unwrap().len(), 2);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_list_returns_empty_for_task_without_relations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/8"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 8, "title": "lonely", "project_id": 3
        })))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_relations_list",
        json!({"task_id": 8}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["task_id"], 8);
    assert_eq!(body["relations"].as_array().unwrap().len(), 0);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_reject_self_relation() {
    // Unreachable Vikunja: validation must fire before any request.
    let client = connect("http://127.0.0.1:1").await;
    for tool in [
        "vikunja_task_relations_create",
        "vikunja_task_relations_delete",
    ] {
        let err = call(
            &client,
            tool,
            json!({"task_id": 5, "other_task_id": 5, "relation_kind": "related"}),
        )
        .await
        .unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error from {tool}, got {err:?}");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(data.message.contains("other_task_id"));
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_reject_invalid_kind() {
    let client = connect("http://127.0.0.1:1").await;
    for kind in ["unknown", "blocks", ""] {
        let err = call(
            &client,
            "vikunja_task_relations_create",
            json!({"task_id": 5, "other_task_id": 9, "relation_kind": kind}),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ServiceError::McpError(_)),
            "kind {kind:?} should be rejected, got {err:?}"
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_reject_nonpositive_ids() {
    let client = connect("http://127.0.0.1:1").await;
    for (tool, args) in [
        (
            "vikunja_task_relations_create",
            json!({"task_id": 0, "other_task_id": 9, "relation_kind": "related"}),
        ),
        (
            "vikunja_task_relations_create",
            json!({"task_id": 5, "other_task_id": -2, "relation_kind": "related"}),
        ),
        (
            "vikunja_task_relations_delete",
            json!({"task_id": -1, "other_task_id": 9, "relation_kind": "related"}),
        ),
        (
            "vikunja_task_relations_delete",
            json!({"task_id": 5, "other_task_id": 0, "relation_kind": "related"}),
        ),
        ("vikunja_task_relations_list", json!({"task_id": 0})),
    ] {
        let err = call(&client, tool, args).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error from {tool}");
        };
        assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(data.message.contains("task_id"));
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_tool_annotations() {
    let client = connect("http://127.0.0.1:1").await;
    let tools = client.list_all_tools().await.unwrap();

    let list_tool = tools
        .iter()
        .find(|t| t.name == "vikunja_task_relations_list")
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
        .find(|t| t.name == "vikunja_task_relations_delete")
        .unwrap();
    assert_eq!(
        delete_tool
            .annotations
            .as_ref()
            .and_then(|a| a.destructive_hint),
        Some(true)
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn task_relations_create_surfaces_already_exists_error() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/5/relations"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "code": 4012, "message": "The task relation already exists."
        })))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let err = call(
        &client,
        "vikunja_task_relations_create",
        json!({"task_id": 5, "other_task_id": 9, "relation_kind": "blocking"}),
    )
    .await
    .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error, got {err:?}");
    };
    // A conflict is user-correctable, not a server failure: it must surface
    // as invalid_params, like the other request-level errors.
    assert_eq!(data.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("already exists"));
    let details = data.data.expect("error data");
    assert_eq!(details["http_status"], 409);
    assert_eq!(details["kind"], "conflict");
    assert_eq!(details["vikunja_error_code"], 4012);
    assert_eq!(details["endpoint"], "task_relations.create");
    client.cancel().await.unwrap();
}

// ----- auto-pagination through the tool layer ---------------------------------

/// Mounts one page of a paginated list endpoint that reports `total` pages.
async fn mount_page(
    server: &MockServer,
    endpoint_path: &str,
    page: u32,
    total: u32,
    body: serde_json::Value,
) {
    Mock::given(method("GET"))
        .and(path(endpoint_path))
        .and(query_param("page", page.to_string()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body)
                .insert_header("x-pagination-total-pages", total.to_string()),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_tools_publish_auto_pagination_args() {
    let client = connect("http://127.0.0.1:1").await;
    let tools = client.list_all_tools().await.unwrap();
    for name in [
        "vikunja_projects_list",
        "vikunja_tasks_list",
        "vikunja_labels_list",
        "vikunja_task_attachments_list",
        "vikunja_teams_list",
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        for arg in ["auto_paginate", "max_pages"] {
            assert!(
                properties.contains_key(arg),
                "tool {name} is missing the {arg} argument"
            );
        }
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tasks_list_auto_paginate_walks_pages_preserving_args() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", page.to_string()))
            .and(query_param("per_page", "2"))
            .and(query_param("s", "report"))
            .and(query_param("filter", "(done = false) && project_id = 4"))
            .and(query_param("sort_by", "priority"))
            .and(query_param("order_by", "desc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id": page, "title": format!("T{page}"), "done": false, "project_id": 4}]))
                    .insert_header("x-pagination-total-pages", "2"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_tasks_list",
        json!({
            "auto_paginate": true,
            "per_page": 2,
            "search": "report",
            "filter": "done = false",
            "project_id": 4,
            "sort_by": "priority",
            "order_by": "desc"
        }),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(body["tasks"][1]["title"], "T2");
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    assert_eq!(body["auto_pagination"]["page_cap"], 10);
    assert_eq!(body["auto_pagination"]["truncated"], false);
    assert_eq!(body["auto_pagination"]["count"], 2);
    assert_eq!(body["pagination"]["page"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn projects_list_auto_paginate_reports_truncation_at_cap() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        mount_page(
            &server,
            "/api/v1/projects",
            page,
            5,
            json!([{"id": page, "title": format!("P{page}")}]),
        )
        .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_projects_list",
        json!({"auto_paginate": true, "max_pages": 2}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["projects"].as_array().unwrap().len(), 2);
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    assert_eq!(body["auto_pagination"]["page_cap"], 2);
    assert_eq!(body["auto_pagination"]["truncated"], true);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn labels_list_auto_paginate_walks_pages() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/labels"))
            .and(query_param("page", page.to_string()))
            .and(query_param("s", "urgent"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id": page, "title": format!("L{page}")}]))
                    .insert_header("x-pagination-total-pages", "2"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_labels_list",
        json!({"auto_paginate": true, "search": "urgent"}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["labels"].as_array().unwrap().len(), 2);
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    assert_eq!(body["auto_pagination"]["truncated"], false);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn attachments_list_auto_paginate_walks_pages() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        mount_page(
            &server,
            "/api/v1/tasks/7/attachments",
            page,
            2,
            json!([{"id": page, "task_id": 7}]),
        )
        .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_task_attachments_list",
        json!({"task_id": 7, "auto_paginate": true}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn teams_list_auto_paginate_walks_pages() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        mount_page(
            &server,
            "/api/v1/teams",
            page,
            2,
            json!([{"id": page, "name": format!("team{page}")}]),
        )
        .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_teams_list",
        json!({"auto_paginate": true}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["teams"].as_array().unwrap().len(), 2);
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn project_teams_list_auto_paginate_walks_pages() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        mount_page(
            &server,
            "/api/v1/projects/5/teams",
            page,
            2,
            json!([{"id": page, "name": format!("team{page}"), "permission": 1}]),
        )
        .await;
    }

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_teams_list",
        json!({"project_id": 5, "auto_paginate": true}),
    )
    .await
    .unwrap();

    let body = structured(&result);
    assert_eq!(body["teams"].as_array().unwrap().len(), 2);
    assert_eq!(body["teams"][0]["permission"], 1);
    assert_eq!(body["auto_pagination"]["pages_read"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn auto_pagination_validation_rejects_bad_arguments_before_any_request() {
    // The unreachable server makes any HTTP attempt fail loudly, so an
    // invalid-params error proves validation fired first.
    let client = connect("http://127.0.0.1:1").await;

    let cases = [
        (
            "vikunja_tasks_list",
            json!({"auto_paginate": true, "max_pages": 0}),
            "max_pages",
        ),
        (
            "vikunja_tasks_list",
            json!({"auto_paginate": true, "max_pages": 51}),
            "max_pages",
        ),
        (
            "vikunja_tasks_list",
            json!({"max_pages": 3}),
            "auto_paginate",
        ),
        (
            "vikunja_tasks_list",
            json!({"auto_paginate": false, "max_pages": 3}),
            "auto_paginate",
        ),
        (
            "vikunja_tasks_list",
            json!({"auto_paginate": true, "page": 2}),
            "page",
        ),
        (
            "vikunja_projects_list",
            json!({"auto_paginate": true, "max_pages": 0}),
            "max_pages",
        ),
        (
            "vikunja_labels_list",
            json!({"auto_paginate": true, "max_pages": 99}),
            "max_pages",
        ),
        (
            "vikunja_teams_list",
            json!({"max_pages": 3}),
            "auto_paginate",
        ),
        (
            "vikunja_task_attachments_list",
            json!({"task_id": 7, "auto_paginate": true, "page": 2}),
            "page",
        ),
    ];
    for (tool, args, needle) in cases {
        let err = call(&client, tool, args.clone()).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool} {args}");
        };
        assert_eq!(
            data.code,
            rmcp::model::ErrorCode::INVALID_PARAMS,
            "{tool} {args}"
        );
        assert!(
            data.message.contains(needle),
            "{tool} {args}: {}",
            data.message
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn one_page_lists_omit_auto_pagination_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 1, "title": "Inbox"}]))
                .insert_header("x-pagination-total-pages", "3"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    // No auto_paginate: exactly one page is fetched and the response keeps
    // its original shape, without any auto_pagination block.
    let result = call(&client, "vikunja_projects_list", json!({}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["projects"].as_array().unwrap().len(), 1);
    assert_eq!(body["pagination"]["has_more"], true);
    assert!(
        body.get("auto_pagination").is_none(),
        "one-page responses must not grow an auto_pagination block: {body}"
    );
    client.cancel().await.unwrap();
}

// ----- kanban buckets -------------------------------------------------------------

fn buckets_json() -> serde_json::Value {
    json!([
        {
            "id": 1, "title": "Backlog", "project_view_id": 4,
            "limit": 0, "count": 1, "position": 100,
            "tasks": [{"id": 11, "title": "Plan the thing", "project_id": 7, "bucket_id": 1}]
        },
        {
            "id": 2, "title": "Doing", "project_view_id": 4,
            "limit": 3, "count": 1, "position": 200,
            "tasks": [{"id": 12, "title": "Build the thing", "project_id": 7, "bucket_id": 2}]
        },
    ])
}

fn views_json() -> serde_json::Value {
    json!([
        {"id": 1, "title": "List", "project_id": 7, "view_kind": "list"},
        {"id": 4, "title": "Kanban", "project_id": 7, "view_kind": "kanban",
         "default_bucket_id": 1, "done_bucket_id": 2},
    ])
}

#[tokio::test]
async fn project_views_list_returns_views() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views"))
        .respond_with(ResponseTemplate::new(200).set_body_json(views_json()))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_project_views_list",
        json!({"project_id": 7}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["views"].as_array().unwrap().len(), 2);
    assert_eq!(body["views"][1]["view_kind"], "kanban");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn buckets_list_resolves_kanban_view_automatically() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views"))
        .respond_with(ResponseTemplate::new(200).set_body_json(views_json()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views/4/buckets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(buckets_json()))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(&client, "vikunja_buckets_list", json!({"project_id": 7}))
        .await
        .unwrap();
    let body = structured(&result);
    assert_eq!(body["project_id"], 7);
    assert_eq!(body["view_id"], 4);
    assert_eq!(body["view_title"], "Kanban");
    let buckets = body["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0]["title"], "Backlog");
    assert_eq!(buckets[0]["is_default_bucket"], true);
    assert_eq!(buckets[1]["title"], "Doing");
    assert_eq!(buckets[1]["is_done_bucket"], true);
    assert_eq!(buckets[1]["tasks"][0]["title"], "Build the thing");
    assert_eq!(buckets[1]["tasks"][0]["bucket_id"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn buckets_list_uses_explicit_view_id_without_views_call() {
    let server = MockServer::start().await;
    // No /views mock: an explicit view_id must skip view resolution.
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views/4/buckets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(buckets_json()))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let result = call(
        &client,
        "vikunja_buckets_list",
        json!({"project_id": 7, "view_id": 4}),
    )
    .await
    .unwrap();
    let body = structured(&result);
    assert_eq!(body["view_id"], 4);
    // Without the views listing the title is unknown (omitted) and the
    // done/default flags are unknown -> false.
    assert!(body.get("view_title").is_none());
    assert_eq!(body["buckets"][1]["is_done_bucket"], false);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn buckets_list_errors_when_project_has_no_kanban_view() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "title": "List", "project_id": 7, "view_kind": "list"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;
    let err = call(&client, "vikunja_buckets_list", json!({"project_id": 7}))
        .await
        .unwrap_err();
    let ServiceError::McpError(data) = err else {
        panic!("expected MCP error");
    };
    assert!(data.message.contains("kanban"), "{}", data.message);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bucket_tools_validate_ids_before_any_request() {
    let client = connect("http://127.0.0.1:1").await;
    for (tool, args) in [
        ("vikunja_project_views_list", json!({"project_id": 0})),
        ("vikunja_buckets_list", json!({"project_id": -7})),
        (
            "vikunja_buckets_list",
            json!({"project_id": 7, "view_id": 0}),
        ),
    ] {
        let err = call(&client, tool, args.clone()).await.unwrap_err();
        let ServiceError::McpError(data) = err else {
            panic!("expected MCP error for {tool} {args}");
        };
        assert!(data.message.contains("positive"), "{}", data.message);
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn buckets_resource_template_reads_project_buckets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views"))
        .respond_with(ResponseTemplate::new(200).set_body_json(views_json()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/views/4/buckets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(buckets_json()))
        .mount(&server)
        .await;

    let client = connect(&server.uri()).await;

    let templates = client.list_resource_templates(None).await.unwrap();
    let template_uris: Vec<&str> = templates
        .resource_templates
        .iter()
        .map(|t| t.raw.uri_template.as_str())
        .collect();
    assert!(template_uris.contains(&"vikunja://projects/{id}/buckets"));

    let result = client
        .read_resource(ReadResourceRequestParams::new(
            "vikunja://projects/7/buckets",
        ))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
        panic!("expected text contents");
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["view_id"], 4);
    assert_eq!(parsed["buckets"][1]["title"], "Doing");
    client.cancel().await.unwrap();
}
