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
use crate::vikunja::client::TaskListOptions;

pub const STATUS_URI: &str = "vikunja://status";
pub const PROJECTS_URI: &str = "vikunja://projects";
pub const TASKS_URI: &str = "vikunja://tasks";
pub const TASKS_TODAY_URI: &str = "vikunja://tasks/today";
pub const TASKS_OVERDUE_URI: &str = "vikunja://tasks/overdue";
pub const TASKS_UPCOMING_URI: &str = "vikunja://tasks/upcoming";
pub const TASKS_HIGH_PRIORITY_URI: &str = "vikunja://tasks/high-priority";
pub const TASKS_INBOX_URI: &str = "vikunja://tasks/inbox";
pub const TASKS_RECENTLY_UPDATED_URI: &str = "vikunja://tasks/recently-updated";
pub const PROJECT_URI_PREFIX: &str = "vikunja://projects/";
pub const TASK_URI_PREFIX: &str = "vikunja://tasks/";

/// Cap on auto-pagination when rendering list resources, to keep resource
/// reads bounded. With the default page size of 50 this covers 500 items.
const MAX_RESOURCE_PAGES: u32 = 10;

/// A prebuilt read-only task view: a fixed Vikunja filter expression plus
/// sort order, exposed as a `vikunja://tasks/<slug>` resource so agents can
/// read common planning views without writing filter syntax themselves.
///
/// Null-date semantics follow Vikunja >= 1.0 filter syntax, where
/// `due_date = null` / `due_date != null` match tasks without/with a due
/// date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskView {
    Today,
    Overdue,
    Upcoming,
    HighPriority,
    Inbox,
    RecentlyUpdated,
}

impl TaskView {
    const ALL: [TaskView; 6] = [
        TaskView::Today,
        TaskView::Overdue,
        TaskView::Upcoming,
        TaskView::HighPriority,
        TaskView::Inbox,
        TaskView::RecentlyUpdated,
    ];

    fn from_uri(uri: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.uri() == uri)
    }

    fn uri(self) -> &'static str {
        match self {
            TaskView::Today => TASKS_TODAY_URI,
            TaskView::Overdue => TASKS_OVERDUE_URI,
            TaskView::Upcoming => TASKS_UPCOMING_URI,
            TaskView::HighPriority => TASKS_HIGH_PRIORITY_URI,
            TaskView::Inbox => TASKS_INBOX_URI,
            TaskView::RecentlyUpdated => TASKS_RECENTLY_UPDATED_URI,
        }
    }

    /// Short identifier used in resource names and the `view` output field.
    fn slug(self) -> &'static str {
        match self {
            TaskView::Today => "today",
            TaskView::Overdue => "overdue",
            TaskView::Upcoming => "upcoming",
            TaskView::HighPriority => "high-priority",
            TaskView::Inbox => "inbox",
            TaskView::RecentlyUpdated => "recently-updated",
        }
    }

    fn title(self) -> &'static str {
        match self {
            TaskView::Today => "Tasks due today",
            TaskView::Overdue => "Overdue tasks",
            TaskView::Upcoming => "Tasks due in the next 7 days",
            TaskView::HighPriority => "High-priority open tasks",
            TaskView::Inbox => "Inbox: open tasks without a due date",
            TaskView::RecentlyUpdated => "Recently updated tasks",
        }
    }

    fn description(self) -> &'static str {
        match self {
            TaskView::Today => "Open tasks due today, sorted by due date (ascending).",
            TaskView::Overdue => {
                "Open tasks with a due date before today, sorted by due date (ascending)."
            }
            TaskView::Upcoming => {
                "Open tasks due within the next 7 days, starting today \
                 (includes today, excludes overdue), sorted by due date (ascending)."
            }
            TaskView::HighPriority => {
                "Open tasks with priority >= 3 (high/urgent/do-now), \
                 sorted by priority (descending)."
            }
            TaskView::Inbox => {
                "Open tasks without a due date (likely unplanned), \
                 sorted by last update (descending)."
            }
            TaskView::RecentlyUpdated => {
                "All tasks ordered by most recently updated first (includes done tasks)."
            }
        }
    }

    /// Vikunja filter expression for this view, if any.
    fn filter(self) -> Option<&'static str> {
        match self {
            TaskView::Today => Some("done = false && due_date >= now/d && due_date < now/d+1d"),
            TaskView::Overdue => Some("done = false && due_date < now/d && due_date != null"),
            TaskView::Upcoming => Some("done = false && due_date >= now/d && due_date < now/d+7d"),
            TaskView::HighPriority => Some("done = false && priority >= 3"),
            TaskView::Inbox => Some("done = false && due_date = null"),
            TaskView::RecentlyUpdated => None,
        }
    }

    fn sort_by(self) -> &'static str {
        match self {
            TaskView::Today | TaskView::Overdue | TaskView::Upcoming => "due_date",
            TaskView::HighPriority => "priority",
            TaskView::Inbox | TaskView::RecentlyUpdated => "updated",
        }
    }

    fn order_by(self) -> &'static str {
        match self {
            TaskView::Today | TaskView::Overdue | TaskView::Upcoming => "asc",
            TaskView::HighPriority | TaskView::Inbox | TaskView::RecentlyUpdated => "desc",
        }
    }
}

