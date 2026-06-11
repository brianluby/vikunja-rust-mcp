//! The MCP server handler: server metadata, tool routing and resources.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool_handler};

use crate::dates::DateConfig;
use crate::sandbox::AttachmentSandbox;
use crate::vikunja::VikunjaClient;

use super::resources;

/// MCP server exposing Vikunja projects, tasks, labels, comments,
/// attachments, users and teams.
#[derive(Clone)]
pub struct VikunjaMcpServer {
    client: Arc<VikunjaClient>,
    dates: DateConfig,
    attachment_sandbox: Arc<AttachmentSandbox>,
    tool_router: ToolRouter<Self>,
}

impl VikunjaMcpServer {
    pub fn new(client: VikunjaClient) -> Self {
        Self {
            client: Arc::new(client),
            dates: DateConfig::default(),
            attachment_sandbox: Arc::new(AttachmentSandbox::default()),
            tool_router: Self::tool_router(),
        }
    }

    /// Overrides the times of day used by date shortcuts.
    pub fn with_date_config(mut self, dates: DateConfig) -> Self {
        self.dates = dates;
        self
    }

    /// Restricts the attachment tools' file reads/writes to the sandbox's
    /// configured root directories. The default sandbox is permissive.
    pub fn with_attachment_sandbox(mut self, sandbox: AttachmentSandbox) -> Self {
        self.attachment_sandbox = Arc::new(sandbox);
        self
    }

    pub fn client(&self) -> &VikunjaClient {
        &self.client
    }

    /// Times of day applied when date shortcuts resolve.
    pub fn dates(&self) -> &DateConfig {
        &self.dates
    }

    /// Path restrictions applied to attachment file operations.
    pub fn attachment_sandbox(&self) -> &AttachmentSandbox {
        &self.attachment_sandbox
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VikunjaMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(concat!(
            "Bridge to a Vikunja (to-do / project management) instance. ",
            "Tools cover projects, tasks, labels, assignees, comments, attachments, ",
            "user search, teams, saved filters and read-only Kanban buckets ",
            "(board lanes by name via vikunja_buckets_list). ",
            "Typical flow: find a project with vikunja_projects_list, list its tasks ",
            "with vikunja_tasks_list (project_id), then create/update/complete tasks. ",
            "List tools paginate: check the `pagination` object and request further ",
            "pages via `page` until `has_more` is false. ",
            "Task filters use Vikunja syntax, e.g. 'done = false && due_date < now/d+7d'. ",
            "All ids are numeric Vikunja ids. Colors are hex without '#'. ",
            "Dates are RFC 3339 timestamps like 2026-07-01T12:00:00Z; task ",
            "create/update also accept date shortcuts ('tomorrow', 'next friday', ",
            "'in 2 weeks', 'clear') via the *_shortcut fields — preview them with ",
            "vikunja_dates_resolve. ",
            "Backlogs move in and out via vikunja_export_tasks / vikunja_export_project ",
            "(JSON, Markdown or CSV, read-only and bounded) and ",
            "vikunja_import_tasks_markdown / vikunja_import_tasks_csv ",
            "(dry-run preview by default; set dry_run: false to create)."
        ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: resources::list(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: resources::templates(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        resources::read(self.client(), &request.uri).await
    }
}
