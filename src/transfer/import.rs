//! Import parsing: Markdown checklists and CSV backlogs to validated task
//! rows. All functions here are pure; date resolution and Vikunja writes
//! happen in the tool layer.
//!
//! Markdown format: one task per `- [ ] Title` (or `- [x] Title`) line at
//! the start of a line; lines indented with whitespace below a task become
//! its description (trimmed, joined with `\n`); blank lines and `#` headings
//! are ignored; anything else is reported as an error row. The `[x]` done
//! marker is accepted but ignored — Vikunja's create endpoint cannot create
//! completed tasks.
//!
//! CSV format (RFC 4180): a header row is required. Supported columns:
//! `title` (required), `description`, `due_date`, `priority`. Unknown or
//! duplicate columns are rejected — in particular `labels`, which this
//! version does not import. Quoted fields may contain commas, quotes
//! (doubled) and newlines; `\r\n` line endings are accepted.

/// One parsed backlog row, before date resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTask {
    /// 1-based input line the row started on.
    pub line: usize,
    pub title: String,
    pub description: Option<String>,
    /// Raw due date string (RFC 3339 or a date shortcut), resolved later.
    pub due_date: Option<String>,
    /// Validated to 0..=5 by the parser.
    pub priority: Option<i64>,
}

/// A row that could not be parsed or validated.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseIssue {
    /// 1-based input line the problem was found on.
    pub line: usize,
    pub message: String,
}

/// One input row: a valid task or a per-row issue.
pub type RowResult = Result<ParsedTask, ParseIssue>;

/// Parses a Markdown checklist into rows. Never fails as a whole; bad lines
/// become `Err` rows so all problems are reported in one pass.
pub fn parse_markdown(input: &str) -> Vec<RowResult> {
    let mut rows: Vec<RowResult> = Vec::new();
    // Index into `rows` of the task that indented lines currently extend.
    let mut open_task: Option<usize> = None;
    for (index, line) in input.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(title) = task_line_title(line) {
            if title.is_empty() {
                rows.push(Err(ParseIssue {
                    line: number,
                    message: "task title must not be empty".to_string(),
                }));
                open_task = None;
            } else {
                rows.push(Ok(ParsedTask {
                    line: number,
                    title: title.to_string(),
                    description: None,
                    due_date: None,
                    priority: None,
                }));
                open_task = Some(rows.len() - 1);
            }
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let Some(task_index) = open_task else {
                rows.push(Err(ParseIssue {
                    line: number,
                    message: "indented description line without a preceding task".to_string(),
                }));
                continue;
            };
            if let Some(Ok(task)) = rows.get_mut(task_index) {
                let trimmed = line.trim();
                match &mut task.description {
                    Some(description) => {
                        description.push('\n');
                        description.push_str(trimmed);
                    }
                    None => task.description = Some(trimmed.to_string()),
                }
            }
            continue;
        }
        rows.push(Err(ParseIssue {
            line: number,
            message: "unrecognized line; tasks must look like `- [ ] Title`".to_string(),
        }));
        open_task = None;
    }
    rows
}

/// The title of a `- [ ] Title` / `- [x] Title` line, or `None` when the
/// line is not a checklist item.
fn task_line_title(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("- [")?;
    let mut chars = rest.chars();
    if !matches!(chars.next(), Some(' ' | 'x' | 'X')) {
        return None;
    }
    let after_marker = chars.as_str().strip_prefix(']')?;
    Some(after_marker.trim())
}

