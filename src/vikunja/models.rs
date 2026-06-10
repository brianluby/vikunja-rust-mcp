//! Serde models for the Vikunja API, following the upstream swagger spec
//! (Vikunja >= 1.0). Timestamps are kept as RFC 3339 strings exactly as the
//! API returns them; Vikunja uses `0001-01-01T00:00:00Z` for unset dates.
//!
//! Read models declare only the fields this server exposes and ignore the
//! rest. Write payloads are separate types so that `None` fields are omitted
//! from request bodies.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use schemars::transform::RecursiveTransform;
use serde::{Deserialize, Serialize};

use crate::schema::strip_unsigned_formats;

/// A Vikunja user (`user.User`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct User {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// A project (`models.Project`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Short prefix used in task identifiers, e.g. `PROJ` in `PROJ-12`.
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub hex_color: String,
    #[serde(default)]
    pub parent_project_id: i64,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub position: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Payload for `PUT /projects`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProjectCreate {
    /// Project title (required by Vikunja).
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hex color without the leading `#`, e.g. `e8b71c`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
    /// Parent project id for nested projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_project_id: Option<i64>,
    /// Short identifier prefix for task numbers, e.g. `PROJ`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
}

/// Partial update for `POST /projects/{id}`. Fields left as `None` keep
/// their current value (the client merges the patch onto the fetched
/// project before sending, since Vikunja zeroes omitted fields).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProjectUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_project_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
}

/// A label (`models.Label`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Label {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub hex_color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Payload for `PUT /labels`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct LabelCreate {
    /// Label title (required by Vikunja).
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hex color without the leading `#`, e.g. `e8b71c`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
}

/// Partial update for `PUT /labels/{id}` (merged onto the current label).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct LabelUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
}

/// A task (`models.Task`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Task priority: 0 unset, 1 low, 2 medium, 3 high, 4 urgent, 5 DO NOW.
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub percent_done: f64,
    #[serde(default)]
    pub project_id: i64,
    /// Human readable identifier like `PROJ-12`.
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub hex_color: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<Label>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<User>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<User>,
    /// Seconds after which the task repeats when completed.
    #[serde(default)]
    pub repeat_after: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// Related tasks grouped by relation kind, as returned by
    /// `GET /tasks/{id}`. Keys are kept as strings so kinds added by a newer
    /// Vikunja server do not break deserialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_tasks: Option<BTreeMap<String, Vec<Task>>>,
    /// Reminders attached to the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Vec<TaskReminder>>,
}

/// A task reminder (`models.TaskReminder`): either an absolute `reminder`
/// timestamp, or a period relative to one of the task's dates — Vikunja
/// then computes (and recomputes) the absolute time itself. Reminders have
/// no dedicated endpoints; they are read from the task and written by
/// replacing the task's `reminders` array on update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskReminder {
    /// Absolute reminder time as RFC 3339. Filled in by the server for
    /// relative reminders once the anchor date exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<String>,
    /// Offset in seconds from `relative_to`; negative means before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_period: Option<i64>,
    /// Task date the period is relative to: `due_date`, `start_date` or
    /// `end_date`. Kept as a string on read so future anchors do not break
    /// deserialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_to: Option<String>,
}

/// Kind of a relation between two tasks (`models.RelationKind`). Vikunja's
/// `unknown` kind is intentionally not accepted: it is never valid in a
/// create or delete request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
// `lowercase` folds multi-word variants without separators (`ParentTask` ->
// `parenttask`), matching Vikunja exactly; `as_str` must stay in sync.
#[serde(rename_all = "lowercase")]
pub enum RelationKind {
    /// The other task is a subtask of this task.
    Subtask,
    /// The other task is the parent of this task.
    ParentTask,
    /// The tasks are loosely related.
    Related,
    /// This task is a duplicate of the other task.
    DuplicateOf,
    /// The other task duplicates this task.
    Duplicates,
    /// This task blocks the other task.
    Blocking,
    /// This task is blocked by the other task.
    Blocked,
    /// This task precedes the other task.
    Precedes,
    /// This task follows the other task.
    Follows,
    /// This task was copied from the other task.
    CopiedFrom,
    /// The other task was copied from this task.
    CopiedTo,
}

