//! Deterministic export formatting: tasks and projects to JSON, Markdown
//! checklists and CSV. All functions here are pure.

use schemars::JsonSchema;
use serde::Serialize;

use crate::vikunja::models::{Project, Task};

/// The zero timestamp Vikunja uses for unset dates.
const UNSET_DATE: &str = "0001-01-01T00:00:00Z";

/// Trimmed, deterministic view of a task shared by all export formats.
/// Field order here defines the JSON field order and the CSV column order.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ExportTask {
    pub id: i64,
    /// Human readable identifier like `PROJ-12`.
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub done: bool,
    /// RFC 3339, omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// 0 unset, 1 low .. 5 DO NOW.
    pub priority: i64,
    /// Completion fraction between 0 and 1.
    pub percent_done: f64,
    pub project_id: i64,
    /// Label titles, sorted ascending.
    pub labels: Vec<String>,
    /// Assignee usernames, sorted ascending.
    pub assignees: Vec<String>,
}

impl ExportTask {
    /// Normalizes a Vikunja task for export: unset zero-dates become `None`,
    /// labels and assignees collapse to sorted name lists.
    pub fn from_task(task: &Task) -> Self {
        let mut labels: Vec<String> = task
            .labels
            .iter()
            .flatten()
            .map(|label| label.title.clone())
            .collect();
        labels.sort();
        let mut assignees: Vec<String> = task
            .assignees
            .iter()
            .flatten()
            .map(|user| user.username.clone())
            .collect();
        assignees.sort();
        Self {
            id: task.id,
            identifier: task.identifier.clone(),
            title: task.title.clone(),
            description: task.description.clone(),
            done: task.done,
            due_date: set_date(task.due_date.as_deref()),
            start_date: set_date(task.start_date.as_deref()),
            end_date: set_date(task.end_date.as_deref()),
            priority: task.priority,
            percent_done: task.percent_done,
            project_id: task.project_id,
            labels,
            assignees,
        }
    }
}

/// Keeps a date only when Vikunja actually set it: the zero timestamp and
/// empty strings mean unset.
fn set_date(date: Option<&str>) -> Option<String> {
    date.filter(|value| !value.is_empty() && *value != UNSET_DATE)
        .map(str::to_string)
}

/// Project metadata included in project exports, in stable field order.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ExportProject {
    pub id: i64,
    /// Short prefix used in task identifiers, e.g. `PROJ`.
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub is_archived: bool,
    pub parent_project_id: i64,
}

impl ExportProject {
    pub fn from_project(project: &Project) -> Self {
        Self {
            id: project.id,
            identifier: project.identifier.clone(),
            title: project.title.clone(),
            description: project.description.clone(),
            is_archived: project.is_archived,
            parent_project_id: project.parent_project_id,
        }
    }
}

/// Sorts export tasks by id ascending (the deterministic default order).
pub fn sort_by_id(tasks: &mut [ExportTask]) {
    tasks.sort_by_key(|task| task.id);
}

/// Pretty-printed JSON array of tasks, with a trailing newline.
pub fn tasks_to_json(tasks: &[ExportTask]) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_string_pretty(tasks)?;
    json.push('\n');
    Ok(json)
}

/// Markdown checklist: `- [ ] Title` / `- [x] Title`, one task per line,
/// description lines (if any) indented by two spaces below their task.
/// Other task fields are not represented in Markdown; use JSON or CSV for a
/// complete export.
pub fn tasks_to_markdown(tasks: &[ExportTask]) -> String {
    let mut markdown = String::new();
    for task in tasks {
        let marker = if task.done { 'x' } else { ' ' };
        markdown.push_str(&format!("- [{marker}] {}\n", task.title));
        for line in task
            .description
            .lines()
            .filter(|line| !line.trim().is_empty())
        {
            markdown.push_str(&format!("  {line}\n"));
        }
    }
    markdown
}