/// Parses a CSV backlog into rows. Whole-document problems (missing or
/// invalid header, unterminated quote) fail with `Err`; per-row problems
/// become `Err` rows in the result.
pub fn parse_csv(input: &str) -> Result<Vec<RowResult>, String> {
    let records = csv_records(input)?;
    let mut records = records.into_iter();
    let Some((_, header)) = records.next() else {
        return Err(
            "input is empty; a CSV header row with at least a title column is required".to_string(),
        );
    };

    let mut columns: Vec<Column> = Vec::with_capacity(header.len());
    for name in &header {
        let column = match name.trim() {
            "title" => Column::Title,
            "description" => Column::Description,
            "due_date" => Column::DueDate,
            "priority" => Column::Priority,
            other => {
                return Err(format!(
                    "unsupported column {other:?}; supported columns are title, description, \
                     due_date and priority (labels cannot be imported in this version)"
                ));
            }
        };
        if columns.contains(&column) {
            return Err(format!("duplicate column {:?}", name.trim()));
        }
        columns.push(column);
    }
    if !columns.contains(&Column::Title) {
        return Err("the CSV header must contain a title column".to_string());
    }

    let mut rows = Vec::new();
    for (line, fields) in records {
        if fields.len() != columns.len() {
            rows.push(Err(ParseIssue {
                line,
                message: format!(
                    "row has {} fields but the header has {} columns",
                    fields.len(),
                    columns.len()
                ),
            }));
            continue;
        }
        let mut task = ParsedTask {
            line,
            title: String::new(),
            description: None,
            due_date: None,
            priority: None,
        };
        let mut issue: Option<ParseIssue> = None;
        for (column, value) in columns.iter().zip(&fields) {
            match column {
                Column::Title => task.title = value.trim().to_string(),
                Column::Description if !value.trim().is_empty() => {
                    task.description = Some(value.clone());
                }
                Column::DueDate if !value.trim().is_empty() => {
                    task.due_date = Some(value.trim().to_string());
                }
                Column::Priority if !value.trim().is_empty() => match value.trim().parse::<i64>() {
                    Ok(priority) if (0..=5).contains(&priority) => {
                        task.priority = Some(priority);
                    }
                    _ => {
                        issue = Some(ParseIssue {
                            line,
                            message: format!(
                                "priority must be an integer between 0 and 5, got {value:?}"
                            ),
                        });
                    }
                },
                _ => {}
            }
        }
        if issue.is_none() && task.title.is_empty() {
            issue = Some(ParseIssue {
                line,
                message: "title must not be empty".to_string(),
            });
        }
        rows.push(match issue {
            Some(issue) => Err(issue),
            None => Ok(task),
        });
    }
    Ok(rows)
}

/// The CSV columns this importer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Title,
    Description,
    DueDate,
    Priority,
}