impl RelationKind {
    /// The lowercase string Vikunja uses for this kind in JSON bodies and
    /// URL paths.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subtask => "subtask",
            Self::ParentTask => "parenttask",
            Self::Related => "related",
            Self::DuplicateOf => "duplicateof",
            Self::Duplicates => "duplicates",
            Self::Blocking => "blocking",
            Self::Blocked => "blocked",
            Self::Precedes => "precedes",
            Self::Follows => "follows",
            Self::CopiedFrom => "copiedfrom",
            Self::CopiedTo => "copiedto",
        }
    }
}

/// A relation between two tasks (`models.TaskRelation`), as returned by
/// `PUT /tasks/{taskID}/relations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRelation {
    #[serde(default)]
    pub task_id: i64,
    #[serde(default)]
    pub other_task_id: i64,
    pub relation_kind: RelationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

/// Payload for `PUT /projects/{id}/tasks`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TaskCreate {
    /// Task title (required by Vikunja).
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Due date as RFC 3339, e.g. `2026-07-01T12:00:00Z`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Task priority: 0 unset, 1 low, 2 medium, 3 high, 4 urgent, 5 DO NOW.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Completion percentage between 0 and 1 (e.g. 0.5 for 50%).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent_done: Option<f64>,
    /// Hex color without the leading `#`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// Seconds after which the task should repeat once completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_after: Option<i64>,
}

/// Partial update for `POST /tasks/{id}` (merged onto the current task by
/// the client, since Vikunja zeroes omitted fields on update).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TaskUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    /// Due date as RFC 3339. Use `0001-01-01T00:00:00Z` to clear it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent_done: Option<f64>,
    /// Move the task to another project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_after: Option<i64>,
    /// Replaces the task's reminder list wholesale; an empty list clears
    /// all reminders. `None` leaves reminders untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Vec<TaskReminder>>,
}

/// A task comment (`models.TaskComment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskComment {
    pub id: i64,
    #[serde(default)]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Metadata about an uploaded file (`files.File`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct FileMeta {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mime: String,
    /// File size in bytes. Vikunja >= 1.0 sends this as a number; older
    /// versions sent a string, so both are accepted.
    #[serde(default, deserialize_with = "deserialize_size")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

/// A task attachment (`models.TaskAttachment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskAttachment {
    pub id: i64,
    #[serde(default)]
    pub task_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

/// A team member (`models.TeamUser`): a user plus team admin flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TeamMember {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub admin: bool,
}

/// A team (`models.Team`). When listed for a project
/// (`GET /projects/{id}/teams`), Vikunja additionally reports the team's
/// `permission` on that project (0 read, 1 write, 2 admin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Team {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<TeamMember>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<User>,
    /// Only present when listing teams of a project: 0 read, 1 write, 2 admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// The query stored inside a saved filter (`models.TaskCollection`):
/// a Vikunja filter expression plus sort order and date semantics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SavedFilterQuery {
    /// Vikunja filter expression, e.g. `done = false && priority >= 3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Fields to sort by, e.g. `["due_date", "id"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<Vec<String>>,
    /// Sort directions matching `sort_by` position by position, e.g.
    /// `["asc", "desc"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_by: Option<Vec<String>>,
    /// IANA timezone used to resolve relative dates like `now/d`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_timezone: Option<String>,
    /// Whether tasks with a null value in a filtered field match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_include_nulls: Option<bool>,
}

/// A saved filter (`models.SavedFilter`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SavedFilter {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The stored query this filter evaluates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<SavedFilterQuery>,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Payload for `PUT /filters`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SavedFilterCreate {
    /// Filter title (required by Vikunja).
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The query to store.
    pub filters: SavedFilterQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
}

/// Partial update for `POST /filters/{id}`. Fields left as `None` keep
/// their current value; inside `filters`, individual query fields are
/// merged onto the stored query the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SavedFilterUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<SavedFilterQuery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
}

/// Lightweight saved-filter listing entry. Vikunja has no `GET /filters`
/// endpoint; instead each saved filter appears in the project list as a
/// pseudo-project with id `-filter_id - 1` (so ids <= -2 are filters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SavedFilterSummary {
    /// Numeric id of the saved filter, for use with the filter tools.
    pub filter_id: i64,
    /// The negative pseudo-project id Vikunja lists this filter under.
    pub pseudo_project_id: i64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_favorite: bool,
}

impl SavedFilterSummary {
    /// Interprets a project-list entry as a saved filter, if its id is in
    /// the pseudo-project range.
    pub fn from_project(project: &Project) -> Option<Self> {
        let filter_id = saved_filter_id_from_project_id(project.id)?;
        Some(Self {
            filter_id,
            pseudo_project_id: project.id,
            title: project.title.clone(),
            description: project.description.clone(),
            is_favorite: project.is_favorite,
        })
    }
}