/// Static resources advertised by `resources/list`.
pub fn list() -> Vec<Resource> {
    let mut resources = vec![
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
    ];
    resources.extend(TaskView::ALL.into_iter().map(|view| {
        RawResource::new(view.uri(), format!("vikunja-tasks-{}", view.slug()))
            .with_title(view.title())
            .with_description(view.description())
            .with_mime_type("application/json")
            .no_annotation()
    }));
    resources
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
            if let Some(view) = TaskView::from_uri(uri) {
                return task_view(client, view).await;
            }
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

/// Renders a prebuilt task view: tasks matching the view's fixed filter and
/// sort order, bounded by [`MAX_RESOURCE_PAGES`], plus the view definition
/// and pagination metadata so consumers can tell when results were cut off.
async fn task_view(client: &VikunjaClient, view: TaskView) -> Result<ReadResourceResult, McpError> {
    let options = TaskListOptions {
        filter: view.filter().map(str::to_string),
        sort_by: Some(view.sort_by().to_string()),
        order_by: Some(view.order_by().to_string()),
        ..Default::default()
    };
    let result = client
        .list_all_tasks_with_options(&options, MAX_RESOURCE_PAGES)
        .await
        .map_err(|e| e.to_mcp())?;
    let body = json!({
        "view": view.slug(),
        "description": view.description(),
        "filter": view.filter(),
        "sort_by": view.sort_by(),
        "order_by": view.order_by(),
        "page_cap": MAX_RESOURCE_PAGES,
        "pages_read": result.pages_read,
        "truncated": result.truncated,
        "count": result.items.len(),
        "tasks": result.items,
    });
    Ok(json_result(view.uri(), &body))
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
        assert_eq!(
            uris,
            vec![
                STATUS_URI,
                PROJECTS_URI,
                TASKS_URI,
                TASKS_TODAY_URI,
                TASKS_OVERDUE_URI,
                TASKS_UPCOMING_URI,
                TASKS_HIGH_PRIORITY_URI,
                TASKS_INBOX_URI,
                TASKS_RECENTLY_UPDATED_URI,
            ]
        );
        assert!(
            resources
                .iter()
                .all(|r| r.raw.mime_type.as_deref() == Some("application/json"))
        );
        for resource in &resources {
            assert!(
                resource.raw.title.is_some(),
                "{} has no title",
                resource.raw.uri
            );
            assert!(
                resource
                    .raw
                    .description
                    .as_ref()
                    .is_some_and(|d| !d.is_empty()),
                "{} has no description",
                resource.raw.uri
            );
        }
    }

    #[test]
    fn task_view_uris_round_trip() {
        for view in TaskView::ALL {
            assert_eq!(TaskView::from_uri(view.uri()), Some(view));
        }
        assert_eq!(TaskView::from_uri("vikunja://tasks"), None);
        assert_eq!(TaskView::from_uri("vikunja://tasks/42"), None);
        assert_eq!(TaskView::from_uri("vikunja://tasks/bogus"), None);
    }

    #[test]
    fn task_view_definitions_do_not_drift() {
        let cases: &[(TaskView, &str, Option<&str>, &str, &str)] = &[
            (
                TaskView::Today,
                "vikunja://tasks/today",
                Some("done = false && due_date >= now/d && due_date < now/d+1d"),
                "due_date",
                "asc",
            ),
            (
                TaskView::Overdue,
                "vikunja://tasks/overdue",
                Some("done = false && due_date < now/d && due_date != null"),
                "due_date",
                "asc",
            ),
            (
                TaskView::Upcoming,
                "vikunja://tasks/upcoming",
                Some("done = false && due_date >= now/d && due_date < now/d+7d"),
                "due_date",
                "asc",
            ),
            (
                TaskView::HighPriority,
                "vikunja://tasks/high-priority",
                Some("done = false && priority >= 3"),
                "priority",
                "desc",
            ),
            (
                TaskView::Inbox,
                "vikunja://tasks/inbox",
                Some("done = false && due_date = null"),
                "updated",
                "desc",
            ),
            (
                TaskView::RecentlyUpdated,
                "vikunja://tasks/recently-updated",
                None,
                "updated",
                "desc",
            ),
        ];
        assert_eq!(cases.len(), TaskView::ALL.len());
        for (view, uri, filter, sort_by, order_by) in cases {
            assert_eq!(view.uri(), *uri);
            assert_eq!(view.filter(), *filter, "{uri} filter");
            assert_eq!(view.sort_by(), *sort_by, "{uri} sort_by");
            assert_eq!(view.order_by(), *order_by, "{uri} order_by");
            assert!(!view.slug().is_empty());
            assert!(!view.description().is_empty());
        }
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