/// CSV with a fixed header
/// `id,identifier,title,description,done,due_date,start_date,end_date,priority,percent_done,project_id,labels,assignees`.
/// Unset dates are empty fields; labels/assignees are joined with `|`;
/// fields containing commas, quotes or newlines are quoted with `"` doubled
/// (RFC 4180); rows end with `\n`.
pub fn tasks_to_csv(tasks: &[ExportTask]) -> String {
    let mut csv = String::from(
        "id,identifier,title,description,done,due_date,start_date,end_date,priority,percent_done,project_id,labels,assignees\n",
    );
    for task in tasks {
        let fields = [
            task.id.to_string(),
            task.identifier.clone(),
            task.title.clone(),
            task.description.clone(),
            task.done.to_string(),
            task.due_date.clone().unwrap_or_default(),
            task.start_date.clone().unwrap_or_default(),
            task.end_date.clone().unwrap_or_default(),
            task.priority.to_string(),
            task.percent_done.to_string(),
            task.project_id.to_string(),
            task.labels.join("|"),
            task.assignees.join("|"),
        ];
        let row: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    csv
}

/// Quotes a CSV field when it contains a comma, quote or newline; embedded
/// quotes are doubled (RFC 4180).
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// JSON project export: `{"project": {...}}` plus a `tasks` array when
/// tasks were fetched. Pretty-printed with a trailing newline.
pub fn project_to_json(
    project: &ExportProject,
    tasks: Option<&[ExportTask]>,
) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct ProjectExport<'a> {
        project: &'a ExportProject,
        #[serde(skip_serializing_if = "Option::is_none")]
        tasks: Option<&'a [ExportTask]>,
    }
    let mut json = serde_json::to_string_pretty(&ProjectExport { project, tasks })?;
    json.push('\n');
    Ok(json)
}