/// The pseudo-project id Vikunja uses for a saved filter.
pub fn saved_filter_pseudo_project_id(filter_id: i64) -> i64 {
    -filter_id - 1
}

/// The saved filter id behind a pseudo-project id, if it is one
/// (ids <= -2; -1 is reserved and positive ids are real projects).
/// Project ids come from API responses, so `i64::MIN` (whose negation is
/// not representable) is rejected instead of overflowing.
pub fn saved_filter_id_from_project_id(project_id: i64) -> Option<i64> {
    if project_id > -2 {
        return None;
    }
    project_id.checked_neg().map(|negated| negated - 1)
}

/// Generic `{"message": "..."}` response (`models.Message`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    #[serde(default)]
    pub message: String,
}

/// Response of `PUT /tasks/{task}/labels` (`models.LabelTask`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabelTask {
    #[serde(default)]
    pub label_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

/// Response of `PUT /tasks/{taskID}/assignees` (`models.TaskAssginee`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskAssignee {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

/// Accepts a file size as either a JSON number or a string.
fn deserialize_size<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SizeRepr {
        Num(u64),
        Str(String),
        Null,
    }
    match SizeRepr::deserialize(deserializer)? {
        SizeRepr::Num(n) => Ok(Some(n)),
        SizeRepr::Str(s) => Ok(s.parse().ok()),
        SizeRepr::Null => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_deserializes_from_api_shape() {
        let json = serde_json::json!({
            "id": 7, "title": "Write docs", "description": "<p>hi</p>",
            "done": false, "done_at": "0001-01-01T00:00:00Z",
            "due_date": "2026-07-01T12:00:00Z",
            "priority": 2, "percent_done": 0.5, "project_id": 3,
            "identifier": "DOCS-7", "index": 7, "hex_color": "e8b71c",
            "is_favorite": true,
            "labels": [{"id": 1, "title": "urgent", "description": "", "hex_color": "ff0000"}],
            "assignees": [{"id": 2, "username": "ada", "name": "Ada"}],
            "created_by": {"id": 1, "username": "root"},
            "repeat_after": 0,
            "created": "2026-01-01T00:00:00Z", "updated": "2026-01-02T00:00:00Z",
            "some_future_field": {"ignored": true}
        });
        let task: Task = serde_json::from_value(json).unwrap();
        assert_eq!(task.id, 7);
        assert_eq!(task.title, "Write docs");
        assert_eq!(task.priority, 2);
        assert_eq!(task.percent_done, 0.5);
        assert_eq!(task.labels.as_ref().unwrap()[0].title, "urgent");
        assert_eq!(task.assignees.as_ref().unwrap()[0].username, "ada");
        assert_eq!(task.identifier, "DOCS-7");
    }

    #[test]
    fn task_reminder_serde_round_trip() {
        // Absolute reminder: only the timestamp is on the wire.
        let absolute = TaskReminder {
            reminder: Some("2026-07-01T09:00:00Z".into()),
            relative_period: None,
            relative_to: None,
        };
        let value = serde_json::to_value(&absolute).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"reminder": "2026-07-01T09:00:00Z"})
        );
        let back: TaskReminder = serde_json::from_value(value).unwrap();
        assert_eq!(back, absolute);

        // Relative reminder: one hour before the due date.
        let relative: TaskReminder = serde_json::from_value(serde_json::json!({
            "reminder": "2026-06-30T08:00:00Z",
            "relative_period": -3600,
            "relative_to": "due_date"
        }))
        .unwrap();
        assert_eq!(relative.relative_period, Some(-3600));
        assert_eq!(relative.relative_to.as_deref(), Some("due_date"));
    }

    #[test]
    fn task_parses_reminders_and_defaults_to_none() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "t", "project_id": 2
        }))
        .unwrap();
        assert!(task.reminders.is_none());
        // None must stay invisible when serialized (backward compatibility).
        let value = serde_json::to_value(&task).unwrap();
        assert!(value.get("reminders").is_none());

        let task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "t", "project_id": 2,
            "reminders": [
                {"reminder": "2026-07-01T09:00:00Z"},
                {"relative_period": -600, "relative_to": "start_date"}
            ]
        }))
        .unwrap();
        let reminders = task.reminders.unwrap();
        assert_eq!(reminders.len(), 2);
        assert_eq!(
            reminders[0].reminder.as_deref(),
            Some("2026-07-01T09:00:00Z")
        );
        assert_eq!(reminders[1].relative_period, Some(-600));
    }

    #[test]
    fn task_update_reminders_serialization() {
        // Omitted: the merge must not touch reminders.
        let patch = TaskUpdate::default();
        let value = serde_json::to_value(&patch).unwrap();
        assert!(value.get("reminders").is_none());

        // Empty list: explicit clear.
        let patch = TaskUpdate {
            reminders: Some(vec![]),
            ..Default::default()
        };
        let value = serde_json::to_value(&patch).unwrap();
        assert_eq!(value["reminders"], serde_json::json!([]));

        // Replacement list.
        let patch = TaskUpdate {
            reminders: Some(vec![TaskReminder {
                reminder: Some("2026-07-01T09:00:00Z".into()),
                relative_period: None,
                relative_to: None,
            }]),
            ..Default::default()
        };
        let value = serde_json::to_value(&patch).unwrap();
        assert_eq!(
            value["reminders"],
            serde_json::json!([{"reminder": "2026-07-01T09:00:00Z"}])
        );
    }

    #[test]
    fn relation_kind_serializes_to_vikunja_strings() {
        let cases: &[(RelationKind, &str)] = &[
            (RelationKind::Subtask, "subtask"),
            (RelationKind::ParentTask, "parenttask"),
            (RelationKind::Related, "related"),
            (RelationKind::DuplicateOf, "duplicateof"),
            (RelationKind::Duplicates, "duplicates"),
            (RelationKind::Blocking, "blocking"),
            (RelationKind::Blocked, "blocked"),
            (RelationKind::Precedes, "precedes"),
            (RelationKind::Follows, "follows"),
            (RelationKind::CopiedFrom, "copiedfrom"),
            (RelationKind::CopiedTo, "copiedto"),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(expected),
                "serializing {kind:?}"
            );
            let back: RelationKind = serde_json::from_value(serde_json::json!(expected)).unwrap();
            assert_eq!(back, *kind, "deserializing {expected}");
            assert_eq!(kind.as_str(), *expected);
        }
    }

    #[test]
    fn relation_kind_rejects_unknown_values() {
        for bad in ["unknown", "blocks", "SUBTASK", ""] {
            assert!(
                serde_json::from_value::<RelationKind>(serde_json::json!(bad)).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn task_relation_deserializes_from_api_shape() {
        let relation: TaskRelation = serde_json::from_value(serde_json::json!({
            "task_id": 5, "other_task_id": 9, "relation_kind": "precedes",
            "created_by": {"id": 1, "username": "ada"},
            "created": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(relation.task_id, 5);
        assert_eq!(relation.other_task_id, 9);
        assert_eq!(relation.relation_kind, RelationKind::Precedes);
    }

    #[test]
    fn task_related_tasks_default_to_none_and_round_trip() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "t", "project_id": 2
        }))
        .unwrap();
        assert!(task.related_tasks.is_none());
        // None must stay invisible when serialized (backward compatibility).
        let value = serde_json::to_value(&task).unwrap();
        assert!(value.get("related_tasks").is_none());

        let task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "t", "project_id": 2,
            "related_tasks": {"subtask": [{"id": 4, "title": "child", "project_id": 2}]}
        }))
        .unwrap();
        let related = task.related_tasks.unwrap();
        assert_eq!(related["subtask"][0].id, 4);
    }

    #[test]
    fn task_create_omits_unset_fields() {
        let body = TaskCreate {
            title: "New".into(),
            ..Default::default()
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value, serde_json::json!({"title": "New"}));
    }

    #[test]
    fn task_update_serializes_only_set_fields() {
        let patch = TaskUpdate {
            done: Some(true),
            priority: Some(4),
            ..Default::default()
        };
        let value = serde_json::to_value(&patch).unwrap();
        assert_eq!(value, serde_json::json!({"done": true, "priority": 4}));
    }

    #[test]
    fn project_create_omits_unset_fields() {
        let body = ProjectCreate {
            title: "Inbox".into(),
            hex_color: Some("00ff00".into()),
            ..Default::default()
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"title": "Inbox", "hex_color": "00ff00"})
        );
    }

    #[test]
    fn file_size_accepts_number_and_string() {
        let num: FileMeta = serde_json::from_value(
            serde_json::json!({"id": 1, "name": "a", "mime": "text/plain", "size": 42}),
        )
        .unwrap();
        assert_eq!(num.size, Some(42));
        let text: FileMeta = serde_json::from_value(
            serde_json::json!({"id": 1, "name": "a", "mime": "", "size": "1337"}),
        )
        .unwrap();
        assert_eq!(text.size, Some(1337));
        let absent: FileMeta =
            serde_json::from_value(serde_json::json!({"id": 1, "name": "a", "mime": ""})).unwrap();
        assert_eq!(absent.size, None);
    }

    #[test]
    fn team_with_permission_deserializes() {
        let json = serde_json::json!({
            "id": 4, "name": "devs", "description": "",
            "permission": 1,
            "members": [{"id": 1, "username": "ada", "admin": true}]
        });
        let team: Team = serde_json::from_value(json).unwrap();
        assert_eq!(team.permission, Some(1));
        assert!(team.members.unwrap()[0].admin);
    }

    #[test]
    fn saved_filter_round_trips_and_ignores_unknown_fields() {
        let json = serde_json::json!({
            "id": 9, "title": "Open work", "description": "open tasks",
            "filters": {
                "sort_by": ["due_date"], "order_by": ["asc"],
                "filter": "done = false",
                "filter_timezone": "UTC", "filter_include_nulls": true,
                "some_future_field": 1
            },
            "owner": {"id": 1, "username": "ada"},
            "is_favorite": true,
            "created": "2026-01-01T00:00:00Z"
        });
        let filter: SavedFilter = serde_json::from_value(json).unwrap();
        assert_eq!(filter.id, 9);
        assert!(filter.is_favorite);
        let query = filter.filters.as_ref().unwrap();
        assert_eq!(query.filter.as_deref(), Some("done = false"));
        assert_eq!(query.filter_include_nulls, Some(true));
        let back: SavedFilter =
            serde_json::from_value(serde_json::to_value(&filter).unwrap()).unwrap();
        assert_eq!(back, filter);
    }

    #[test]
    fn saved_filter_create_omits_unset_fields() {
        let body = SavedFilterCreate {
            title: "Open".into(),
            filters: SavedFilterQuery {
                filter: Some("done = false".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"title": "Open", "filters": {"filter": "done = false"}})
        );
    }

    #[test]
    fn saved_filter_pseudo_project_ids_convert_both_ways() {
        assert_eq!(saved_filter_pseudo_project_id(1), -2);
        assert_eq!(saved_filter_pseudo_project_id(41), -42);
        assert_eq!(saved_filter_id_from_project_id(-2), Some(1));
        assert_eq!(saved_filter_id_from_project_id(-42), Some(41));
        // Real projects and the reserved -1 id are not filters.
        assert_eq!(saved_filter_id_from_project_id(7), None);
        assert_eq!(saved_filter_id_from_project_id(0), None);
        assert_eq!(saved_filter_id_from_project_id(-1), None);
        // i64::MIN has no representable negation; a corrupted or malicious
        // API response must not overflow.
        assert_eq!(saved_filter_id_from_project_id(i64::MIN), None);
        assert_eq!(
            saved_filter_id_from_project_id(i64::MIN + 1),
            Some(i64::MAX - 1)
        );
    }

    #[test]
    fn saved_filter_summary_derives_only_from_pseudo_projects() {
        let pseudo: Project = serde_json::from_value(serde_json::json!({
            "id": -2, "title": "Open work", "description": "d", "is_favorite": true
        }))
        .unwrap();
        let summary = SavedFilterSummary::from_project(&pseudo).unwrap();
        assert_eq!(summary.filter_id, 1);
        assert_eq!(summary.pseudo_project_id, -2);
        assert_eq!(summary.title, "Open work");
        assert!(summary.is_favorite);

        let real: Project =
            serde_json::from_value(serde_json::json!({"id": 3, "title": "Inbox"})).unwrap();
        assert!(SavedFilterSummary::from_project(&real).is_none());
    }

    #[test]
    fn label_and_comment_round_trip() {
        let label = Label {
            id: 9,
            title: "bug".into(),
            description: String::new(),
            hex_color: "ff0000".into(),
            created_by: None,
            created: None,
            updated: None,
        };
        let back: Label = serde_json::from_value(serde_json::to_value(&label).unwrap()).unwrap();
        assert_eq!(back, label);

        let comment: TaskComment = serde_json::from_value(serde_json::json!({
            "id": 3, "comment": "hello", "author": {"id": 1, "username": "ada"}
        }))
        .unwrap();
        assert_eq!(comment.comment, "hello");
        assert_eq!(comment.author.unwrap().username, "ada");
    }
}
