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