/// Splits CSV input into records of fields (RFC 4180), each tagged with the
/// 1-based line it starts on. Quoted fields may contain commas, doubled
/// quotes and newlines; `\r\n` endings are accepted; records that are
/// entirely blank are skipped.
fn csv_records(input: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut line = 1usize;
    let mut record_line = 1usize;
    let mut chars = input.chars().peekable();

    let mut end_record = |fields: &mut Vec<String>, field: &mut String, record_line: usize| {
        fields.push(std::mem::take(field));
        // Only a single whitespace-only field is a blank line. A record
        // like `,` is a data row with empty fields and must be kept so
        // row-level validation can report it with its line number.
        let blank = fields.len() == 1 && fields[0].trim().is_empty();
        if !blank {
            records.push((record_line, std::mem::take(fields)));
        } else {
            fields.clear();
        }
    };

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                // CRLF inside a quoted cell (Excel-style) normalizes to \n;
                // a lone \r is kept as data.
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' => {
                    line += 1;
                    field.push('\n');
                }
                _ => field.push(ch),
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => in_quotes = true,
            ',' => fields.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                end_record(&mut fields, &mut field, record_line);
                line += 1;
                record_line = line;
            }
            _ => field.push(ch),
        }
    }
    if in_quotes {
        return Err(format!(
            "unterminated quoted field starting before line {line}: every opening quote needs \
             a closing quote"
        ));
    }
    if !field.is_empty() || !fields.is_empty() {
        end_record(&mut fields, &mut field, record_line);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(rows: &[RowResult]) -> Vec<&ParsedTask> {
        rows.iter().filter_map(|row| row.as_ref().ok()).collect()
    }

    fn issues(rows: &[RowResult]) -> Vec<&ParseIssue> {
        rows.iter().filter_map(|row| row.as_ref().err()).collect()
    }

    // ----- Markdown ---------------------------------------------------------

    #[test]
    fn markdown_parses_simple_checklist() {
        let rows = parse_markdown("- [ ] First\n- [x] Second\n- [X] Third\n");
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "First");
        assert_eq!(tasks[0].line, 1);
        assert_eq!(tasks[1].title, "Second");
        assert_eq!(tasks[2].title, "Third");
        assert!(issues(&rows).is_empty());
        assert!(tasks.iter().all(|t| t.description.is_none()));
    }

    #[test]
    fn markdown_attaches_indented_description_lines() {
        let rows = parse_markdown("- [ ] Plan\n  line one\n\tline two\n- [ ] Next\n");
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description.as_deref(), Some("line one\nline two"));
        assert_eq!(tasks[1].description, None);
    }

    #[test]
    fn markdown_ignores_blank_lines_and_headings() {
        let rows = parse_markdown("# Backlog\n\n- [ ] Only task\n\n## Notes\n");
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Only task");
        assert_eq!(tasks[0].line, 3);
        assert!(issues(&rows).is_empty());
    }

    #[test]
    fn markdown_flags_empty_titles() {
        let rows = parse_markdown("- [ ]   \n");
        let problems = issues(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 1);
        assert!(problems[0].message.contains("title"), "{problems:?}");
    }

    #[test]
    fn markdown_flags_unrecognized_lines() {
        let rows = parse_markdown("- [ ] Fine\njust prose\n* [ ] wrong bullet\n");
        let problems = issues(&rows);
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].line, 2);
        assert!(problems[0].message.contains("- [ ]"), "{problems:?}");
        assert_eq!(problems[1].line, 3);
    }

    #[test]
    fn markdown_flags_indented_line_without_task() {
        let rows = parse_markdown("  floating description\n- [ ] Task\n");
        let problems = issues(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 1);
        assert_eq!(ok(&rows).len(), 1);
    }

    #[test]
    fn markdown_of_empty_input_yields_no_rows() {
        assert!(parse_markdown("").is_empty());
        assert!(parse_markdown("\n\n").is_empty());
    }

    // ----- CSV --------------------------------------------------------------

    #[test]
    fn csv_parses_all_supported_columns() {
        let rows = parse_csv(
            "title,description,due_date,priority\n\
             Write docs,Body text,2026-07-01T12:00:00Z,3\n\
             Minimal,,,\n",
        )
        .unwrap();
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Write docs");
        assert_eq!(tasks[0].description.as_deref(), Some("Body text"));
        assert_eq!(tasks[0].due_date.as_deref(), Some("2026-07-01T12:00:00Z"));
        assert_eq!(tasks[0].priority, Some(3));
        assert_eq!(tasks[0].line, 2);
        assert_eq!(tasks[1].title, "Minimal");
        assert_eq!(tasks[1].description, None);
        assert_eq!(tasks[1].due_date, None);
        assert_eq!(tasks[1].priority, None);
    }

    #[test]
    fn csv_whitespace_only_description_means_none() {
        let rows = parse_csv("title,description\nTask,\"   \"\n").unwrap();
        assert_eq!(ok(&rows)[0].description, None);
    }

    #[test]
    fn csv_title_only_header_is_enough() {
        let rows = parse_csv("title\nJust a title\n").unwrap();
        assert_eq!(ok(&rows)[0].title, "Just a title");
    }

    #[test]
    fn csv_handles_quoted_fields() {
        let rows = parse_csv(
            "title,description\n\
             \"Comma, in title\",\"He said \"\"hi\"\"\nover two lines\"\n\
             After,plain\n",
        )
        .unwrap();
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Comma, in title");
        assert_eq!(
            tasks[0].description.as_deref(),
            Some("He said \"hi\"\nover two lines")
        );
        assert_eq!(tasks[1].title, "After");
        assert_eq!(tasks[1].line, 4, "multi-line field shifts later rows");
    }

    #[test]
    fn csv_quoted_fields_normalize_crlf_to_newline() {
        // Excel-style CSV: CRLF both as record separator and inside quoted
        // multi-line cells. No carriage returns may leak into values.
        let rows = parse_csv("title,description\r\nTask,\"line one\r\nline two\"\r\n").unwrap();
        let tasks = ok(&rows);
        assert_eq!(tasks[0].description.as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn csv_accepts_crlf_line_endings() {
        let rows = parse_csv("title,priority\r\nTask A,1\r\nTask B,\r\n").unwrap();
        let tasks = ok(&rows);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Task A");
        assert_eq!(tasks[0].priority, Some(1));
        assert_eq!(tasks[1].priority, None);
    }

    #[test]
    fn csv_requires_title_column() {
        let err = parse_csv("description,priority\nno title here,1\n").unwrap_err();
        assert!(err.contains("title"), "{err}");
    }

    #[test]
    fn csv_rejects_unknown_columns() {
        let err = parse_csv("title,labels\nTask,urgent\n").unwrap_err();
        assert!(err.contains("labels"), "{err}");
        let err = parse_csv("title,Due Date\nTask,tomorrow\n").unwrap_err();
        assert!(
            err.contains("due date") || err.contains("Due Date"),
            "{err}"
        );
    }

    #[test]
    fn csv_rejects_duplicate_columns() {
        let err = parse_csv("title,title\nTask,Again\n").unwrap_err();
        assert!(err.contains("title"), "{err}");
    }

    #[test]
    fn csv_rejects_empty_input_and_missing_rows_are_fine() {
        assert!(parse_csv("").is_err());
        assert!(parse_csv("   \n").is_err());
        // A header with no data rows is valid and yields no rows.
        let rows = parse_csv("title\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn csv_flags_empty_titles_per_row() {
        let rows = parse_csv("title,priority\n,3\nReal task,2\n").unwrap();
        let problems = issues(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 2);
        assert!(problems[0].message.contains("title"), "{problems:?}");
        assert_eq!(ok(&rows).len(), 1);
    }

    #[test]
    fn csv_flags_invalid_priorities_per_row() {
        let rows = parse_csv("title,priority\nA,high\nB,6\nC,-1\nD,5\n").unwrap();
        let problems = issues(&rows);
        assert_eq!(problems.len(), 3);
        assert!(
            problems.iter().all(|p| p.message.contains("priority")),
            "{problems:?}"
        );
        assert_eq!(problems[0].line, 2);
        assert_eq!(problems[1].line, 3);
        assert_eq!(problems[2].line, 4);
        assert_eq!(ok(&rows)[0].priority, Some(5));
    }

    #[test]
    fn csv_flags_field_count_mismatch_per_row() {
        let rows = parse_csv("title,priority\nA,1,extra\nB\nC,2\n").unwrap();
        let problems = issues(&rows);
        assert_eq!(problems.len(), 2);
        assert!(problems[0].message.contains("2 columns"), "{problems:?}");
        assert_eq!(ok(&rows).len(), 1);
    }

    #[test]
    fn csv_rejects_unterminated_quotes() {
        let err = parse_csv("title\n\"never closed\n").unwrap_err();
        assert!(err.contains("quote"), "{err}");
    }

    #[test]
    fn csv_skips_blank_lines() {
        let rows = parse_csv("title\n\nTask A\n\n").unwrap();
        assert_eq!(ok(&rows).len(), 1);
    }

    #[test]
    fn csv_all_empty_rows_are_reported_not_skipped() {
        // A `,`-only line is a data row with empty fields (RFC 4180), not a
        // blank line: it must surface as a row error, not vanish.
        let rows = parse_csv("title,priority\n,\nReal,1\n").unwrap();
        let problems = issues(&rows);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 2);
        assert!(problems[0].message.contains("title"), "{problems:?}");
        assert_eq!(ok(&rows).len(), 1);

        // Wrong column count of an all-empty row is reported too.
        let rows = parse_csv("title\n,,\nReal\n").unwrap();
        let problems = issues(&rows);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("1 column"), "{problems:?}");
    }
}