/// Markdown project export: an `# Title` heading, the project description
/// (when non-empty), and a `## Tasks` checklist section when tasks were
/// fetched.
pub fn project_to_markdown(project: &ExportProject, tasks: Option<&[ExportTask]>) -> String {
    let mut markdown = format!("# {}\n", project.title);
    if !project.description.is_empty() {
        markdown.push_str(&format!("\n{}\n", project.description));
    }
    if let Some(tasks) = tasks {
        markdown.push_str("\n## Tasks\n\n");
        markdown.push_str(&tasks_to_markdown(tasks));
    }
    markdown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: i64, title: &str) -> ExportTask {
        ExportTask {
            id,
            identifier: format!("T-{id}"),
            title: title.to_string(),
            description: String::new(),
            done: false,
            due_date: None,
            start_date: None,
            end_date: None,
            priority: 0,
            percent_done: 0.0,
            project_id: 4,
            labels: Vec::new(),
            assignees: Vec::new(),
        }
    }

    #[test]
    fn from_task_normalizes_dates_and_sorts_names() {
        let raw: Task = serde_json::from_value(serde_json::json!({
            "id": 7, "title": "Write docs", "description": "body",
            "done": true, "due_date": "2026-07-01T12:00:00Z",
            "start_date": UNSET_DATE, "end_date": "",
            "priority": 3, "percent_done": 0.5, "project_id": 4,
            "identifier": "DOCS-7",
            "labels": [
                {"id": 2, "title": "zeta"},
                {"id": 1, "title": "alpha"}
            ],
            "assignees": [
                {"id": 2, "username": "zoe"},
                {"id": 1, "username": "ada"}
            ]
        }))
        .unwrap();
        let export = ExportTask::from_task(&raw);
        assert_eq!(export.id, 7);
        assert_eq!(export.due_date.as_deref(), Some("2026-07-01T12:00:00Z"));
        assert_eq!(export.start_date, None, "zero date must become None");
        assert_eq!(export.end_date, None, "empty date must become None");
        assert_eq!(export.labels, vec!["alpha", "zeta"]);
        assert_eq!(export.assignees, vec!["ada", "zoe"]);
        assert!(export.done);
        assert_eq!(export.priority, 3);
    }

    #[test]
    fn sort_by_id_orders_ascending() {
        let mut tasks = vec![task(3, "c"), task(1, "a"), task(2, "b")];
        sort_by_id(&mut tasks);
        let ids: Vec<i64> = tasks.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn json_export_is_deterministic_and_round_trips() {
        let tasks = vec![task(1, "First"), task(2, "Second")];
        let json = tasks_to_json(&tasks).unwrap();
        assert!(json.ends_with('\n'), "must end with a newline");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(parsed[1]["title"], "Second");
        // Unset dates are omitted entirely.
        assert!(parsed[0].get("due_date").is_none());
        // Field order is declaration order: id is the first key.
        let first_key = json
            .lines()
            .nth(2)
            .expect("pretty JSON has one key per line");
        assert!(first_key.contains("\"id\""), "got {first_key:?}");
        assert_eq!(
            tasks_to_json(&tasks).unwrap(),
            json,
            "output must be stable"
        );
    }

    #[test]
    fn markdown_export_renders_checklist_with_descriptions() {
        let mut done = task(2, "Ship it");
        done.done = true;
        let mut described = task(1, "Plan");
        described.description = "line one\nline two".to_string();
        let markdown = tasks_to_markdown(&[described, done]);
        assert_eq!(
            markdown,
            "- [ ] Plan\n  line one\n  line two\n- [x] Ship it\n"
        );
    }

    #[test]
    fn markdown_export_of_no_tasks_is_empty() {
        assert_eq!(tasks_to_markdown(&[]), "");
    }

    #[test]
    fn csv_export_escapes_and_joins() {
        let mut tricky = task(1, "Title, with comma");
        tricky.description = "say \"hi\"\nsecond line".to_string();
        tricky.due_date = Some("2026-07-01T12:00:00Z".to_string());
        tricky.priority = 5;
        tricky.percent_done = 0.5;
        tricky.labels = vec!["a".to_string(), "b".to_string()];
        tricky.assignees = vec!["ada".to_string()];
        let csv = tasks_to_csv(&[tricky]);
        let mut lines = csv.splitn(2, '\n');
        assert_eq!(
            lines.next().unwrap(),
            "id,identifier,title,description,done,due_date,start_date,end_date,priority,percent_done,project_id,labels,assignees"
        );
        assert_eq!(
            lines.next().unwrap(),
            "1,T-1,\"Title, with comma\",\"say \"\"hi\"\"\nsecond line\",false,2026-07-01T12:00:00Z,,,5,0.5,4,a|b,ada\n"
        );
    }

    #[test]
    fn csv_export_of_no_tasks_is_header_only() {
        let csv = tasks_to_csv(&[]);
        assert_eq!(csv.lines().count(), 1);
        assert!(csv.ends_with('\n'));
    }

    #[test]
    fn project_json_includes_tasks_only_when_fetched() {
        let project = ExportProject {
            id: 4,
            identifier: "VRM".to_string(),
            title: "vikunja-rust-mcp".to_string(),
            description: String::new(),
            is_archived: false,
            parent_project_id: 0,
        };
        let without = project_to_json(&project, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&without).unwrap();
        assert_eq!(parsed["project"]["id"], 4);
        assert!(parsed.get("tasks").is_none());

        let with = project_to_json(&project, Some(&[task(1, "One")])).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&with).unwrap();
        assert_eq!(parsed["tasks"][0]["title"], "One");
    }

    #[test]
    fn project_markdown_renders_heading_description_and_tasks() {
        let project = ExportProject {
            id: 4,
            identifier: String::new(),
            title: "Backlog".to_string(),
            description: "Quarterly plan".to_string(),
            is_archived: false,
            parent_project_id: 0,
        };
        assert_eq!(
            project_to_markdown(&project, None),
            "# Backlog\n\nQuarterly plan\n"
        );
        assert_eq!(
            project_to_markdown(&project, Some(&[task(1, "One")])),
            "# Backlog\n\nQuarterly plan\n\n## Tasks\n\n- [ ] One\n"
        );
        let plain = ExportProject {
            description: String::new(),
            ..project
        };
        assert_eq!(project_to_markdown(&plain, None), "# Backlog\n");
    }

    #[test]
    fn export_project_from_project_copies_fields() {
        let raw: Project = serde_json::from_value(serde_json::json!({
            "id": 4, "title": "Backlog", "description": "d",
            "identifier": "BL", "is_archived": true, "parent_project_id": 2
        }))
        .unwrap();
        let export = ExportProject::from_project(&raw);
        assert_eq!(export.id, 4);
        assert_eq!(export.identifier, "BL");
        assert!(export.is_archived);
        assert_eq!(export.parent_project_id, 2);
    }
}
