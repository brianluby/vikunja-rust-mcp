//! MCP tool definitions: argument schemas, output shapes and the mapping of
//! each tool onto [`VikunjaClient`] calls. All tools return structured JSON
//! via `rmcp`'s `Json` wrapper, which also publishes an output schema.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Local, SecondsFormat, TimeZone};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ErrorData as McpError, tool, tool_router};
use schemars::JsonSchema;
use schemars::transform::RecursiveTransform;
use serde::{Deserialize, Serialize};

use crate::dates::{self, DateConfig, Resolution};
use crate::error::{ApiErrorKind, Error};
use crate::schema::strip_unsigned_formats;
use crate::vikunja::VikunjaClient;
use crate::vikunja::client::{TaskListOptions, saved_filter_options};
use crate::vikunja::models::{
    Bucket, Label, LabelCreate, LabelUpdate, Project, ProjectCreate, ProjectUpdate, ProjectView,
    RelationKind, SavedFilter, SavedFilterCreate, SavedFilterQuery, SavedFilterSummary,
    SavedFilterUpdate, Task, TaskAttachment, TaskComment, TaskCreate, TaskRelation, TaskReminder,
    TaskUpdate, Team, User,
};
use crate::vikunja::pagination::{BoundedPage, PageInfo, PageParams, walk_pages};

use super::server::VikunjaMcpServer;

/// Largest attachment returned inline as base64 (bytes). Bigger files must
/// be saved with `save_path`.
pub const MAX_INLINE_DOWNLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Largest accepted upload (decoded bytes).
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
/// Largest number of task ids accepted by one bulk tool call. Bounds the
/// fan-out (each id can cost several Vikunja requests) so a single MCP call
/// cannot flood the instance or hold the connection open indefinitely.
pub const MAX_BULK_TASK_IDS: usize = 100;
/// Pages fetched by an auto-paginating list call when `max_pages` is not
/// given. With the default page size of 50 this covers 500 items.
pub const DEFAULT_AUTO_MAX_PAGES: u32 = 10;
/// Hard upper bound on `max_pages`, so one tool call can never trigger an
/// unbounded number of Vikunja requests.
pub const MAX_AUTO_MAX_PAGES: u32 = 50;
/// Page cap when walking the project list to enumerate saved filters
/// (Vikunja has no `GET /filters` list endpoint). With the default page
/// size of 50 this covers 500 entries.
pub const MAX_FILTER_LIST_PAGES: u32 = 10;
/// Page cap when walking a project's views to find its kanban view.
pub const MAX_VIEW_RESOLUTION_PAGES: u32 = 10;

