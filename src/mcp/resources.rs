//! Read-only MCP resources: server status, project/task lists and
//! individual entities via URI templates. All resources render as
//! `application/json`.

use rmcp::ErrorData as McpError;
use rmcp::model::{
    AnnotateAble, RawResource, RawResourceTemplate, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate,
};
use serde::Serialize;
use serde_json::json;

use crate::vikunja::VikunjaClient;

pub const STATUS_URI: &str = "vikunja://status";
pub const PROJECTS_URI: &str = "vikunja://projects";
pub const TASKS_URI: &str = "vikunja://tasks";
pub const PROJECT_URI_PREFIX: &str = "vikunja://projects/";
pub const TASK_URI_PREFIX: &str = "vikunja://tasks/";

/// Cap on auto-pagination when rendering list resources, to keep resource
/// reads bounded. With the default page size of 50 this covers 500 items.
const MAX_RESOURCE_PAGES: u32 = 10;

/// Static resources advertised by `resources/list`.
pub fn list() -> Vec<Resource> {
    vec![
        RawResource::new(STATUS_URI, "vikunja-status")
            .with_title("Vikunja server status")
            .with_description(
                "Connection status and configuration summary of this MCP server (no secrets).",
            )
            .with_mime_type("application/json")
            .no_annotation(),
        RawResource::new(PROJECTS_URI, "vikunja-projects")
            .with_title("All Vikunja projects")
            .with_description("All projects visible to the configured API token.")
            .with_mime_type("application/json")
            .no_annotation(),
        RawResource::new(TASKS_URI, "vikunja-tasks")
            .with_title("All Vikunja tasks")
            .with_description(
                "Tasks visible to the configured API token (bounded to the first 10 pages).",
            )
            .with_mime_type("application/json")
            .no_annotation(),
    ]
}

/// URI templates advertised by `resources/templates/list`.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        RawResourceTemplate::new("vikunja://projects/{id}", "vikunja-project")
            .with_title("A single Vikunja project")
            .with_description("One project by numeric id.")
            .with_mime_type("application/json")
            .no_annotation(),
        RawResourceTemplate::new("vikunja://tasks/{id}", "vikunja-task")
            .with_title("A single Vikunja task")
            .with_description("One task by numeric id, including labels and assignees.")
            .with_mime_type("application/json")
            .no_annotation(),
    ]
}

/// Resolves and renders a resource URI.
pub async fn read(client: &VikunjaClient, uri: &str) -> Result<ReadResourceResult, McpError> {
    match uri {
        STATUS_URI => status(client).await,
        PROJECTS_URI => projects(client).await,
        TASKS_URI => tasks(client).await,
        _ => {
            if let Some(id) = parse_id(uri, PROJECT_URI_PREFIX) {
                let project = client.get_project(id).await.map_err(|e| e.to_mcp())?;
                return Ok(json_result(uri, &project));
            }
            if let Some(id) = parse_id(uri, TASK_URI_PREFIX) {
                let task = client.get_task(id).await.map_err(|e| e.to_mcp())?;
                return Ok(json_result(uri, &task));
            }
            Err(McpError::resource_not_found(
                format!("unknown resource URI: {uri}"),
                Some(json!({ "uri": uri })),
            ))
        }
    }
}

async fn status(client: &VikunjaClient) -> Result<ReadResourceResult, McpError> {
    let connectivity = match client.probe().await {
        Ok(status) => json!({ "ok": true, "http_status": status.as_u16() }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    };
    let body = json!({
        "server": {
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
        },
        "vikunja_url": client.base_url().as_str(),
        "default_page_size": client.default_page_size(),
        "auth": "api-token (redacted)",
        "connectivity": connectivity,
    });
    Ok(json_result(STATUS_URI, &body))
}

async fn projects(client: &VikunjaClient) -> Result<ReadResourceResult, McpError> {
    let projects = client
        .list_all_projects(MAX_RESOURCE_PAGES)
        .await
        .map_err(|e| e.to_mcp())?;
    let body = json!({ "count": projects.len(), "projects": projects });
    Ok(json_result(PROJECTS_URI, &body))
}

async fn tasks(client: &VikunjaClient) -> Result<ReadResourceResult, McpError> {
    let tasks = client
        .list_all_tasks(MAX_RESOURCE_PAGES)
        .await
        .map_err(|e| e.to_mcp())?;
    let body = json!({ "count": tasks.len(), "tasks": tasks });
    Ok(json_result(TASKS_URI, &body))
}

fn parse_id(uri: &str, prefix: &str) -> Option<i64> {
    uri.strip_prefix(prefix)
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|id| *id > 0)
}

fn json_result(uri: &str, value: &impl Serialize) -> ReadResourceResult {
    let text = serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| format!("{{\"error\": \"failed to serialize resource: {e}\"}}"));
    ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
        uri: uri.to_string(),
        mime_type: Some("application/json".to_string()),
        text,
        meta: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_resources_are_advertised() {
        let resources = list();
        let uris: Vec<&str> = resources.iter().map(|r| r.raw.uri.as_str()).collect();
        assert_eq!(uris, vec![STATUS_URI, PROJECTS_URI, TASKS_URI]);
        assert!(
            resources
                .iter()
                .all(|r| r.raw.mime_type.as_deref() == Some("application/json"))
        );
    }

    #[test]
    fn templates_are_advertised() {
        let templates = templates();
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].raw.uri_template, "vikunja://projects/{id}");
        assert_eq!(templates[1].raw.uri_template, "vikunja://tasks/{id}");
    }

    #[test]
    fn parse_id_accepts_only_positive_integers() {
        assert_eq!(parse_id("vikunja://tasks/42", TASK_URI_PREFIX), Some(42));
        assert_eq!(parse_id("vikunja://tasks/0", TASK_URI_PREFIX), None);
        assert_eq!(parse_id("vikunja://tasks/-3", TASK_URI_PREFIX), None);
        assert_eq!(parse_id("vikunja://tasks/abc", TASK_URI_PREFIX), None);
        assert_eq!(parse_id("vikunja://other/42", TASK_URI_PREFIX), None);
    }

    #[test]
    fn json_result_sets_mime_type() {
        let result = json_result("vikunja://status", &serde_json::json!({"a": 1}));
        match &result.contents[0] {
            ResourceContents::TextResourceContents {
                uri,
                mime_type,
                text,
                ..
            } => {
                assert_eq!(uri, "vikunja://status");
                assert_eq!(mime_type.as_deref(), Some("application/json"));
                assert!(text.contains("\"a\": 1"));
            }
            other => panic!("unexpected contents: {other:?}"),
        }
    }
}
