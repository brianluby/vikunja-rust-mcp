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
use crate::vikunja::VikunjaClient;

use super::resources;

/// MCP server exposing Vikunja projects, tasks, labels, comments,
/// attachments, users and teams.
#[derive(Clone)]
pub struct VikunjaMcpServer {
    client: Arc<VikunjaClient>,
    dates: DateConfig,
    tool_router: ToolRouter<Self>,
}

impl VikunjaMcpServer {
    pub fn new(client: VikunjaClient) -> Self {
        Self {
            client: Arc::new(client),
            dates: DateConfig::default(),
            tool_router: Self::tool_router(),
        }
    }

    /// Overrides the times of day used by date shortcuts.
    pub fn with_date_config(mut self, dates: DateConfig) -> Self {
        self.dates = dates;
        self
    }

    pub fn client(&self) -> &VikunjaClient {
        &self.client
    }

    /// Times of day applied when date shortcuts resolve.
    pub fn dates(&self) -> &DateConfig {
        &self.dates
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
            "vikunja_dates_resolve."
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