// ----- Shared argument/output shapes ----------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct ProjectsListArgs {
    /// Search projects by title.
    pub search: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page; the Vikunja server caps this (50 by default).
    pub per_page: Option<u32>,
    /// If true, return archived projects instead of active ones.
    pub is_archived: Option<bool>,
    /// If true, fetch up to max_pages pages starting at page 1 and return
    /// them as one result with auto_pagination metadata. Cannot be combined
    /// with page.
    pub auto_paginate: Option<bool>,
    /// Page cap for auto_paginate: 1-50, default 10. Requires
    /// auto_paginate: true.
    pub max_pages: Option<u32>,
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
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
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
    /// If true, fetch up to max_pages pages starting at page 1 and return
    /// them as one result with auto_pagination metadata. Cannot be combined
    /// with page.
    pub auto_paginate: Option<bool>,
    /// Page cap for auto_paginate: 1-50, default 10. Requires
    /// auto_paginate: true.
    pub max_pages: Option<u32>,
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
    /// Date shortcut for the due date, e.g. `tomorrow`, `next friday`,
    /// `in 2 weeks`, `end of week`, `2026-07-01`. Mutually exclusive with
    /// due_date; `clear`/`none` leave the date unset.
    pub due_date_shortcut: Option<String>,
    /// Date shortcut for the start date. Mutually exclusive with start_date.
    pub start_date_shortcut: Option<String>,
    /// Date shortcut for the end date. Mutually exclusive with end_date.
    pub end_date_shortcut: Option<String>,
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
    /// Date shortcut for the due date, e.g. `tomorrow`, `next friday`,
    /// `in 2 weeks`, `end of week`, `2026-07-01`. Mutually exclusive with
    /// due_date; `clear`/`none` clear the date.
    pub due_date_shortcut: Option<String>,
    /// Date shortcut for the start date. Mutually exclusive with start_date.
    pub start_date_shortcut: Option<String>,
    /// Date shortcut for the end date. Mutually exclusive with end_date.
    pub end_date_shortcut: Option<String>,
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
pub struct BulkTaskIdsArgs {
    /// Numeric ids of the tasks to operate on. Must be non-empty, all
    /// positive, and at most 100 ids per call.
    pub task_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TasksBulkUpdateArgs {
    /// Numeric ids of the tasks to update. Must be non-empty, all positive,
    /// and at most 100 ids per call.
    pub task_ids: Vec<i64>,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// Set done state. Prefer vikunja_tasks_bulk_complete /
    /// vikunja_tasks_bulk_reopen.
    pub done: Option<bool>,
    /// New due date as RFC 3339; `0001-01-01T00:00:00Z` clears it.
    pub due_date: Option<String>,
    /// New start date as RFC 3339.
    pub start_date: Option<String>,
    /// New end date as RFC 3339.
    pub end_date: Option<String>,
    /// Date shortcut for the due date, e.g. `tomorrow`, `next friday`,
    /// `in 2 weeks`, `end of week`, `2026-07-01`. Mutually exclusive with
    /// due_date; `clear`/`none` clear the date.
    pub due_date_shortcut: Option<String>,
    /// Date shortcut for the start date. Mutually exclusive with start_date.
    pub start_date_shortcut: Option<String>,
    /// Date shortcut for the end date. Mutually exclusive with end_date.
    pub end_date_shortcut: Option<String>,
    /// Priority: 0 unset, 1 low, 2 medium, 3 high, 4 urgent, 5 DO NOW.
    pub priority: Option<i64>,
    /// Completion fraction between 0 and 1.
    pub percent_done: Option<f64>,
    /// Move the tasks to another project.
    pub project_id: Option<i64>,
    /// New hex color without the leading `#`.
    pub hex_color: Option<String>,
    /// Favorite (true) or unfavorite (false).
    pub is_favorite: Option<bool>,
    /// Repeat interval in seconds.
    pub repeat_after: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TasksBulkMoveArgs {
    /// Numeric ids of the tasks to move. Must be non-empty, all positive,
    /// and at most 100 ids per call.
    pub task_ids: Vec<i64>,
    /// Id of the project to move all listed tasks to.
    pub project_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BulkTaskLabelArgs {
    /// Numeric ids of the tasks. Must be non-empty, all positive, and at
    /// most 100 ids per call.
    pub task_ids: Vec<i64>,
    /// Numeric id of the label (find it with vikunja_labels_list).
    pub label_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BulkTaskUserArgs {
    /// Numeric ids of the tasks. Must be non-empty, all positive, and at
    /// most 100 ids per call.
    pub task_ids: Vec<i64>,
    /// Numeric id of the user (find it with vikunja_users_search).
    pub user_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct LabelsListArgs {
    /// Search labels by title.
    pub search: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
    /// If true, fetch up to max_pages pages starting at page 1 and return
    /// them as one result with auto_pagination metadata. Cannot be combined
    /// with page.
    pub auto_paginate: Option<bool>,
    /// Page cap for auto_paginate: 1-50, default 10. Requires
    /// auto_paginate: true.
    pub max_pages: Option<u32>,
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
pub struct TaskRelationArgs {
    /// Numeric id of the task the relation starts from.
    pub task_id: i64,
    /// Numeric id of the task the relation points to. Must differ from
    /// `task_id`.
    pub other_task_id: i64,
    /// Kind of the relation as seen from `task_id`, e.g. `blocking` means
    /// `task_id` blocks `other_task_id`.
    pub relation_kind: RelationKind,
}

/// Task date a relative reminder is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReminderRelativeTo {
    DueDate,
    StartDate,
    EndDate,
}

impl ReminderRelativeTo {
    /// The snake_case string Vikunja uses for this anchor on the wire.
    fn as_str(self) -> &'static str {
        match self {
            Self::DueDate => "due_date",
            Self::StartDate => "start_date",
            Self::EndDate => "end_date",
        }
    }
}

/// One reminder to write: an absolute time (`reminder` or
/// `reminder_shortcut`) or a relative one (`relative_to` +
/// `relative_period_seconds`), never both.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReminderInput {
    /// Absolute reminder time as RFC 3339, e.g. `2026-07-01T09:00:00Z`.
    pub reminder: Option<String>,
    /// Date shortcut for the reminder time, e.g. `tomorrow`, `next friday`,
    /// `in 3 days`, `2026-07-01`. Mutually exclusive with `reminder`.
    pub reminder_shortcut: Option<String>,
    /// Anchor a relative reminder to this task date.
    pub relative_to: Option<ReminderRelativeTo>,
    /// Offset in seconds from `relative_to`; negative means before it
    /// (e.g. -3600 = one hour before). Required with `relative_to`.
    pub relative_period_seconds: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskRemindersAddArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// The reminder to add, inlined into the arguments.
    #[serde(flatten)]
    pub reminder: ReminderInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskRemindersSetArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// Replacement reminder list. An empty list removes all reminders.
    pub reminders: Vec<ReminderInput>,
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
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct AttachmentsListArgs {
    /// Numeric id of the task.
    pub task_id: i64,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
    /// If true, fetch up to max_pages pages starting at page 1 and return
    /// them as one result with auto_pagination metadata. Cannot be combined
    /// with page.
    pub auto_paginate: Option<bool>,
    /// Page cap for auto_paginate: 1-50, default 10. Requires
    /// auto_paginate: true.
    pub max_pages: Option<u32>,
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
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
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
    /// If true, fetch up to max_pages pages starting at page 1 and return
    /// them as one result with auto_pagination metadata. Cannot be combined
    /// with page.
    pub auto_paginate: Option<bool>,
    /// Page cap for auto_paginate: 1-50, default 10. Requires
    /// auto_paginate: true.
    pub max_pages: Option<u32>,
}

/// Metadata describing a bounded auto-paginated fetch. Present in list
/// results only when the call used `auto_paginate`.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct AutoPagination {
    /// Number of pages actually fetched (at least 1).
    pub pages_read: u32,
    /// The page cap the fetch was bounded by.
    pub page_cap: u32,
    /// True when the page cap was hit while the server still reported more
    /// pages; the result is incomplete. Only set when the server explicitly
    /// reported more pages via its pagination headers.
    pub truncated: bool,
    /// Total number of items returned across all fetched pages.
    pub count: usize,
}

impl AutoPagination {
    fn from_bounded<T>(bounded: &BoundedPage<T>) -> Self {
        Self {
            pages_read: bounded.pages_read,
            page_cap: bounded.page_cap,
            truncated: bounded.truncated,
            count: bounded.items.len(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FiltersListArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FilterIdArgs {
    /// Numeric id of the saved filter (not the negative pseudo-project id;
    /// find it with vikunja_filters_list).
    pub filter_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FiltersCreateArgs {
    /// Filter title.
    pub title: String,
    /// Filter description.
    pub description: Option<String>,
    /// Vikunja filter expression to store, e.g. `done = false && priority >= 3`.
    /// See https://vikunja.io/docs/filters.
    pub filter: String,
    /// Fields to sort matching tasks by, e.g. `["due_date", "id"]`.
    pub sort_by: Option<Vec<String>>,
    /// Sort directions matching `sort_by` position by position, e.g. `["asc"]`.
    pub order_by: Option<Vec<String>>,
    /// IANA timezone used to resolve relative dates like `now/d`.
    pub filter_timezone: Option<String>,
    /// Whether tasks with a null value in a filtered field match.
    pub filter_include_nulls: Option<bool>,
    /// Mark the saved filter as a favorite.
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FiltersUpdateArgs {
    /// Numeric id of the saved filter to update.
    pub filter_id: i64,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New filter expression (replaces the stored one; other stored query
    /// fields keep their value).
    pub filter: Option<String>,
    /// New sort fields (replaces the stored list).
    pub sort_by: Option<Vec<String>>,
    /// New sort directions (replaces the stored list).
    pub order_by: Option<Vec<String>>,
    /// New timezone for relative dates.
    pub filter_timezone: Option<String>,
    /// Whether tasks with a null value in a filtered field match.
    pub filter_include_nulls: Option<bool>,
    /// Favorite (true) or unfavorite (false) the saved filter.
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct FilterTasksArgs {
    /// Numeric id of the saved filter to evaluate.
    pub filter_id: i64,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page; the Vikunja server caps this (50 by default).
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SavedFilterListResult {
    pub filters: Vec<SavedFilterSummary>,
}

/// One page of tasks matching a saved filter, plus the query that was
/// evaluated to produce it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FilterTasksResult {
    pub filter_id: i64,
    /// Title of the saved filter.
    pub title: String,
    /// The stored filter expression that was evaluated.
    pub filter: Option<String>,
    /// Sort field applied (first stored `sort_by` entry).
    pub sort_by: Option<String>,
    /// Sort direction applied (first stored `order_by` entry).
    pub order_by: Option<String>,
    pub tasks: Vec<Task>,
    pub pagination: PageInfo,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DatesResolveArgs {
    /// Date shortcut to preview, e.g. `tomorrow`, `next friday`,
    /// `in 2 weeks`, `end of week`, `2026-07-01`, `clear`.
    pub expression: String,
    /// RFC 3339 instant to resolve against instead of the current server
    /// time. Its UTC offset defines the timezone used for the resolution.
    pub reference_time: Option<String>,
    /// Which date field the expression is meant for: `due_date`,
    /// `start_date` or `end_date`. Informational only; resolution is the
    /// same for all targets.
    pub target: Option<String>,
}

/// Preview of how a date shortcut resolves, without calling Vikunja.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DatesResolveResult {
    /// The expression that was resolved.
    pub expression: String,
    /// Reference instant the expression was resolved against (RFC 3339).
    pub reference_time: String,
    /// Resolved RFC 3339 timestamp; null when the expression clears the
    /// date.
    pub resolved: Option<String>,
    /// True when the expression means clear/omit the date (`clear`, `none`,
    /// `unset`, `no due date`).
    pub clears_date: bool,
    /// Timezone the resolution used.
    pub timezone_description: String,
    /// Configured time of day (HH:MM) applied to the resolved date, when a
    /// date was resolved.
    pub default_time_used: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct ProjectViewsListArgs {
    /// Numeric id of the project.
    pub project_id: i64,
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct BucketsListArgs {
    /// Numeric id of the project.
    pub project_id: i64,
    /// Numeric id of the kanban view to read. When omitted, the project's
    /// first kanban view is used (find views with vikunja_project_views_list).
    pub view_id: Option<i64>,
    /// 1-based page number (default 1). Paginates buckets; each bucket also
    /// carries at most one page of its tasks.
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

/// Views of one project. Unlike the other list results there is no
/// auto-pagination variant: a project only ever has a handful of views.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectViewListResult {
    pub views: Vec<ProjectView>,
    pub pagination: PageInfo,
}

/// One kanban lane with its tasks and name, plus the flags that need the
/// view definition to resolve.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BucketInfo {
    pub id: i64,
    pub title: String,
    /// Maximum number of tasks allowed in the bucket; 0 means no limit.
    pub limit: i64,
    /// Number of tasks in the bucket as counted by the server.
    pub count: i64,
    pub position: f64,
    /// True when new tasks land in this bucket by default. Only known when
    /// the view definition was fetched (i.e. view_id was auto-resolved);
    /// omitted when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default_bucket: Option<bool>,
    /// True when tasks in this bucket are treated as done. Only known when
    /// the view definition was fetched; omitted when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_done_bucket: Option<bool>,
    /// Tasks currently in this bucket (one page per bucket); omitted when
    /// the bucket is empty and Vikunja sends no list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<Task>>,
}

/// The buckets of one project's kanban view.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BucketsListResult {
    pub project_id: i64,
    /// Id of the kanban view the buckets belong to.
    pub view_id: i64,
    /// Title of the view, when its definition was fetched (omitted with an
    /// explicit view_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_title: Option<String>,
    pub buckets: Vec<BucketInfo>,
    pub pagination: PageInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectListResult {
    pub projects: Vec<Project>,
    pub pagination: PageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pagination: Option<AutoPagination>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TaskListResult {
    pub tasks: Vec<Task>,
    pub pagination: PageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pagination: Option<AutoPagination>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelListResult {
    pub labels: Vec<Label>,
    pub pagination: PageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pagination: Option<AutoPagination>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TeamListResult {
    pub teams: Vec<Team>,
    pub pagination: PageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pagination: Option<AutoPagination>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UserListResult {
    pub users: Vec<User>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommentListResult {
    pub comments: Vec<TaskComment>,
}

/// Tasks related to one task under a single relation kind.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TaskRelationGroup {
    /// Relation kind as seen from the queried task, e.g. `blocking`.
    pub relation_kind: String,
    /// The related tasks.
    pub tasks: Vec<Task>,
}

/// All relations of one task. `relation_kind` is a plain string here (not
/// the [`RelationKind`] enum) so kinds added by a newer Vikunja server pass
/// through instead of failing deserialization.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RelationListResult {
    /// The task whose relations were listed.
    pub task_id: i64,
    /// Related tasks grouped by relation kind, sorted by kind.
    pub relations: Vec<TaskRelationGroup>,
}

/// The reminders of one task, as stored by Vikunja after a read or write.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReminderListResult {
    /// The task whose reminders these are.
    pub task_id: i64,
    /// The task's reminders. Relative reminders carry `relative_period` and
    /// `relative_to`; Vikunja fills in the absolute `reminder` time once the
    /// anchor date exists.
    pub reminders: Vec<TaskReminder>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachmentListResult {
    pub attachments: Vec<TaskAttachment>,
    pub pagination: PageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pagination: Option<AutoPagination>,
}

/// Result of a mutation that only returns a confirmation message.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OperationResult {
    pub ok: bool,
    pub message: String,
}

/// Result of a bulk task operation: aggregate counts plus one entry per
/// requested task id, in input order.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct BulkOperationResult {
    /// True only when every task succeeded (`failed == 0`).
    pub ok: bool,
    /// Number of task ids processed.
    pub total: usize,
    /// Number of tasks that succeeded.
    pub succeeded: usize,
    /// Number of tasks that failed.
    pub failed: usize,
    /// Per-task outcomes, in the same order as the requested `task_ids`.
    pub results: Vec<BulkTaskResult>,
}

/// Outcome of one task within a bulk operation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BulkTaskResult {
    pub task_id: i64,
    pub ok: bool,
    /// Operation applied, e.g. `complete`, `update`, `label_add`.
    pub operation: String,
    /// The resulting task, for operations that return the task.
    pub task: Option<Task>,
    /// Confirmation message, for operations that do not return a task.
    pub message: Option<String>,
    /// Failure detail when `ok` is false.
    pub error: Option<BulkItemError>,
}

/// Structured error detail for one failed item of a bulk operation. Carries
/// the same safe fields as MCP error data — never tokens or headers.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct BulkItemError {
    /// Error category: `auth`, `forbidden`, `not_found`, `validation`,
    /// `conflict`, `rate_limited`, `server`, `network`, `timeout`,
    /// `invalid_response`, `io`, `too_large`, `invalid_argument` or `other`.
    pub kind: String,
    /// HTTP status reported by Vikunja, when the API answered.
    pub http_status: Option<u16>,
    /// Vikunja-specific error code from the response body, if present.
    pub vikunja_error_code: Option<i64>,
    /// Human-readable description of the failure.
    pub message: String,
}

impl BulkItemError {
    fn from_error(err: &Error) -> Self {
        let (kind, http_status, vikunja_error_code) = match err {
            Error::Api {
                status, kind, code, ..
            } => {
                let kind = match kind {
                    ApiErrorKind::Auth => "auth",
                    ApiErrorKind::Forbidden => "forbidden",
                    ApiErrorKind::NotFound => "not_found",
                    ApiErrorKind::Validation => "validation",
                    ApiErrorKind::Conflict => "conflict",
                    ApiErrorKind::RateLimited => "rate_limited",
                    ApiErrorKind::Server => "server",
                    ApiErrorKind::Other => "other",
                };
                (kind, Some(*status), *code)
            }
            Error::Network { .. } => ("network", None, None),
            Error::Timeout { .. } => ("timeout", None, None),
            Error::InvalidResponse { .. } => ("invalid_response", None, None),
            Error::Io { .. } => ("io", None, None),
            Error::TooLarge { .. } => ("too_large", None, None),
            Error::InvalidArgument(_) => ("invalid_argument", None, None),
        };
        Self {
            kind: kind.to_string(),
            http_status,
            vikunja_error_code,
            message: err.to_string(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadResult {
    pub ok: bool,
    pub message: String,
    /// The uploaded attachment, when it could be identified after upload.
    pub attachment: Option<TaskAttachment>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
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

/// Validates the auto-pagination arguments of a list call before any
/// request is made. Returns the effective page cap when auto-pagination was
/// requested, or `None` for a plain one-page call. `auto_paginate: false`
/// and omitting the argument are treated identically: both reject
/// `max_pages`.
fn auto_page_cap(
    page: Option<u32>,
    auto_paginate: Option<bool>,
    max_pages: Option<u32>,
) -> Result<Option<u32>, McpError> {
    if auto_paginate != Some(true) {
        if max_pages.is_some() {
            return Err(Error::InvalidArgument(
                "max_pages requires auto_paginate: true".to_string(),
            )
            .to_mcp());
        }
        return Ok(None);
    }
    if page.is_some() {
        return Err(Error::InvalidArgument(
            "page cannot be combined with auto_paginate; auto-pagination always starts at page 1"
                .to_string(),
        )
        .to_mcp());
    }
    let cap = max_pages.unwrap_or(DEFAULT_AUTO_MAX_PAGES);
    if !(1..=MAX_AUTO_MAX_PAGES).contains(&cap) {
        return Err(Error::InvalidArgument(format!(
            "max_pages must be between 1 and {MAX_AUTO_MAX_PAGES}, got {cap}"
        ))
        .to_mcp());
    }
    Ok(Some(cap))
}

/// Validates the two task ids of a relation: both positive and distinct.
fn relation_pair(task_id: i64, other_task_id: i64) -> Result<(i64, i64), McpError> {
    let task_id = positive("task_id", task_id)?;
    let other_task_id = positive("other_task_id", other_task_id)?;
    if task_id == other_task_id {
        return Err(Error::InvalidArgument(
            "task_id and other_task_id must be different tasks".to_string(),
        )
        .to_mcp());
    }
    Ok((task_id, other_task_id))
}

/// Validates one [`ReminderInput`] and turns it into the wire shape.
/// Exactly one of "absolute" (`reminder` / `reminder_shortcut`) or
/// "relative" (`relative_to` + `relative_period_seconds`) must be given.
/// Shortcuts resolve against `reference` so every reminder in one call uses
/// the same instant.
fn build_reminder(
    input: ReminderInput,
    reference: &DateTime<Local>,
    dates: &DateConfig,
) -> Result<TaskReminder, McpError> {
    let has_absolute = input.reminder.is_some() || input.reminder_shortcut.is_some();
    let has_relative = input.relative_to.is_some() || input.relative_period_seconds.is_some();
    if has_absolute && has_relative {
        return Err(Error::InvalidArgument(
            "provide either an absolute reminder (reminder or reminder_shortcut) or a relative \
             one (relative_to with relative_period_seconds), not both"
                .to_string(),
        )
        .to_mcp());
    }
    match (
        input.reminder,
        input.reminder_shortcut,
        input.relative_to,
        input.relative_period_seconds,
    ) {
        // Still reachable after the guard above: both absolute forms set
        // with no relative fields.
        (Some(_), Some(_), _, _) => Err(Error::InvalidArgument(
            "provide either reminder or reminder_shortcut, not both".to_string(),
        )
        .to_mcp()),
        (Some(timestamp), None, _, _) => {
            if DateTime::parse_from_rfc3339(&timestamp).is_err() {
                return Err(Error::InvalidArgument(format!(
                    "reminder must be a valid RFC 3339 timestamp like 2026-07-01T09:00:00Z, \
                     got {timestamp:?}"
                ))
                .to_mcp());
            }
            Ok(TaskReminder {
                reminder: Some(timestamp),
                relative_period: None,
                relative_to: None,
            })
        }
        (None, Some(expression), _, _) => {
            match dates::resolve(&expression, reference, dates).map_err(|e| e.to_mcp())? {
                Resolution::Clear => Err(Error::InvalidArgument(
                    "a clear shortcut cannot be used as a reminder time; to remove reminders \
                     call vikunja_task_reminders_set with an empty list"
                        .to_string(),
                )
                .to_mcp()),
                Resolution::Timestamp { datetime, .. } => Ok(TaskReminder {
                    reminder: Some(datetime.to_rfc3339_opts(SecondsFormat::Secs, true)),
                    relative_period: None,
                    relative_to: None,
                }),
            }
        }
        (None, None, Some(anchor), Some(period)) => Ok(TaskReminder {
            reminder: None,
            relative_period: Some(period),
            relative_to: Some(anchor.as_str().to_string()),
        }),
        (None, None, Some(_), None) => Err(Error::InvalidArgument(
            "relative_period_seconds is required with relative_to".to_string(),
        )
        .to_mcp()),
        (None, None, None, Some(_)) => Err(Error::InvalidArgument(
            "relative_to is required with relative_period_seconds".to_string(),
        )
        .to_mcp()),
        (None, None, None, None) => Err(Error::InvalidArgument(
            "provide a reminder: reminder, reminder_shortcut, or relative_to with \
             relative_period_seconds"
                .to_string(),
        )
        .to_mcp()),
    }
}

fn non_empty(name: &str, value: &str) -> Result<(), McpError> {
    if value.trim().is_empty() {
        return Err(Error::InvalidArgument(format!("{name} must not be empty")).to_mcp());
    }
    Ok(())
}

/// Validates the sort arguments of the saved-filter tools: Vikunja pairs
/// `sort_by` and `order_by` positionally, so a length mismatch or a
/// direction other than `asc`/`desc` silently misbehaves server-side and is
/// rejected here instead.
fn validate_sort_order(
    sort_by: Option<&[String]>,
    order_by: Option<&[String]>,
) -> Result<(), McpError> {
    if let (Some(sort), Some(order)) = (sort_by, order_by)
        && sort.len() != order.len()
    {
        return Err(Error::InvalidArgument(format!(
            "sort_by ({}) and order_by ({}) must have the same number of entries",
            sort.len(),
            order.len()
        ))
        .to_mcp());
    }
    for direction in order_by.unwrap_or_default() {
        if direction != "asc" && direction != "desc" {
            return Err(Error::InvalidArgument(format!(
                "order_by entries must be 'asc' or 'desc', got {direction:?}"
            ))
            .to_mcp());
        }
    }
    Ok(())
}

/// Light syntactic validation of a Vikunja filter expression. The server
/// does not implement Vikunja's full filter grammar, but an empty
/// expression, unbalanced parentheses or an unterminated quoted string are
/// always invalid and are rejected before any request is sent.
fn validate_filter_expression(name: &str, value: &str) -> Result<(), McpError> {
    non_empty(name, value)?;
    let mut depth: u32 = 0;
    let mut quote: Option<char> = None;
    for ch in value.chars() {
        match quote {
            Some(open) => {
                if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        Error::InvalidArgument(format!(
                            "{name} has unbalanced parentheses: ')' without a matching '('"
                        ))
                        .to_mcp()
                    })?;
                }
                _ => {}
            },
        }
    }
    if let Some(open) = quote {
        return Err(Error::InvalidArgument(format!(
            "{name} has an unterminated quoted string (missing closing {open})"
        ))
        .to_mcp());
    }
    if depth != 0 {
        return Err(Error::InvalidArgument(format!(
            "{name} has unbalanced parentheses: '(' without a matching ')'"
        ))
        .to_mcp());
    }
    Ok(())
}

/// Validates the id list of a bulk operation before any write is issued.
fn validate_task_ids(task_ids: &[i64]) -> Result<(), McpError> {
    if task_ids.is_empty() {
        return Err(Error::InvalidArgument("task_ids must not be empty".to_string()).to_mcp());
    }
    if task_ids.len() > MAX_BULK_TASK_IDS {
        return Err(Error::InvalidArgument(format!(
            "task_ids has {} entries; at most {MAX_BULK_TASK_IDS} tasks are allowed per bulk call — split the batch",
            task_ids.len()
        ))
        .to_mcp());
    }
    if task_ids.iter().any(|&id| id <= 0) {
        return Err(Error::InvalidArgument(
            "task_ids must contain only positive integers".to_string(),
        )
        .to_mcp());
    }
    Ok(())
}

/// What one successful bulk item produced.
enum BulkOutcome {
    Task(Box<Task>),
    Message(String),
}

/// Runs `op` for each task id in turn, collecting per-task outcomes. A
/// failing item is recorded and does not abort the remaining items; writes
/// are never retried (the underlying client only retries idempotent GETs).
async fn run_bulk<F, Fut>(task_ids: Vec<i64>, operation: &str, op: F) -> Json<BulkOperationResult>
where
    F: Fn(i64) -> Fut,
    Fut: std::future::Future<Output = Result<BulkOutcome, Error>>,
{
    let mut results = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        results.push(match op(task_id).await {
            Ok(BulkOutcome::Task(task)) => BulkTaskResult {
                task_id,
                ok: true,
                operation: operation.to_string(),
                task: Some(*task),
                message: None,
                error: None,
            },
            Ok(BulkOutcome::Message(message)) => BulkTaskResult {
                task_id,
                ok: true,
                operation: operation.to_string(),
                task: None,
                message: Some(message),
                error: None,
            },
            Err(err) => BulkTaskResult {
                task_id,
                ok: false,
                operation: operation.to_string(),
                task: None,
                message: None,
                error: Some(BulkItemError::from_error(&err)),
            },
        });
    }
    let total = results.len();
    let succeeded = results.iter().filter(|r| r.ok).count();
    Json(BulkOperationResult {
        ok: succeeded == total,
        total,
        succeeded,
        failed: total - succeeded,
        results,
    })
}

/// Resolves an optional `*_shortcut` argument against its RFC 3339
/// counterpart. Providing both is rejected. `clear_replacement` is what a
/// clear shortcut produces: the zero date for updates, `None` (omit the
/// field) for create. Callers capture `reference` once per request so every
/// shortcut field in one call resolves against the same instant, even when
/// the call straddles a midnight or DST boundary.
fn resolved_date(
    field: &'static str,
    explicit: Option<String>,
    shortcut: Option<String>,
    clear_replacement: Option<&str>,
    reference: &DateTime<Local>,
    dates: &DateConfig,
) -> Result<Option<String>, McpError> {
    match (explicit, shortcut) {
        (Some(_), Some(_)) => Err(Error::InvalidArgument(format!(
            "provide either {field} or {field}_shortcut, not both"
        ))
        .to_mcp()),
        (explicit, None) => Ok(explicit),
        (None, Some(expression)) => {
            match dates::resolve(&expression, reference, dates).map_err(|e| e.to_mcp())? {
                Resolution::Clear => Ok(clear_replacement.map(str::to_string)),
                Resolution::Timestamp { datetime, .. } => {
                    Ok(Some(datetime.to_rfc3339_opts(SecondsFormat::Secs, true)))
                }
            }
        }
    }
}

/// Builds the structured output of `vikunja_dates_resolve` for a reference
/// instant in any timezone.
fn resolution_result<Tz: TimeZone>(
    expression: String,
    reference: &DateTime<Tz>,
    timezone_description: String,
    dates: &DateConfig,
) -> Result<Json<DatesResolveResult>, McpError>
where
    Tz::Offset: std::fmt::Display,
{
    let (resolved, default_time_used, clears_date) =
        match dates::resolve(&expression, reference, dates).map_err(|e| e.to_mcp())? {
            Resolution::Clear => (None, None, true),
            Resolution::Timestamp {
                datetime,
                time_of_day,
            } => (
                Some(datetime.to_rfc3339_opts(SecondsFormat::Secs, true)),
                Some(time_of_day.format("%H:%M").to_string()),
                false,
            ),
        };
    Ok(Json(DatesResolveResult {
        expression,
        reference_time: reference.to_rfc3339_opts(SecondsFormat::Secs, true),
        resolved,
        clears_date,
        timezone_description,
        default_time_used,
    }))
}

/// Loads the buckets of a project's kanban view. With an explicit
/// `view_id` the buckets are fetched directly; otherwise the project's
/// views are listed and the first kanban view is used (its definition then
/// also resolves the default/done bucket flags). Shared by the
/// vikunja_buckets_list tool and the projects/{id}/buckets resource.
pub(crate) async fn load_project_buckets(
    client: &VikunjaClient,
    project_id: i64,
    view_id: Option<i64>,
    params: PageParams,
) -> Result<BucketsListResult, Error> {
    let view: Option<ProjectView> = match view_id {
        Some(_) => None,
        None => {
            // Walk all view pages (bounded): a project's kanban view must
            // not be missed just because it sits beyond the first page.
            let views = walk_pages(MAX_VIEW_RESOLUTION_PAGES, |page| {
                client.list_project_views(project_id, PageParams::new(Some(page), None))
            })
            .await?;
            let kanban = views.items.into_iter().find(ProjectView::is_kanban);
            Some(kanban.ok_or_else(|| {
                Error::InvalidArgument(format!(
                    "project {project_id} has no kanban view; check its views with \
                     vikunja_project_views_list or pass view_id explicitly"
                ))
            })?)
        }
    };
    let view_id = match (view_id, view.as_ref()) {
        (Some(id), _) => id,
        (None, Some(view)) => view.id,
        (None, None) => unreachable!("view resolution always yields an id or errors"),
    };

    let page = client
        .list_view_buckets(project_id, view_id, params)
        .await?;
    let buckets = page
        .items
        .into_iter()
        .map(|bucket: Bucket| BucketInfo {
            is_default_bucket: view
                .as_ref()
                .map(|view| view.default_bucket_id == bucket.id),
            is_done_bucket: view.as_ref().map(|view| view.done_bucket_id == bucket.id),
            id: bucket.id,
            title: bucket.title,
            limit: bucket.limit,
            count: bucket.count,
            position: bucket.position,
            tasks: bucket.tasks,
        })
        .collect();
    Ok(BucketsListResult {
        project_id,
        view_id,
        view_title: view.map(|view| view.title),
        buckets,
        pagination: page.info,
    })
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
        description = "List or search Vikunja projects the user has access to. Returns one page of projects plus pagination info, or — with auto_paginate — up to max_pages pages (default 10, max 50) as one bounded result.",
        annotations(read_only_hint = true)
    )]
    pub async fn projects_list(
        &self,
        Parameters(args): Parameters<ProjectsListArgs>,
    ) -> Result<Json<ProjectListResult>, McpError> {
        let cap = auto_page_cap(args.page, args.auto_paginate, args.max_pages)?;
        let params = page_params(args.page, args.per_page)?;
        if let Some(cap) = cap {
            let bounded = walk_pages(cap, |page| {
                self.client().list_projects(
                    PageParams::new(Some(page), args.per_page),
                    args.search.as_deref(),
                    args.is_archived,
                )
            })
            .await?;
            return Ok(Json(ProjectListResult {
                pagination: bounded.last_info.clone(),
                auto_pagination: Some(AutoPagination::from_bounded(&bounded)),
                projects: bounded.items,
            }));
        }
        let page = self
            .client()
            .list_projects(params, args.search.as_deref(), args.is_archived)
            .await?;
        Ok(Json(ProjectListResult {
            projects: page.items,
            pagination: page.info,
            auto_pagination: None,
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
        description = "List or search Vikunja tasks across all projects, or within one project via project_id. Supports Vikunja filter expressions (e.g. 'done = false && priority >= 3', 'due_date < now/d+7d') and sorting. With auto_paginate, fetches up to max_pages pages (default 10, max 50) as one bounded result.",
        annotations(read_only_hint = true)
    )]
    pub async fn tasks_list(
        &self,
        Parameters(args): Parameters<TasksListArgs>,
    ) -> Result<Json<TaskListResult>, McpError> {
        let cap = auto_page_cap(args.page, args.auto_paginate, args.max_pages)?;
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
            ..Default::default()
        };
        if let Some(cap) = cap {
            let bounded = self
                .client()
                .list_all_tasks_with_options(&options, cap)
                .await?;
            return Ok(Json(TaskListResult {
                pagination: bounded.last_info.clone(),
                auto_pagination: Some(AutoPagination::from_bounded(&bounded)),
                tasks: bounded.items,
            }));
        }
        let page = self.client().list_tasks(&options).await?;
        Ok(Json(TaskListResult {
            tasks: page.items,
            pagination: page.info,
            auto_pagination: None,
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
        // On create, a clear shortcut omits the field entirely. One shared
        // reference instant keeps all three dates consistent.
        let reference = Local::now();
        let due_date = resolved_date(
            "due_date",
            args.due_date,
            args.due_date_shortcut,
            None,
            &reference,
            self.dates(),
        )?;
        let start_date = resolved_date(
            "start_date",
            args.start_date,
            args.start_date_shortcut,
            None,
            &reference,
            self.dates(),
        )?;
        let end_date = resolved_date(
            "end_date",
            args.end_date,
            args.end_date_shortcut,
            None,
            &reference,
            self.dates(),
        )?;
        let body = TaskCreate {
            title: args.title,
            description: args.description,
            due_date,
            start_date,
            end_date,
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
        // On update, a clear shortcut sends Vikunja's zero date. One shared
        // reference instant keeps all three dates consistent.
        let reference = Local::now();
        let due_date = resolved_date(
            "due_date",
            args.due_date,
            args.due_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let start_date = resolved_date(
            "start_date",
            args.start_date,
            args.start_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let end_date = resolved_date(
            "end_date",
            args.end_date,
            args.end_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let patch = TaskUpdate {
            title: args.title,
            description: args.description,
            done: args.done,
            due_date,
            start_date,
            end_date,
            priority: args.priority,
            percent_done: args.percent_done,
            project_id: args.project_id,
            hex_color: args.hex_color,
            is_favorite: args.is_favorite,
            repeat_after: args.repeat_after,
            reminders: None,
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

    // -- Bulk task operations --
    //
    // All bulk tools take an explicit, validated list of task ids (no
    // filter-based selection), fan out over the same safe per-task client
    // calls as the single-task tools, and report per-task results: after
    // argument validation passes, one failing task never fails the call.

    #[tool(
        name = "vikunja_tasks_bulk_complete",
        description = "Mark several Vikunja tasks as done in one call. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_bulk_complete(
        &self,
        Parameters(args): Parameters<BulkTaskIdsArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        Ok(run_bulk(args.task_ids, "complete", |task_id| async move {
            self.client()
                .set_task_done(task_id, true)
                .await
                .map(|task| BulkOutcome::Task(Box::new(task)))
        })
        .await)
    }

    #[tool(
        name = "vikunja_tasks_bulk_reopen",
        description = "Mark several done Vikunja tasks as not done in one call. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_bulk_reopen(
        &self,
        Parameters(args): Parameters<BulkTaskIdsArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        Ok(run_bulk(args.task_ids, "reopen", |task_id| async move {
            self.client()
                .set_task_done(task_id, false)
                .await
                .map(|task| BulkOutcome::Task(Box::new(task)))
        })
        .await)
    }

    #[tool(
        name = "vikunja_tasks_bulk_update",
        description = "Apply one partial update to several Vikunja tasks. Takes explicit task ids only; provided fields change on every listed task, others keep their value. Returns per-task results; a failing task does not abort the rest.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_bulk_update(
        &self,
        Parameters(args): Parameters<TasksBulkUpdateArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        if let Some(project_id) = args.project_id {
            positive("project_id", project_id)?;
        }
        // Shortcuts resolve once against one shared reference instant and
        // the same RFC 3339 value is applied to every listed task; a clear
        // shortcut sends Vikunja's zero date.
        let reference = Local::now();
        let due_date = resolved_date(
            "due_date",
            args.due_date,
            args.due_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let start_date = resolved_date(
            "start_date",
            args.start_date,
            args.start_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let end_date = resolved_date(
            "end_date",
            args.end_date,
            args.end_date_shortcut,
            Some(dates::CLEAR_DATE_RFC3339),
            &reference,
            self.dates(),
        )?;
        let patch = TaskUpdate {
            title: args.title,
            description: args.description,
            done: args.done,
            due_date,
            start_date,
            end_date,
            priority: args.priority,
            percent_done: args.percent_done,
            project_id: args.project_id,
            hex_color: args.hex_color,
            is_favorite: args.is_favorite,
            repeat_after: args.repeat_after,
            reminders: None,
        };
        // Reject an empty patch up front instead of failing every item.
        let patch_value = serde_json::to_value(&patch).map_err(|e| {
            Error::InvalidResponse {
                endpoint: "tasks.update",
                detail: format!("failed to serialize update payload: {e}"),
            }
            .to_mcp()
        })?;
        if patch_value
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
        {
            return Err(Error::InvalidArgument(
                "nothing to update: provide at least one field".to_string(),
            )
            .to_mcp());
        }
        Ok(run_bulk(args.task_ids, "update", |task_id| {
            let patch = &patch;
            async move {
                self.client()
                    .update_task(task_id, patch)
                    .await
                    .map(|task| BulkOutcome::Task(Box::new(task)))
            }
        })
        .await)
    }

    #[tool(
        name = "vikunja_tasks_bulk_move",
        description = "Move several Vikunja tasks to another project. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest.",
        annotations(idempotent_hint = true)
    )]
    pub async fn tasks_bulk_move(
        &self,
        Parameters(args): Parameters<TasksBulkMoveArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        let project_id = positive("project_id", args.project_id)?;
        let patch = TaskUpdate {
            project_id: Some(project_id),
            ..Default::default()
        };
        Ok(run_bulk(args.task_ids, "move", |task_id| {
            let patch = &patch;
            async move {
                self.client()
                    .update_task(task_id, patch)
                    .await
                    .map(|task| BulkOutcome::Task(Box::new(task)))
            }
        })
        .await)
    }

    #[tool(
        name = "vikunja_task_labels_bulk_add",
        description = "Add one existing label to several Vikunja tasks. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest."
    )]
    pub async fn task_labels_bulk_add(
        &self,
        Parameters(args): Parameters<BulkTaskLabelArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        let label_id = positive("label_id", args.label_id)?;
        Ok(run_bulk(args.task_ids, "label_add", |task_id| async move {
            self.client()
                .add_task_label(task_id, label_id)
                .await
                .map(|_| BulkOutcome::Message(format!("label {label_id} added to task {task_id}")))
        })
        .await)
    }

    #[tool(
        name = "vikunja_task_labels_bulk_remove",
        description = "Remove one label from several Vikunja tasks. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest.",
        annotations(destructive_hint = true)
    )]
    pub async fn task_labels_bulk_remove(
        &self,
        Parameters(args): Parameters<BulkTaskLabelArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        let label_id = positive("label_id", args.label_id)?;
        Ok(
            run_bulk(args.task_ids, "label_remove", |task_id| async move {
                self.client()
                    .remove_task_label(task_id, label_id)
                    .await
                    .map(|message| BulkOutcome::Message(message.message))
            })
            .await,
        )
    }

    #[tool(
        name = "vikunja_tasks_bulk_assign",
        description = "Assign one user to several Vikunja tasks. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest. Find user ids with vikunja_users_search."
    )]
    pub async fn tasks_bulk_assign(
        &self,
        Parameters(args): Parameters<BulkTaskUserArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        let user_id = positive("user_id", args.user_id)?;
        Ok(run_bulk(args.task_ids, "assign", |task_id| async move {
            self.client()
                .assign_user(task_id, user_id)
                .await
                .map(|_| BulkOutcome::Message(format!("user {user_id} assigned to task {task_id}")))
        })
        .await)
    }

    #[tool(
        name = "vikunja_tasks_bulk_unassign",
        description = "Remove one user's assignment from several Vikunja tasks. Takes explicit task ids only and returns per-task results; a failing task does not abort the rest.",
        annotations(destructive_hint = true)
    )]
    pub async fn tasks_bulk_unassign(
        &self,
        Parameters(args): Parameters<BulkTaskUserArgs>,
    ) -> Result<Json<BulkOperationResult>, McpError> {
        validate_task_ids(&args.task_ids)?;
        let user_id = positive("user_id", args.user_id)?;
        Ok(run_bulk(args.task_ids, "unassign", |task_id| async move {
            self.client()
                .unassign_user(task_id, user_id)
                .await
                .map(|message| BulkOutcome::Message(message.message))
        })
        .await)
    }

    // -- Dates --

    #[tool(
        name = "vikunja_dates_resolve",
        description = "Preview how a date shortcut resolves to an RFC 3339 timestamp without writing anything. Supported: today, tomorrow, yesterday, in N days/weeks/months, [next] monday..sunday, end of week, YYYY-MM-DD, and clear/none/unset/no due date. Use before vikunja_tasks_create/update.",
        annotations(read_only_hint = true)
    )]
    pub async fn dates_resolve(
        &self,
        Parameters(args): Parameters<DatesResolveArgs>,
    ) -> Result<Json<DatesResolveResult>, McpError> {
        if let Some(target) = args.target.as_deref()
            && !matches!(target, "due_date" | "start_date" | "end_date")
        {
            return Err(Error::InvalidArgument(
                "target must be one of due_date, start_date or end_date".to_string(),
            )
            .to_mcp());
        }
        match args.reference_time.as_deref() {
            Some(raw) => {
                let reference = DateTime::parse_from_rfc3339(raw).map_err(|e| {
                    Error::InvalidArgument(format!(
                        "reference_time is not a valid RFC 3339 timestamp: {e}"
                    ))
                    .to_mcp()
                })?;
                let timezone_description = format!(
                    "fixed UTC offset {} from reference_time",
                    reference.offset()
                );
                resolution_result(
                    args.expression,
                    &reference,
                    timezone_description,
                    self.dates(),
                )
            }
            None => {
                let reference = Local::now();
                let timezone_description =
                    format!("server local timezone (UTC offset {})", reference.offset());
                resolution_result(
                    args.expression,
                    &reference,
                    timezone_description,
                    self.dates(),
                )
            }
        }
    }

    // -- Labels --

    #[tool(
        name = "vikunja_labels_list",
        description = "List or search the user's Vikunja labels. With auto_paginate, fetches up to max_pages pages (default 10, max 50) as one bounded result.",
        annotations(read_only_hint = true)
    )]
    pub async fn labels_list(
        &self,
        Parameters(args): Parameters<LabelsListArgs>,
    ) -> Result<Json<LabelListResult>, McpError> {
        let cap = auto_page_cap(args.page, args.auto_paginate, args.max_pages)?;
        let params = page_params(args.page, args.per_page)?;
        if let Some(cap) = cap {
            let bounded = walk_pages(cap, |page| {
                self.client().list_labels(
                    PageParams::new(Some(page), args.per_page),
                    args.search.as_deref(),
                )
            })
            .await?;
            return Ok(Json(LabelListResult {
                pagination: bounded.last_info.clone(),
                auto_pagination: Some(AutoPagination::from_bounded(&bounded)),
                labels: bounded.items,
            }));
        }
        let page = self
            .client()
            .list_labels(params, args.search.as_deref())
            .await?;
        Ok(Json(LabelListResult {
            labels: page.items,
            pagination: page.info,
            auto_pagination: None,
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

    // -- Task relations --

    #[tool(
        name = "vikunja_task_relations_list",
        description = "List a Vikunja task's relations (subtasks, parent, blocking, ... ) grouped by relation kind.",
        annotations(read_only_hint = true)
    )]
    pub async fn task_relations_list(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<RelationListResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let task = self.client().get_task(task_id).await?;
        let relations = task
            .related_tasks
            .unwrap_or_default()
            .into_iter()
            .map(|(relation_kind, tasks)| TaskRelationGroup {
                relation_kind,
                tasks,
            })
            .collect();
        Ok(Json(RelationListResult { task_id, relations }))
    }

    #[tool(
        name = "vikunja_task_relations_create",
        description = "Create a relation between two Vikunja tasks, e.g. relation_kind 'blocking' means task_id blocks other_task_id. Kinds: subtask, parenttask, related, duplicateof, duplicates, blocking, blocked, precedes, follows, copiedfrom, copiedto."
    )]
    pub async fn task_relations_create(
        &self,
        Parameters(args): Parameters<TaskRelationArgs>,
    ) -> Result<Json<TaskRelation>, McpError> {
        let (task_id, other_task_id) = relation_pair(args.task_id, args.other_task_id)?;
        Ok(Json(
            self.client()
                .create_task_relation(task_id, other_task_id, args.relation_kind)
                .await?,
        ))
    }

    #[tool(
        name = "vikunja_task_relations_delete",
        description = "Delete a relation between two Vikunja tasks. The relation_kind must match the existing relation as seen from task_id.",
        annotations(destructive_hint = true)
    )]
    pub async fn task_relations_delete(
        &self,
        Parameters(args): Parameters<TaskRelationArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let (task_id, other_task_id) = relation_pair(args.task_id, args.other_task_id)?;
        let message = self
            .client()
            .delete_task_relation(task_id, other_task_id, args.relation_kind)
            .await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    // -- Task reminders --

    #[tool(
        name = "vikunja_task_reminders_list",
        description = "List a Vikunja task's reminders (absolute times and offsets relative to due/start/end dates).",
        annotations(read_only_hint = true)
    )]
    pub async fn task_reminders_list(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<Json<ReminderListResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let task = self.client().get_task(task_id).await?;
        Ok(Json(ReminderListResult {
            task_id,
            reminders: task.reminders.unwrap_or_default(),
        }))
    }

    #[tool(
        name = "vikunja_task_reminders_add",
        description = "Add one reminder to a Vikunja task, keeping existing reminders. Give an absolute time (reminder as RFC 3339, or reminder_shortcut like 'tomorrow'), or a relative one (relative_to due_date/start_date/end_date plus relative_period_seconds, negative = before). Not idempotent: calling it twice adds the same reminder twice."
    )]
    pub async fn task_reminders_add(
        &self,
        Parameters(args): Parameters<TaskRemindersAddArgs>,
    ) -> Result<Json<ReminderListResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let reference = Local::now();
        let new_reminder = build_reminder(args.reminder, &reference, self.dates())?;
        let updated = self
            .client()
            .append_task_reminder(task_id, &new_reminder)
            .await?;
        Ok(Json(ReminderListResult {
            task_id,
            reminders: updated.reminders.unwrap_or_default(),
        }))
    }

    #[tool(
        name = "vikunja_task_reminders_set",
        description = "Replace all reminders of a Vikunja task with the given list; an empty list removes every reminder. Each entry is an absolute time (reminder RFC 3339 or reminder_shortcut) or a relative one (relative_to + relative_period_seconds).",
        annotations(idempotent_hint = true)
    )]
    pub async fn task_reminders_set(
        &self,
        Parameters(args): Parameters<TaskRemindersSetArgs>,
    ) -> Result<Json<ReminderListResult>, McpError> {
        let task_id = positive("task_id", args.task_id)?;
        let reference = Local::now();
        let reminders = args
            .reminders
            .into_iter()
            .map(|input| build_reminder(input, &reference, self.dates()))
            .collect::<Result<Vec<_>, _>>()?;
        let patch = TaskUpdate {
            reminders: Some(reminders),
            ..Default::default()
        };
        let updated = self.client().update_task(task_id, &patch).await?;
        Ok(Json(ReminderListResult {
            task_id,
            reminders: updated.reminders.unwrap_or_default(),
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
        description = "List the attachments of a Vikunja task. With auto_paginate, fetches up to max_pages pages (default 10, max 50) as one bounded result.",
        annotations(read_only_hint = true)
    )]
    pub async fn task_attachments_list(
        &self,
        Parameters(args): Parameters<AttachmentsListArgs>,
    ) -> Result<Json<AttachmentListResult>, McpError> {
        let id = positive("task_id", args.task_id)?;
        let cap = auto_page_cap(args.page, args.auto_paginate, args.max_pages)?;
        let params = page_params(args.page, args.per_page)?;
        if let Some(cap) = cap {
            let bounded = walk_pages(cap, |page| {
                self.client()
                    .list_task_attachments(id, PageParams::new(Some(page), args.per_page))
            })
            .await?;
            return Ok(Json(AttachmentListResult {
                pagination: bounded.last_info.clone(),
                auto_pagination: Some(AutoPagination::from_bounded(&bounded)),
                attachments: bounded.items,
            }));
        }
        let page = self.client().list_task_attachments(id, params).await?;
        Ok(Json(AttachmentListResult {
            attachments: page.items,
            pagination: page.info,
            auto_pagination: None,
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
                // Resolve against the sandbox first (canonicalizing the
                // path), then do all IO through the resolved path.
                let resolved = self
                    .attachment_sandbox()
                    .resolve_upload_path(&path)
                    .map_err(|e| e.to_mcp())?;
                // Check the size before reading so a huge file is rejected
                // without buffering it.
                let metadata = tokio::fs::metadata(&resolved).await.map_err(|e| {
                    Error::Io {
                        detail: format!("could not read {path}: {e}"),
                    }
                    .to_mcp()
                })?;
                if metadata.len() > MAX_UPLOAD_BYTES as u64 {
                    return Err(oversized_upload(metadata.len() as usize).to_mcp());
                }
                let bytes = tokio::fs::read(&resolved).await.map_err(|e| {
                    Error::Io {
                        detail: format!("could not read {path}: {e}"),
                    }
                    .to_mcp()
                })?;
                let file_name = match args.file_name {
                    Some(name) => name,
                    None => resolved
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
                // Resolve against the sandbox first; the write goes to the
                // resolved path (canonical parent + file name).
                let resolved = self
                    .attachment_sandbox()
                    .resolve_download_path(&path)
                    .map_err(|e| e.to_mcp())?;
                let (size_bytes, mime) = self
                    .client()
                    .download_attachment_to_file(task_id, attachment_id, &resolved)
                    .await?;
                Ok(Json(DownloadResult {
                    task_id,
                    attachment_id,
                    mime,
                    size_bytes,
                    saved_to: Some(resolved.display().to_string()),
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
        description = "List Vikunja teams the user belongs to, or — with project_id — the teams that have access to a project including their permission level (0 read, 1 write, 2 admin). With auto_paginate, fetches up to max_pages pages (default 10, max 50) as one bounded result.",
        annotations(read_only_hint = true)
    )]
    pub async fn teams_list(
        &self,
        Parameters(args): Parameters<TeamsListArgs>,
    ) -> Result<Json<TeamListResult>, McpError> {
        let cap = auto_page_cap(args.page, args.auto_paginate, args.max_pages)?;
        let params = page_params(args.page, args.per_page)?;
        let project_id = match args.project_id {
            Some(project_id) => Some(positive("project_id", project_id)?),
            None => None,
        };
        if let Some(cap) = cap {
            let bounded = walk_pages(cap, |page| {
                let params = PageParams::new(Some(page), args.per_page);
                let search = args.search.as_deref();
                async move {
                    match project_id {
                        Some(project_id) => {
                            self.client()
                                .list_project_teams(project_id, params, search)
                                .await
                        }
                        None => self.client().list_teams(params, search).await,
                    }
                }
            })
            .await?;
            return Ok(Json(TeamListResult {
                pagination: bounded.last_info.clone(),
                auto_pagination: Some(AutoPagination::from_bounded(&bounded)),
                teams: bounded.items,
            }));
        }
        let page = match project_id {
            Some(project_id) => {
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
            auto_pagination: None,
        }))
    }

    // -- Project views & kanban buckets --

    #[tool(
        name = "vikunja_project_views_list",
        description = "List the views (list, gantt, table, kanban) configured for a Vikunja project. Kanban boards are views with view_kind 'kanban'; use their id with vikunja_buckets_list.",
        annotations(read_only_hint = true)
    )]
    pub async fn project_views_list(
        &self,
        Parameters(args): Parameters<ProjectViewsListArgs>,
    ) -> Result<Json<ProjectViewListResult>, McpError> {
        let project_id = positive("project_id", args.project_id)?;
        let params = page_params(args.page, args.per_page)?;
        let page = self.client().list_project_views(project_id, params).await?;
        Ok(Json(ProjectViewListResult {
            views: page.items,
            pagination: page.info,
        }))
    }

    #[tool(
        name = "vikunja_buckets_list",
        description = "List the Kanban buckets (board lanes like Backlog/Doing/Done) of a Vikunja project, including each bucket's name and the tasks in it. Without view_id, the project's first kanban view is used.",
        annotations(read_only_hint = true)
    )]
    pub async fn buckets_list(
        &self,
        Parameters(args): Parameters<BucketsListArgs>,
    ) -> Result<Json<BucketsListResult>, McpError> {
        let project_id = positive("project_id", args.project_id)?;
        if let Some(view_id) = args.view_id {
            positive("view_id", view_id)?;
        }
        let params = page_params(args.page, args.per_page)?;
        Ok(Json(
            load_project_buckets(self.client(), project_id, args.view_id, params).await?,
        ))
    }

    // -- Saved filters --
    //
    // Vikunja stores saved filters behind /filters/{id} but has no list
    // endpoint; each filter also appears in the project list as a
    // pseudo-project with id `-filter_id - 1`. The list tool resolves that
    // mapping so agents never have to deal with negative ids.

    #[tool(
        name = "vikunja_filters_list",
        description = "List the user's saved Vikunja filters: durable, named task queries. Returns each filter's id, title and the negative pseudo-project id Vikunja lists it under. Use vikunja_filters_get for the stored query.",
        annotations(read_only_hint = true)
    )]
    pub async fn filters_list(
        &self,
        Parameters(_args): Parameters<FiltersListArgs>,
    ) -> Result<Json<SavedFilterListResult>, McpError> {
        let filters = self
            .client()
            .list_saved_filters(MAX_FILTER_LIST_PAGES)
            .await?;
        Ok(Json(SavedFilterListResult { filters }))
    }

    #[tool(
        name = "vikunja_filters_get",
        description = "Get one saved Vikunja filter by id, including its stored filter expression, sort order and date semantics.",
        annotations(read_only_hint = true)
    )]
    pub async fn filters_get(
        &self,
        Parameters(args): Parameters<FilterIdArgs>,
    ) -> Result<Json<SavedFilter>, McpError> {
        let id = positive("filter_id", args.filter_id)?;
        Ok(Json(self.client().get_saved_filter(id).await?))
    }

    #[tool(
        name = "vikunja_filters_create",
        description = "Create a saved Vikunja filter from a filter expression (e.g. 'done = false && priority >= 3') plus optional sort order. The expression is checked for balanced parentheses and quotes before the write."
    )]
    pub async fn filters_create(
        &self,
        Parameters(args): Parameters<FiltersCreateArgs>,
    ) -> Result<Json<SavedFilter>, McpError> {
        non_empty("title", &args.title)?;
        validate_filter_expression("filter", &args.filter)?;
        validate_sort_order(args.sort_by.as_deref(), args.order_by.as_deref())?;
        if let Some(timezone) = args.filter_timezone.as_deref() {
            non_empty("filter_timezone", timezone)?;
        }
        let body = SavedFilterCreate {
            title: args.title,
            description: args.description,
            filters: SavedFilterQuery {
                filter: Some(args.filter),
                sort_by: args.sort_by,
                order_by: args.order_by,
                filter_timezone: args.filter_timezone,
                filter_include_nulls: args.filter_include_nulls,
            },
            is_favorite: args.is_favorite,
        };
        Ok(Json(self.client().create_saved_filter(&body).await?))
    }

    #[tool(
        name = "vikunja_filters_update",
        description = "Update a saved Vikunja filter. Only provided fields change: the stored query is merged field by field, so e.g. changing the filter expression keeps the stored sort order.",
        annotations(idempotent_hint = true)
    )]
    pub async fn filters_update(
        &self,
        Parameters(args): Parameters<FiltersUpdateArgs>,
    ) -> Result<Json<SavedFilter>, McpError> {
        let id = positive("filter_id", args.filter_id)?;
        if let Some(title) = args.title.as_deref() {
            non_empty("title", title)?;
        }
        if let Some(filter) = args.filter.as_deref() {
            validate_filter_expression("filter", filter)?;
        }
        validate_sort_order(args.sort_by.as_deref(), args.order_by.as_deref())?;
        if let Some(timezone) = args.filter_timezone.as_deref() {
            non_empty("filter_timezone", timezone)?;
        }
        let query = SavedFilterQuery {
            filter: args.filter,
            sort_by: args.sort_by,
            order_by: args.order_by,
            filter_timezone: args.filter_timezone,
            filter_include_nulls: args.filter_include_nulls,
        };
        let patch = SavedFilterUpdate {
            title: args.title,
            description: args.description,
            filters: (query != SavedFilterQuery::default()).then_some(query),
            is_favorite: args.is_favorite,
        };
        Ok(Json(self.client().update_saved_filter(id, &patch).await?))
    }

    #[tool(
        name = "vikunja_filters_delete",
        description = "Delete a saved Vikunja filter. Tasks are not affected; only the stored query is removed. This cannot be undone.",
        annotations(destructive_hint = true)
    )]
    pub async fn filters_delete(
        &self,
        Parameters(args): Parameters<FilterIdArgs>,
    ) -> Result<Json<OperationResult>, McpError> {
        let id = positive("filter_id", args.filter_id)?;
        let message = self.client().delete_saved_filter(id).await?;
        Ok(Json(OperationResult {
            ok: true,
            message: message.message,
        }))
    }

    #[tool(
        name = "vikunja_filters_tasks",
        description = "List the tasks matching a saved Vikunja filter by evaluating its stored query (filter expression, timezone/null handling and the first stored sort pair). Paginated like vikunja_tasks_list.",
        annotations(read_only_hint = true)
    )]
    pub async fn filters_tasks(
        &self,
        Parameters(args): Parameters<FilterTasksArgs>,
    ) -> Result<Json<FilterTasksResult>, McpError> {
        let id = positive("filter_id", args.filter_id)?;
        page_params(args.page, args.per_page)?;
        let filter = self.client().get_saved_filter(id).await?;
        let mut options = saved_filter_options(&filter);
        options.page = args.page;
        options.per_page = args.per_page;
        let page = self.client().list_tasks(&options).await?;
        Ok(Json(FilterTasksResult {
            filter_id: id,
            title: filter.title,
            filter: options.filter,
            sort_by: options.sort_by,
            order_by: options.order_by,
            tasks: page.items,
            pagination: page.info,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_item_error_maps_api_errors_with_status_and_code() {
        let err = Error::Api {
            endpoint: "tasks.update",
            status: 404,
            kind: ApiErrorKind::NotFound,
            code: Some(4002),
            message: "The task does not exist.".to_string(),
        };
        let detail = BulkItemError::from_error(&err);
        assert_eq!(detail.kind, "not_found");
        assert_eq!(detail.http_status, Some(404));
        assert_eq!(detail.vikunja_error_code, Some(4002));
        assert!(detail.message.contains("does not exist"));
    }

    #[test]
    fn bulk_item_error_covers_every_api_kind() {
        for (status, expected) in [
            (401u16, "auth"),
            (403, "forbidden"),
            (404, "not_found"),
            (400, "validation"),
            (412, "validation"),
            (422, "validation"),
            (429, "rate_limited"),
            (500, "server"),
            (418, "other"),
        ] {
            let detail =
                BulkItemError::from_error(&Error::from_status("tasks.update", status, b""));
            assert_eq!(detail.kind, expected, "status {status}");
            assert_eq!(detail.http_status, Some(status));
        }
    }

    #[test]
    fn bulk_item_error_maps_non_api_errors_without_status() {
        let cases: Vec<(Error, &str)> = vec![
            (
                Error::Network {
                    endpoint: "tasks.update",
                    detail: "connection failed".to_string(),
                },
                "network",
            ),
            (
                Error::Timeout {
                    endpoint: "tasks.update",
                },
                "timeout",
            ),
            (
                Error::InvalidResponse {
                    endpoint: "tasks.update",
                    detail: "expected JSON".to_string(),
                },
                "invalid_response",
            ),
            (
                Error::Io {
                    detail: "permission denied".to_string(),
                },
                "io",
            ),
            (
                Error::TooLarge {
                    endpoint: "attachments.download",
                    size: Some(5),
                    limit: 2,
                },
                "too_large",
            ),
            (
                Error::InvalidArgument("nope".to_string()),
                "invalid_argument",
            ),
        ];
        for (err, expected) in cases {
            let detail = BulkItemError::from_error(&err);
            assert_eq!(detail.kind, expected);
            assert_eq!(detail.http_status, None);
            assert_eq!(detail.vikunja_error_code, None);
            assert!(!detail.message.is_empty());
        }
    }

    #[test]
    fn validate_filter_expression_accepts_well_formed_expressions() {
        for expression in [
            "done = false",
            "(done = false) && priority >= 3",
            "title ~ 'has (parens) inside'",
            "title ~ \"it's quoted\"",
            "due_date < now/d+7d",
        ] {
            assert!(
                validate_filter_expression("filter", expression).is_ok(),
                "{expression} should validate"
            );
        }
    }

    #[test]
    fn validate_filter_expression_rejects_malformed_expressions() {
        for (expression, expected) in [
            ("", "empty"),
            ("   ", "empty"),
            ("(done = false", "parenthes"),
            ("done = false)", "parenthes"),
            ("((done = false) && x = 1", "parenthes"),
            ("title ~ 'unterminated", "quote"),
            ("title ~ \"unterminated", "quote"),
        ] {
            let err = validate_filter_expression("filter", expression).unwrap_err();
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
            assert!(
                err.message.contains(expected),
                "{expression:?}: expected '{expected}' in '{}'",
                err.message
            );
        }
    }

    #[test]
    fn validate_sort_order_checks_lengths_and_directions() {
        let fields = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert!(validate_sort_order(None, None).is_ok());
        assert!(validate_sort_order(Some(&fields(&["due_date"])), None).is_ok());
        assert!(validate_sort_order(Some(&fields(&["due_date"])), Some(&fields(&["asc"]))).is_ok());
        assert!(
            validate_sort_order(
                Some(&fields(&["due_date", "id"])),
                Some(&fields(&["asc", "desc"]))
            )
            .is_ok()
        );

        let mismatch =
            validate_sort_order(Some(&fields(&["due_date", "id"])), Some(&fields(&["asc"])))
                .unwrap_err();
        assert!(mismatch.message.contains("same number"));

        let bad_direction = validate_sort_order(None, Some(&fields(&["upward"]))).unwrap_err();
        assert!(bad_direction.message.contains("'asc' or 'desc'"));
    }

    #[test]
    fn validate_task_ids_accepts_positive_ids_only() {
        assert!(validate_task_ids(&[1, 2, 3]).is_ok());
        assert!(validate_task_ids(&[]).is_err());
        assert!(validate_task_ids(&[0]).is_err());
        assert!(validate_task_ids(&[5, -1]).is_err());
    }

    #[test]
    fn validate_task_ids_enforces_batch_cap() {
        let at_cap: Vec<i64> = (1..=MAX_BULK_TASK_IDS as i64).collect();
        assert!(validate_task_ids(&at_cap).is_ok());

        let over_cap: Vec<i64> = (1..=MAX_BULK_TASK_IDS as i64 + 1).collect();
        let err = validate_task_ids(&over_cap).unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("task_ids"));
        assert!(err.message.contains(&MAX_BULK_TASK_IDS.to_string()));
    }
}
