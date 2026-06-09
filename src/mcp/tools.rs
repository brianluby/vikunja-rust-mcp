//! MCP tool definitions: argument schemas, output shapes and the mapping of
//! each tool onto [`VikunjaClient`] calls. All tools return structured JSON
//! via `rmcp`'s `Json` wrapper, which also publishes an output schema.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ErrorData as McpError, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::vikunja::client::TaskListOptions;
use crate::vikunja::models::{
    Label, LabelCreate, LabelUpdate, Project, ProjectCreate, ProjectUpdate, Task, TaskAttachment,
    TaskComment, TaskCreate, TaskUpdate, Team, User,
};
use crate::vikunja::pagination::{PageInfo, PageParams};

use super::server::VikunjaMcpServer;

/// Largest attachment returned inline as base64 (bytes). Bigger files must
/// be saved with `save_path`.
pub const MAX_INLINE_DOWNLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Largest accepted upload (decoded bytes).
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

// ----- Shared argument/output shapes ----------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectsListArgs {
    /// Search projects by title.
    pub search: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page; the Vikunja server caps this (50 by default).
    pub per_page: Option<u32>,
    /// If true, return archived projects instead of active ones.
    pub is_archived: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectIdArgs {
    /// Numeric id of the project.
    pub project_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectsCreateArgs {
    /// Project title.
    pub title: String,
    /// Project description.
    pub description: Option<String>,
    /// Hex color without the leading `#`, e.g. `e8b71c`.
    pub hex_color: Option<String>,
    /// Id of the parent project for nesting.
    pub parent_project_id: Option<i64>,
    /// Short identifier prefix used in task numbers, e.g. `PROJ`.
    pub identifier: Option<String>,
    /// Mark the project as a favorite.
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectsUpdateArgs {
    /// Numeric id of the project to update.
    pub project_id: i64,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New hex color without the leading `#`.
    pub hex_color: Option<String>,
    /// Move under a different parent project.
    pub parent_project_id: Option<i64>,
    /// New identifier prefix.
    pub identifier: Option<String>,
    /// Archive (true) or unarchive (false) the project.
    pub is_archived: Option<bool>,
    /// Favorite (true) or unfavorite (false) the project.
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TasksListArgs {
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page; the Vikunja server caps this (50 by default).
    pub per_page: Option<u32>,
    /// Search tasks by title.
    pub search: Option<String>,
    /// Vikunja filter expression, e.g. `done = false && priority >= 3` or
    /// `due_date < now/d+7d`. See https://vikunja.io/docs/filters.
    pub filter: Option<String>,
    /// Only return tasks from this project.
    pub project_id: Option<i64>,
    /// Field to sort by, e.g. `due_date`, `priority`, `created`.
    pub sort_by: Option<String>,
    /// Sort direction: `asc` or `desc`.
    pub order_by: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskIdArgs {
    /// Numeric id of the task (not the `PROJ-12` identifier).
    pub task_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TasksCreateArgs {
    /// Id of the project the task is created in.
    pub project_id: i64,
    /// Task title.
    pub title: String,
    /// Task description (may contain HTML).
    pub description: Option<String>,
    /// Due date as RFC 3339, e.g. `2026-07-01T12:00:00Z`.
    pub due_date: Option<String>,
    /// Start date as RFC 3339.
    pub start_date: Option<String>,
    /// End date as RFC 3339.
    pub end_date: Option<String>,
    /// Priority: 0 unset, 1 low, 2 medium, 3 high, 4 urgent, 5 DO NOW.
    pub priority: Option<i64>,
    /// Completion fraction between 0 and 1 (0.5 = 50%).
    pub percent_done: Option<f64>,
    /// Hex color without the leading `#`.
    pub hex_color: Option<String>,
    /// Mark the task as favorite.
    pub is_favorite: Option<bool>,
    /// Repeat interval in seconds after the task is completed.
    pub repeat_after: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TasksUpdateArgs {
    /// Numeric id of the task to update.
    pub task_id: i64,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// Set done state. Prefer vikunja_tasks_complete / vikunja_tasks_reopen.
    pub done: Option<bool>,
    /// New due date as RFC 3339; `0001-01-01T00:00:00Z` clears it.
    pub due_date: Option<String>,
    /// New start date as RFC 3339.
    pub start_date: Option<String>,
    /// New end date as RFC 3339.
    pub end_date: Option<String>,
    /// Priority: 0 unset, 1 low, 2 medium, 3 high, 4 urgent, 5 DO NOW.
    pub priority: Option<i64>,
    /// Completion fraction between 0 and 1.
    pub percent_done: Option<f64>,
    /// Move the task to another project.
    pub project_id: Option<i64>,
    /// New hex color without the leading `#`.
    pub hex_color: Option<String>,
    /// Favorite (true) or unfavorite (false).
    pub is_favorite: Option<bool>,
    /// Repeat interval in seconds.
    pub repeat_after: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskUserArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// Numeric id of the user (find it with vikunja_users_search).
    pub user_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelsListArgs {
    /// Search labels by title.
    pub search: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelsCreateArgs {
    /// Label title.
    pub title: String,
    /// Label description.
    pub description: Option<String>,
    /// Hex color without the leading `#`, e.g. `ff0000`.
    pub hex_color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelsUpdateArgs {
    /// Numeric id of the label to update.
    pub label_id: i64,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New hex color without the leading `#`.
    pub hex_color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelIdArgs {
    /// Numeric id of the label.
    pub label_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskLabelArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// Numeric id of the label (find it with vikunja_labels_list).
    pub label_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentsCreateArgs {
    /// Numeric id of the task to comment on.
    pub task_id: i64,
    /// Comment text.
    pub comment: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentsUpdateArgs {
    /// Numeric id of the task the comment belongs to.
    pub task_id: i64,
    /// Numeric id of the comment.
    pub comment_id: i64,
    /// Replacement comment text.
    pub comment: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentIdArgs {
    /// Numeric id of the task the comment belongs to.
    pub task_id: i64,
    /// Numeric id of the comment.
    pub comment_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentsListArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentsUploadArgs {
    /// Numeric id of the task to attach the file to.
    pub task_id: i64,
    /// File name to store the attachment under, e.g. `notes.txt`.
    /// Required with `content_base64`; defaults to the basename of
    /// `file_path` otherwise.
    pub file_name: Option<String>,
    /// File content encoded as standard base64. Mutually exclusive with
    /// `file_path`.
    pub content_base64: Option<String>,
    /// Path to a local file readable by the machine running this MCP
    /// server. Mutually exclusive with `content_base64`.
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentsDownloadArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// Numeric id of the attachment (find it with
    /// vikunja_task_attachments_list).
    pub attachment_id: i64,
    /// Optional path on the machine running this MCP server to save the
    /// file to. Without it, contents up to 2 MiB are returned as base64.
    pub save_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentIdArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// Numeric id of the attachment.
    pub attachment_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UsersSearchArgs {
    /// Search term matching username, name or email. Most instances
    /// require at least a partial username.
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamsListArgs {
    /// Search teams by name.
    pub search: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
    /// If set, list the teams with access to this project (including their
    /// permission level) instead of all teams.
    pub project_id: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectListResult {
    pub projects: Vec<Project>,
    pub pagination: PageInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TaskListResult {
    pub tasks: Vec<Task>,
    pub pagination: PageInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelListResult {
    pub labels: Vec<Label>,
    pub pagination: PageInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TeamListResult {
    pub teams: Vec<Team>,
    pub pagination: PageInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UserListResult {
    pub users: Vec<User>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommentListResult {
    pub comments: Vec<TaskComment>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachmentListResult {
    pub attachments: Vec<TaskAttachment>,
    pub pagination: PageInfo,
}

/// Result of a mutation that only returns a confirmation message.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OperationResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadResult {
    pub ok: bool,
    pub message: String,
    /// The uploaded attachment, when it could be identified after upload.
    pub attachment: Option<TaskAttachment>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadResult {
    pub task_id: i64,
    pub attachment_id: i64,
    /// Content type reported by Vikunja.
    pub mime: Option<String>,
    pub size_bytes: u64,
    /// Where the file was written, when `save_path` was given.
    pub saved_to: Option<String>,
    /// Base64 file content, when no `save_path` was given.
    pub content_base64: Option<String>,
}

// ----- Argument validation helpers -------------------------------------------

fn positive(name: &str, value: i64) -> Result<i64, McpError> {
    if value <= 0 {
        return Err(Error::InvalidArgument(format!("{name} must be a positive integer")).to_mcp());
    }
    Ok(value)
}

fn page_params(page: Option<u32>, per_page: Option<u32>) -> Result<PageParams, McpError> {
    if page == Some(0) {
        return Err(Error::InvalidArgument("page must be >= 1".to_string()).to_mcp());
    }
    if per_page == Some(0) {
        return Err(Error::InvalidArgument("per_page must be >= 1".to_string()).to_mcp());
    }
    Ok(PageParams::new(page, per_page))
}

fn non_empty(name: &str, value: &str) -> Result<(), McpError> {
    if value.trim().is_empty() {
        return Err(Error::InvalidArgument(format!("{name} must not be empty")).to_mcp());
    }
    Ok(())
}

fn oversized_upload(size: usize) -> Error {
    Error::InvalidArgument(format!(
        "attachment is about {size} bytes; the maximum supported upload is {MAX_UPLOAD_BYTES} bytes"
    ))
}

// ----- Tools -----------------------------------------------------------------

#[tool_router(vis = "pub(crate)")]
impl VikunjaMcpServer {
    // -- Projects --

    #[tool(
        name = "vikunja_projects_list",
        description = "List or search Vikunja projects the user has access to. Returns one page of projects plus pagination info.",
        annotations(read_only_hint = true)
    )]
    pub async fn projects_list(
        &self,
        Parameters(args): Parameters<ProjectsListArgs>,
    ) -> Result<Json<ProjectListResult>, McpError> {
        let params = page_params(args.page, args.per_page)?;
        let page = self
            .client()
            .list_projects(params, args.search.as_deref(), args.is_archived)
            .await?;
        Ok(Json(ProjectListResult {
            projects: page.items,
            pagination: page.info,
        }))
    }

    #[tool(
        name = "vikunja_projects_get",
        description = "Get one Vikunja project by id.",
        annotations(read_only_hint = true)
    )]
    pub async fn projects_get(
        &self,
        Parameters(args): Parameters<ProjectIdArgs>,
    ) -> Result<Json<Project>, McpError> {
        let id = positive("project_id", args.project_id)?;
        Ok(Json(self.client().get_project(id).await?))
    }

    #[tool(
        name = "vikunja_projects_create",
        description = "Create a new Vikunja project. Colors are hex strings without '#', e.g. 'e8b71c'."
    )]
    pub async fn projects_create(
        &self,
        Parameters(args): Parameters<ProjectsCreateArgs>,
    ) -> Result<Json<Project>, McpError> {
        non_empty("title", &args.title)?;
        let body = ProjectCreate {
            title: args.title,
            description: args.description,
            hex_color: args.hex_color,
            parent_project_id: args.parent_project_id,
            identifier: args.identifier,
            is_favorite: args.is_favorite,
        };
        Ok(Json(self.client().create_project(&body).await?))
    }

    #[tool(
        name = "vikunja_projects_update",
        description = "Update fields of a Vikunja project. Only provided fields change; others keep their current value. Can also archive/unarchive via is_archived.",
        annotations(idempotent_hint = true)
    )]
    pub async fn projects_update(
        &self,
        Parameters(args): Parameters<ProjectsUpdateArgs>,
    ) -> Result<Json<Project>, McpError> {
        let id = positive("project_id", args.project_id)?;
        let patch = ProjectUpdate {
            title: args.title,
            description: args.description,
            hex_color: args.hex_color,
            parent_project_id: args.parent_project_id,
            identifier: args.identifier,
            is_archived: args.is_archived,
            is_favorite: args.is_favorite,
        };
        Ok(Json(self.client().update_project(id, &patch).await?))
    }

    #[tool(
        name = "vikunja_projects_delete",
        description = "Delete a Vikunja project and all of its tasks. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn projects_delete(
        &self,
        Parameters(args): Parameters<ProjectIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let id = positive("project_id", args.project_id)?;
        let message = self.client().delete_project(id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Tasks --

    #[tool(
        name = "vikunja_tasks_list",
        description = "List or search Vikunja tasks across all projects, or within one project via project_id. Supports Vikunja filter expressions (e.g. 'done = false && priority >= 3', 'due_date < now/d+7d') and sorting.",
        annotations(read_only_hint = true)
    )]
    pub async fn tasks_list(
        &self,
        Parameters(args): Parameters<TasksListArgs>,
    ) -> Result<Json<TaskListResult>, McpError> {
        page_params(args.page, args.per_page)?;
        if let Some(project_id) = args.project_id {
            positive("project_id", project_id)?;
        }
        let options = TaskListOptions {
            page: args.page,
            per_page: args.per_page,
            search: args.search,
            filter: args.filter,
            project_id: args.project_id,
            sort_by: args.sort_by,
            order_by: args.order_by,
        };
        let page = self.client().list_tasks(&options).await?;
        Ok(Json(TaskListResult {
            tasks: page.items,
            pagination: page.info,
        }))
    }

    #[tool(
        name = "vikunja_tasks_get",
        description = "Get one Vikunja task by numeric id, including labels and assignees.",
        annotations(read_only_hint = true)
    )]
    pub async fn tasks_get(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<Task>, McpError> {
        let id = positive("task_id", args.task_id)?;
        Ok(Json(self.client().get_task(id).await?))
    }

    #[tool(
        name = "vikunja_tasks_create",
        description = "Create a task in a Vikunja project. Dates are RFC 3339 (e.g. '2026-07-01T12:00:00Z'); priority is 0-5 (1 low ... 5 DO NOW)."
    )]
    pub async fn tasks_create(
        &self,
        Parameters(args): Parameters<TasksCreateArgs>,
    ) -> Result<Json<Task>, McpError> {
        let project_id = positive("project_id", args.project_id)?;
        non_empty("title", &args.title)?;
        let body = TaskCreate {
            title: args.title,
            description: args.description,
            due_date: args.due_date,
            start_date: args.start_date,
            end_date: args.end_date,
            priority: args.priority,
            percent_done: args.percent_done,
            hex_color: args.hex_color,
            is_favorite: args.is_favorite,
            repeat_after: args.repeat_after,
        };
        Ok(Json(self.client().create_task(project_id, &body).await?))
    }

    #[tool(
        name = "vikunja_tasks_update",
        description = "Update fields of a Vikunja task. Only provided fields change; others keep their current value. Use project_id to move the task to another project.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_update(
        &self,
        Parameters(args): Parameters<TasksUpdateArgs>,
    ) -> Result<Json<Task>, McpError> {
        let id = positive("task_id", args.task_id)?;
        if let Some(project_id) = args.project_id {
            positive("project_id", project_id)?;
        }
        let patch = TaskUpdate {
            title: args.title,
            description: args.description,
            done: args.done,
            due_date: args.due_date,
            start_date: args.start_date,
            end_date: args.end_date,
            priority: args.priority,
            percent_done: args.percent_done,
            project_id: args.project_id,
            hex_color: args.hex_color,
            is_favorite: args.is_favorite,
            repeat_after: args.repeat_after,
        };
        Ok(Json(self.client().update_task(id, &patch).await?))
    }

    #[tool(
        name = "vikunja_tasks_delete",
        description = "Delete a Vikunja task. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn tasks_delete(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let id = positive("task_id", args.task_id)?;
        let message = self.client().delete_task(id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    #[tool(
        name = "vikunja_tasks_complete",
        description = "Mark a Vikunja task as done. Repeating tasks reschedule themselves.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_complete(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<Task>, McpError> {
        let id = positive("task_id", args.task_id)?;
        Ok(Json(self.client().set_task_done(id, true).await?))
    }

    #[tool(
        name = "vikunja_tasks_reopen",
        description = "Mark a done Vikunja task as not done again.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_reopen(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<Task>, McpError> {
        let id = positive("task_id", args.task_id)?;
        Ok(Json(self.client().set_task_done(id, false).await?))
    }

    #[tool(
        name = "vikunja_tasks_assign",
        description = "Assign a user to a Vikunja task. Find user ids with vikunja_users_search."
    )]
    pub async fn tasks_assign(
        &self,
        Parameters(args): Parameters<TaskUserArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let user_id = positive("user_id", args.user_id)?;
        self.client().assign_user(task_id, user_id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: format!("user {user_id} assigned to task {task_id}"),
        }))
    }

    #[tool(
        name = "vikunja_tasks_unassign",
        description = "Remove a user assignment from a Vikunja task."
    )]
    pub async fn tasks_unassign(
        &self,
        Parameters(args): Parameters<TaskUserArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let user_id = positive("user_id", args.user_id)?;
        let message = self.client().unassign_user(task_id, user_id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Labels --

    #[tool(
        name = "vikunja_labels_list",
        description = "List or search the user's Vikunja labels.",
        annotations(read_only_hint = true)
    )]
    pub async fn labels_list(
        &self,
        Parameters(args): Parameters<LabelsListArgs>,
    ) -> Result<Json<LabelListResult>, McpError> {
        let params = page_params(args.page, args.per_page)?;
        let page = self
            .client()
            .list_labels(params, args.search.as_deref())
            .await?;
        Ok(Json(LabelListResult {
            labels: page.items,
            pagination: page.info,
        }))
    }

    #[tool(
        name = "vikunja_labels_create",
        description = "Create a new Vikunja label. Colors are hex strings without '#', e.g. 'ff0000'."
    )]
    pub async fn labels_create(
        &self,
        Parameters(args): Parameters<LabelsCreateArgs>,
    ) -> Result<Json<Label>, McpError> {
        non_empty("title", &args.title)?;
        let body = LabelCreate {
            title: args.title,
            description: args.description,
            hex_color: args.hex_color,
        };
        Ok(Json(self.client().create_label(&body).await?))
    }

    #[tool(
        name = "vikunja_labels_update",
        description = "Update a Vikunja label's title, description or color. Only provided fields change.",
        annotations(idempotent_hint = true)
    )]
    pub async fn labels_update(
        &self,
        Parameters(args): Parameters<LabelsUpdateArgs>,
    ) -> Result<Json<Label>, McpError> {
        let id = positive("label_id", args.label_id)?;
        let patch = LabelUpdate {
            title: args.title,
            description: args.description,
            hex_color: args.hex_color,
        };
        Ok(Json(self.client().update_label(id, &patch).await?))
    }

    #[tool(
        name = "vikunja_labels_delete",
        description = "Delete a Vikunja label. It is removed from all tasks. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn labels_delete(
        &self,
        Parameters(args): Parameters<LabelIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let id = positive("label_id", args.label_id)?;
        // This endpoint returns the deleted label, not a message.
        let label = self.client().delete_label(id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: format!("label {} (\"{}\") deleted", label.id, label.title),
        }))
    }

    // -- Task labels --

    #[tool(
        name = "vikunja_task_labels_add",
        description = "Add an existing label to a Vikunja task. Find label ids with vikunja_labels_list."
    )]
    pub async fn task_labels_add(
        &self,
        Parameters(args): Parameters<TaskLabelArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let label_id = positive("label_id", args.label_id)?;
        self.client().add_task_label(task_id, label_id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: format!("label {label_id} added to task {task_id}"),
        }))
    }

    #[tool(
        name = "vikunja_task_labels_remove",
        description = "Remove a label from a Vikunja task."
    )]
    pub async fn task_labels_remove(
        &self,
        Parameters(args): Parameters<TaskLabelArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let label_id = positive("label_id", args.label_id)?;
        let message = self.client().remove_task_label(task_id, label_id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Comments --

    #[tool(
        name = "vikunja_task_comments_list",
        description = "List all comments on a Vikunja task.",
        annotations(read_only_hint = true)
    )]
    pub async fn task_comments_list(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<CommentListResult>, McpError> {
        let id = positive("task_id", args.task_id)?;
        let comments = self.client().list_task_comments(id).await?;
        Ok(Json(CommentListResult { comments }))
    }

    #[tool(
        name = "vikunja_task_comments_create",
        description = "Add a comment to a Vikunja task."
    )]
    pub async fn task_comments_create(
        &self,
        Parameters(args): Parameters<CommentsCreateArgs>,
    ) -> Result<Json<TaskComment>, McpError> {
        let id = positive("task_id", args.task_id)?;
        non_empty("comment", &args.comment)?;
        Ok(Json(
            self.client().create_task_comment(id, &args.comment).await?,
        ))
    }

    #[tool(
        name = "vikunja_task_comments_update",
        description = "Edit an existing comment on a Vikunja task.",
        annotations(idempotent_hint = true)
    )]
    pub async fn task_comments_update(
        &self,
        Parameters(args): Parameters<CommentsUpdateArgs>,
    ) -> Result<Json<TaskComment>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let comment_id = positive("comment_id", args.comment_id)?;
        non_empty("comment", &args.comment)?;
        Ok(Json(
            self.client()
                .update_task_comment(task_id, comment_id, &args.comment)
                .await?,
        ))
    }

    #[tool(
        name = "vikunja_task_comments_delete",
        description = "Delete a comment from a Vikunja task. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn task_comments_delete(
        &self,
        Parameters(args): Parameters<CommentIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let comment_id = positive("comment_id", args.comment_id)?;
        let message = self
            .client()
            .delete_task_comment(task_id, comment_id)
            .await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Attachments --

    #[tool(
        name = "vikunja_task_attachments_list",
        description = "List the attachments of a Vikunja task.",
        annotations(read_only_hint = true)
    )]
    pub async fn task_attachments_list(
        &self,
        Parameters(args): Parameters<AttachmentsListArgs>,
    ) -> Result<Json<AttachmentListResult>, McpError> {
        let id = positive("task_id", args.task_id)?;
        let params = page_params(args.page, args.per_page)?;
        let page = self.client().list_task_attachments(id, params).await?;
        Ok(Json(AttachmentListResult {
            attachments: page.items,
            pagination: page.info,
        }))
    }

    #[tool(
        name = "vikunja_task_attachments_upload",
        description = "Upload a file as an attachment to a Vikunja task. Provide either content_base64 (with file_name) or file_path (a path local to the MCP server)."
    )]
    pub async fn task_attachments_upload(
        &self,
        Parameters(args): Parameters<AttachmentsUploadArgs>,
    ) -> Result<Json<UploadResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let (file_name, bytes) = match (args.content_base64, args.file_path) {
            (Some(_), Some(_)) => {
                return Err(Error::InvalidArgument(
                    "provide either content_base64 or file_path, not both".to_string(),
                )
                .to_mcp());
            }
            (None, None) => {
                return Err(Error::InvalidArgument(
                    "provide content_base64 or file_path".to_string(),
                )
                .to_mcp());
            }
            (Some(content), None) => {
                let Some(file_name) = args.file_name else {
                    return Err(Error::InvalidArgument(
                        "file_name is required with content_base64".to_string(),
                    )
                    .to_mcp());
                };
                let content = content.trim();
                // Reject before decoding: base64 decodes to ~3/4 of its
                // encoded length, so this bounds the allocation below.
                let estimated_size = content.len() / 4 * 3;
                if estimated_size > MAX_UPLOAD_BYTES {
                    return Err(oversized_upload(estimated_size).to_mcp());
                }
                let bytes = BASE64.decode(content).map_err(|e| {
                    Error::InvalidArgument(format!("content_base64 is not valid base64: {e}"))
                        .to_mcp()
                })?;
                (file_name, bytes)
            }
            (None, Some(path)) => {
                // Check the size before reading so a huge file is rejected
                // without buffering it.
                let metadata = tokio::fs::metadata(&path).await.map_err(|e| {
                    Error::Io {
                        detail: format!("could not read {path}: {e}"),
                    }
                    .to_mcp()
                })?;
                if metadata.len() > MAX_UPLOAD_BYTES as u64 {
                    return Err(oversized_upload(metadata.len() as usize).to_mcp());
                }
                let bytes = tokio::fs::read(&path).await.map_err(|e| {
                    Error::Io {
                        detail: format!("could not read {path}: {e}"),
                    }
                    .to_mcp()
                })?;
                let file_name = match args.file_name {
                    Some(name) => name,
                    None => std::path::Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "attachment".to_string()),
                };
                (file_name, bytes)
            }
        };
        non_empty("file_name", &file_name)?;
        // Re-check the actual byte count (the file may have grown since the
        // metadata call; the base64 check above is an estimate).
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(oversized_upload(bytes.len()).to_mcp());
        }

        let message = self
            .client()
            .upload_attachment(task_id, &file_name, bytes)
            .await?;

        // Best effort: identify the attachment we just created so the model
        // gets its id without another round trip.
        let attachment = self
            .client()
            .list_task_attachments(task_id, PageParams::default())
            .await
            .ok()
            .and_then(|page| {
                page.items
                    .into_iter()
                    .filter(|a| {
                        a.file
                            .as_ref()
                            .map(|f| f.name == file_name)
                            .unwrap_or(false)
                    })
                    .max_by_key(|a| a.id)
            });

        Ok(Json(UploadResult {
            ok: true,
            message: message.message,
            attachment,
        }))
    }

    #[tool(
        name = "vikunja_task_attachments_download",
        description = "Download a Vikunja task attachment. With save_path the file is written to disk on the MCP server machine; otherwise contents up to 2 MiB are returned base64-encoded."
    )]
    pub async fn task_attachments_download(
        &self,
        Parameters(args): Parameters<AttachmentsDownloadArgs>,
    ) -> Result<Json<DownloadResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let attachment_id = positive("attachment_id", args.attachment_id)?;

        match args.save_path {
            // Stream straight to disk: no size limit and no full buffering.
            Some(path) => {
                let (size_bytes, mime) = self
                    .client()
                    .download_attachment_to_file(task_id, attachment_id, &path)
                    .await?;
                Ok(Json(DownloadResult {
                    task_id,
                    attachment_id,
                    mime,
                    size_bytes,
                    saved_to: Some(path),
                    content_base64: None,
                }))
            }
            // Inline: the client aborts the download beyond the cap.
            None => {
                let content = self
                    .client()
                    .download_attachment(
                        task_id,
                        attachment_id,
                        MAX_INLINE_DOWNLOAD_BYTES as u64,
                    )
                    .await
                    .map_err(|err| match err {
                        Error::TooLarge { size, .. } => {
                            let reported = size
                                .map(|s| format!("attachment is {s} bytes, "))
                                .unwrap_or_else(|| "attachment is ".to_string());
                            Error::InvalidArgument(format!(
                                "{reported}larger than the {MAX_INLINE_DOWNLOAD_BYTES} byte inline limit; pass save_path to write it to disk instead"
                            ))
                            .to_mcp()
                        }
                        other => other.to_mcp(),
                    })?;
                let size_bytes = content.bytes.len() as u64;
                Ok(Json(DownloadResult {
                    task_id,
                    attachment_id,
                    mime: content.content_type,
                    size_bytes,
                    saved_to: None,
                    content_base64: Some(BASE64.encode(&content.bytes)),
                }))
            }
        }
    }

    #[tool(
        name = "vikunja_task_attachments_delete",
        description = "Delete an attachment from a Vikunja task. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn task_attachments_delete(
        &self,
        Parameters(args): Parameters<AttachmentIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let attachment_id = positive("attachment_id", args.attachment_id)?;
        let message = self
            .client()
            .delete_attachment(task_id, attachment_id)
            .await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Users & teams --

    #[tool(
        name = "vikunja_users_search",
        description = "Search Vikunja users (for assigning tasks). Returns ids, usernames and names.",
        annotations(read_only_hint = true)
    )]
    pub async fn users_search(
        &self,
        Parameters(args): Parameters<UsersSearchArgs>,
    ) -> Result<Json<UserListResult>, McpError> {
        let users = self.client().search_users(args.search.as_deref()).await?;
        Ok(Json(UserListResult { users }))
    }

    #[tool(
        name = "vikunja_teams_list",
        description = "List Vikunja teams the user belongs to, or — with project_id — the teams that have access to a project including their permission level (0 read, 1 write, 2 admin).",
        annotations(read_only_hint = true)
    )]
    pub async fn teams_list(
        &self,
        Parameters(args): Parameters<TeamsListArgs>,
    ) -> Result<Json<TeamListResult>, McpError> {
        let params = page_params(args.page, args.per_page)?;
        let page = match args.project_id {
            Some(project_id) => {
                let project_id = positive("project_id", project_id)?;
                self.client()
                    .list_project_teams(project_id, params, args.search.as_deref())
                    .await?
            }
            None => {
                self.client()
                    .list_teams(params, args.search.as_deref())
                    .await?
            }
        };
        Ok(Json(TeamListResult {
            teams: page.items,
            pagination: page.info,
        }))
    }
}
