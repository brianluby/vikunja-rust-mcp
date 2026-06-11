//! Import/export backlog workflows: pure, dependency-free parsers and
//! formatters used by the `vikunja_export_*` / `vikunja_import_tasks_*`
//! tools.
//!
//! Exports are deterministic: stable task ordering (id ascending unless the
//! caller requested a server-side sort), stable field order, documented CSV
//! escaping (RFC 4180: fields containing commas, quotes or newlines are
//! quoted, quotes are doubled) and `\n` line endings.
//!
//! Imports parse simple Markdown checklists and CSV backlogs into
//! [`import::ParsedTask`] rows; date resolution and Vikunja writes happen in
//! the tool layer. Attachments and comments are not part of either format in
//! this version.

pub mod export;
pub mod import;

use schemars::JsonSchema;
use serde::Deserialize;

/// Largest accepted import payload (bytes of the markdown/csv argument).
/// Bounds MCP message processing time and memory.
pub const MAX_IMPORT_BYTES: usize = 256 * 1024;
/// Largest number of backlog rows (tasks or errors) accepted by one import
/// call. Matches the bulk tool cap: each created task costs one Vikunja
/// request.
pub const MAX_IMPORT_TASKS: usize = 100;

/// Output format of the export tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Markdown,
    Csv,
}

impl ExportFormat {
    /// The lowercase name used in tool arguments and results.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::Csv => "csv",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_format_parses_lowercase_names_only() {
        for (name, expected) in [
            ("json", ExportFormat::Json),
            ("markdown", ExportFormat::Markdown),
            ("csv", ExportFormat::Csv),
        ] {
            let format: ExportFormat = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(format, expected);
            assert_eq!(format.as_str(), name);
        }
        for bad in ["JSON", "md", "tsv", ""] {
            assert!(
                serde_json::from_value::<ExportFormat>(serde_json::json!(bad)).is_err(),
                "{bad:?} must not parse"
            );
        }
    }
}
