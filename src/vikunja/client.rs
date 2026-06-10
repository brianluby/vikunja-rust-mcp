//! Async client for the Vikunja REST API.
//!
//! Endpoint paths and payload shapes follow the upstream swagger spec
//! (Vikunja >= 1.0). Note Vikunja's conventions: `PUT` creates entities and
//! `POST` updates them.
//!
//! Updates are implemented as read-merge-write: Vikunja's update endpoints
//! reset fields that are omitted from the payload to their zero values, so
//! the client first fetches the current entity, overlays the requested
//! changes and sends the merged object back.

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{debug, warn};
use url::Url;

use crate::config::Config;
use crate::error::Error;

use super::models::{
    Label, LabelCreate, LabelTask, LabelUpdate, Message, Project, ProjectCreate, ProjectUpdate,
    Task, TaskAssignee, TaskComment, TaskCreate, TaskUpdate, Team, User,
};
use super::pagination::{Page, PageInfo, PageParams};

/// How long to wait before the single retry of an idempotent request.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// Options for listing tasks via `GET /tasks`.
#[derive(Debug, Clone, Default)]
pub struct TaskListOptions {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    /// Full-text search on the task title.
    pub search: Option<String>,
    /// Vikunja filter expression, e.g. `done = false && priority >= 3`.
    pub filter: Option<String>,
    /// Restrict results to one project (combined into the filter).
    pub project_id: Option<i64>,
    /// Field to sort by, e.g. `due_date`.
    pub sort_by: Option<String>,
    /// Sort direction: `asc` or `desc`.
    pub order_by: Option<String>,
}

impl TaskListOptions {
    /// Combines the explicit filter expression with the project restriction.
    fn combined_filter(&self) -> Option<String> {
        match (self.filter.as_deref(), self.project_id) {
            (Some(filter), Some(project_id)) => {
                Some(format!("({filter}) && project_id = {project_id}"))
            }
            (Some(filter), None) => Some(filter.to_string()),
            (None, Some(project_id)) => Some(format!("project_id = {project_id}")),
            (None, None) => None,
        }
    }
}

/// Result of a bounded multi-page task listing: the concatenated tasks plus
/// metadata about how far pagination got.
#[derive(Debug, Clone)]
pub struct BoundedTasks {
    pub tasks: Vec<Task>,
    /// Number of pages actually fetched (at least 1).
    pub pages_read: u32,
    /// True when the page cap was hit while the server still reported more
    /// pages (`has_more == Some(true)` on the last fetched page).
    pub truncated: bool,
}

/// A downloaded attachment body.
#[derive(Debug, Clone)]
pub struct AttachmentContent {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

fn extract_content_type(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

/// Async client for one Vikunja instance.
#[derive(Debug, Clone)]
pub struct VikunjaClient {
    http: reqwest::Client,
    base_url: Url,
    default_page_size: u32,
}

impl VikunjaClient {
    /// Builds a client from validated configuration. The API token is stored
    /// only inside the HTTP client's default headers and is marked sensitive
    /// so it is redacted from any debug output.
    pub fn new(config: &Config) -> Result<Self, Error> {
        let mut auth_value =
            HeaderValue::from_str(&format!("Bearer {}", config.api_token.reveal())).map_err(
                |_| {
                    Error::InvalidArgument(
                    "VIKUNJA_API_TOKEN contains characters that are not valid in an HTTP header"
                        .to_string(),
                )
                },
            )?;
        auth_value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth_value);

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(config.timeout)
            .user_agent(concat!("vikunja-rust-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Network {
                endpoint: "client.init",
                detail: format!("failed to build HTTP client: {e}"),
            })?;

        Ok(Self {
            http,
            base_url: config.vikunja_url.clone(),
            default_page_size: config.default_page_size,
        })
    }

    /// Base URL of the Vikunja instance (without `/api/v1`).
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Default page size applied when a request does not specify one.
    pub fn default_page_size(&self) -> u32 {
        self.default_page_size
    }

    /// Builds an absolute URL for an `/api/v1` path. `path` must start
    /// with `/` and is appended verbatim, so callers must only pass
    /// well-formed paths built from validated ids.
    fn api_url(&self, path: &str) -> Url {
        let mut url = self.base_url.clone();
        let base_path = self.base_url.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/api/v1{path}"));
        url
    }

    /// Sends a request, retrying once for idempotent requests that fail with
    /// a timeout or connection error, and maps non-2xx responses to errors.
    async fn execute(
        &self,
        endpoint: &'static str,
        builder: RequestBuilder,
        idempotent: bool,
    ) -> Result<reqwest::Response, Error> {
        let retry = if idempotent {
            builder.try_clone()
        } else {
            None
        };

        let first = builder.send().await;
        let response = match first {
            Ok(response) => response,
            Err(err) => {
                let mapped = Error::from_reqwest(endpoint, err);
                let retriable = matches!(mapped, Error::Timeout { .. } | Error::Network { .. });
                match (retry, retriable) {
                    (Some(retry_builder), true) => {
                        warn!(endpoint, error = %mapped, "retrying idempotent request once");
                        tokio::time::sleep(RETRY_DELAY).await;
                        retry_builder
                            .send()
                            .await
                            .map_err(|e| Error::from_reqwest(endpoint, e))?
                    }
                    _ => return Err(mapped),
                }
            }
        };

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().await.unwrap_or_default();
        let error = Error::from_status(endpoint, status.as_u16(), &body);
        warn!(endpoint, status = status.as_u16(), "Vikunja API error");
        Err(error)
    }

    /// Reads and decodes a JSON response body.
    async fn decode<T: DeserializeOwned>(
        endpoint: &'static str,
        response: reqwest::Response,
    ) -> Result<T, Error> {
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::from_reqwest(endpoint, e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::InvalidResponse {
            endpoint,
            detail: e.to_string(),
        })
    }

    /// GET a JSON document.
    async fn get_json<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, Error> {
        debug!(endpoint, path, "GET");
        let builder = self.http.get(self.api_url(path)).query(query);
        let response = self.execute(endpoint, builder, true).await?;
        Self::decode(endpoint, response).await
    }

    /// GET one page of a list endpoint, capturing pagination headers.
    /// Vikunja (Go) serializes empty lists as `null`, which is mapped to an
    /// empty vector.
    async fn get_page<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        path: &str,
        extra_query: &[(&str, String)],
        params: PageParams,
    ) -> Result<Page<T>, Error> {
        let params = PageParams {
            page: params.page,
            per_page: params.per_page.or(Some(self.default_page_size)),
        };
        let mut query = params.to_query();
        query.extend(extra_query.iter().map(|(k, v)| (*k, v.clone())));

        debug!(endpoint, path, page = ?params.page, "GET (paged)");
        let builder = self.http.get(self.api_url(path)).query(&query);
        let response = self.execute(endpoint, builder, true).await?;
        let info = PageInfo::from_headers(params, response.headers());
        let items: Option<Vec<T>> = Self::decode(endpoint, response).await?;
        Ok(Page {
            items: items.unwrap_or_default(),
            info,
        })
    }

    /// Sends a JSON body with the given method and decodes a JSON response.
    async fn send_json<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        method: Method,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        debug!(endpoint, %method, path, "request with JSON body");
        let builder = self
            .http
            .request(method.clone(), self.api_url(path))
            .json(body);
        let idempotent = method == Method::GET;
        let response = self.execute(endpoint, builder, idempotent).await?;
        Self::decode(endpoint, response).await
    }

    /// Sends a body-less request (DELETE and friends) and decodes JSON.
    async fn send_empty<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        method: Method,
        path: &str,
    ) -> Result<T, Error> {
        debug!(endpoint, %method, path, "request");
        let builder = self.http.request(method, self.api_url(path));
        let response = self.execute(endpoint, builder, false).await?;
        Self::decode(endpoint, response).await
    }

    /// Fetches the current entity as raw JSON, overlays `patch` and writes
    /// the merged object back. This preserves fields the caller did not
    /// touch, because Vikunja's update endpoints zero omitted fields.
    async fn read_merge_write<T: DeserializeOwned>(
        &self,
        get_endpoint: &'static str,
        write_endpoint: &'static str,
        path: &str,
        write_method: Method,
        patch: &impl Serialize,
    ) -> Result<T, Error> {
        let patch_value = serde_json::to_value(patch).map_err(|e| Error::InvalidResponse {
            endpoint: write_endpoint,
            detail: format!("failed to serialize update payload: {e}"),
        })?;
        let Value::Object(patch_map) = patch_value else {
            return Err(Error::InvalidArgument(
                "update payload must be a JSON object".to_string(),
            ));
        };
        if patch_map.is_empty() {
            return Err(Error::InvalidArgument(
                "nothing to update: provide at least one field".to_string(),
            ));
        }

        let mut current: Value = self.get_json(get_endpoint, path, &[]).await?;
        let Some(target) = current.as_object_mut() else {
            return Err(Error::InvalidResponse {
                endpoint: get_endpoint,
                detail: "expected a JSON object".to_string(),
            });
        };
        for (key, value) in patch_map {
            target.insert(key, value);
        }

        self.send_json(write_endpoint, write_method, path, &current)
            .await
    }

    // ----- Projects ---------------------------------------------------------

    /// `GET /projects` — list or search projects the user has access to.
    pub async fn list_projects(
        &self,
        params: PageParams,
        search: Option<&str>,
        is_archived: Option<bool>,
    ) -> Result<Page<Project>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        if let Some(is_archived) = is_archived {
            query.push(("is_archived", is_archived.to_string()));
        }
        self.get_page("projects.list", "/projects", &query, params)
            .await
    }

    /// `GET /projects/{id}`.
    pub async fn get_project(&self, project_id: i64) -> Result<Project, Error> {
        self.get_json("projects.get", &format!("/projects/{project_id}"), &[])
            .await
    }

    /// `PUT /projects` — create a project.
    pub async fn create_project(&self, body: &ProjectCreate) -> Result<Project, Error> {
        self.send_json("projects.create", Method::PUT, "/projects", body)
            .await
    }

    /// `POST /projects/{id}` — partial update via read-merge-write.
    pub async fn update_project(
        &self,
        project_id: i64,
        patch: &ProjectUpdate,
    ) -> Result<Project, Error> {
        self.read_merge_write(
            "projects.get",
            "projects.update",
            &format!("/projects/{project_id}"),
            Method::POST,
            patch,
        )
        .await
    }

    /// `DELETE /projects/{id}`.
    pub async fn delete_project(&self, project_id: i64) -> Result<Message, Error> {
        self.send_empty(
            "projects.delete",
            Method::DELETE,
            &format!("/projects/{project_id}"),
        )
        .await
    }

    // ----- Tasks ------------------------------------------------------------

    /// `GET /tasks` — list or search tasks across all projects, with
    /// optional Vikunja filter expression and project restriction.
    pub async fn list_tasks(&self, options: &TaskListOptions) -> Result<Page<Task>, Error> {
        let mut query = Vec::new();
        if let Some(search) = options.search.as_deref() {
            query.push(("s", search.to_string()));
        }
        if let Some(filter) = options.combined_filter() {
            query.push(("filter", filter));
        }
        if let Some(sort_by) = options.sort_by.as_deref() {
            query.push(("sort_by", sort_by.to_string()));
        }
        if let Some(order_by) = options.order_by.as_deref() {
            query.push(("order_by", order_by.to_string()));
        }
        self.get_page(
            "tasks.list",
            "/tasks",
            &query,
            PageParams::new(options.page, options.per_page),
        )
        .await
    }

    /// `GET /tasks/{id}`.
    pub async fn get_task(&self, task_id: i64) -> Result<Task, Error> {
        self.get_json("tasks.get", &format!("/tasks/{task_id}"), &[])
            .await
    }

    /// `PUT /projects/{id}/tasks` — create a task in a project.
    pub async fn create_task(&self, project_id: i64, body: &TaskCreate) -> Result<Task, Error> {
        self.send_json(
            "tasks.create",
            Method::PUT,
            &format!("/projects/{project_id}/tasks"),
            body,
        )
        .await
    }

    /// `POST /tasks/{id}` — partial update via read-merge-write.
    pub async fn update_task(&self, task_id: i64, patch: &TaskUpdate) -> Result<Task, Error> {
        self.read_merge_write(
            "tasks.get",
            "tasks.update",
            &format!("/tasks/{task_id}"),
            Method::POST,
            patch,
        )
        .await
    }

    /// Marks a task done or not done (`POST /tasks/{id}` with `done` set).
    pub async fn set_task_done(&self, task_id: i64, done: bool) -> Result<Task, Error> {
        self.update_task(
            task_id,
            &TaskUpdate {
                done: Some(done),
                ..Default::default()
            },
        )
        .await
    }

    /// `DELETE /tasks/{id}`.
    pub async fn delete_task(&self, task_id: i64) -> Result<Message, Error> {
        self.send_empty("tasks.delete", Method::DELETE, &format!("/tasks/{task_id}"))
            .await
    }

    // ----- Labels -----------------------------------------------------------

    /// `GET /labels` — list or search the user's labels.
    pub async fn list_labels(
        &self,
        params: PageParams,
        search: Option<&str>,
    ) -> Result<Page<Label>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        self.get_page("labels.list", "/labels", &query, params)
            .await
    }

    /// `GET /labels/{id}`.
    pub async fn get_label(&self, label_id: i64) -> Result<Label, Error> {
        self.get_json("labels.get", &format!("/labels/{label_id}"), &[])
            .await
    }

    /// `PUT /labels` — create a label.
    pub async fn create_label(&self, body: &LabelCreate) -> Result<Label, Error> {
        self.send_json("labels.create", Method::PUT, "/labels", body)
            .await
    }

    /// `PUT /labels/{id}` — partial update via read-merge-write.
    pub async fn update_label(&self, label_id: i64, patch: &LabelUpdate) -> Result<Label, Error> {
        self.read_merge_write(
            "labels.get",
            "labels.update",
            &format!("/labels/{label_id}"),
            Method::PUT,
            patch,
        )
        .await
    }

    /// `DELETE /labels/{id}`. Unlike the other delete endpoints, the API
    /// returns the deleted label itself rather than a `models.Message`.
    pub async fn delete_label(&self, label_id: i64) -> Result<Label, Error> {
        self.send_empty(
            "labels.delete",
            Method::DELETE,
            &format!("/labels/{label_id}"),
        )
        .await
    }

    // ----- Task labels ------------------------------------------------------

    /// `GET /tasks/{task}/labels`.
    pub async fn list_task_labels(
        &self,
        task_id: i64,
        params: PageParams,
        search: Option<&str>,
    ) -> Result<Page<Label>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        self.get_page(
            "task_labels.list",
            &format!("/tasks/{task_id}/labels"),
            &query,
            params,
        )
        .await
    }

    /// `PUT /tasks/{task}/labels` — add a label to a task.
    pub async fn add_task_label(&self, task_id: i64, label_id: i64) -> Result<LabelTask, Error> {
        self.send_json(
            "task_labels.add",
            Method::PUT,
            &format!("/tasks/{task_id}/labels"),
            &serde_json::json!({ "label_id": label_id }),
        )
        .await
    }

    /// `DELETE /tasks/{task}/labels/{label}` — remove a label from a task.
    pub async fn remove_task_label(&self, task_id: i64, label_id: i64) -> Result<Message, Error> {
        self.send_empty(
            "task_labels.remove",
            Method::DELETE,
            &format!("/tasks/{task_id}/labels/{label_id}"),
        )
        .await
    }

    // ----- Assignees --------------------------------------------------------

    /// `GET /tasks/{taskID}/assignees`.
    pub async fn list_task_assignees(
        &self,
        task_id: i64,
        params: PageParams,
    ) -> Result<Page<User>, Error> {
        self.get_page(
            "assignees.list",
            &format!("/tasks/{task_id}/assignees"),
            &[],
            params,
        )
        .await
    }

    /// `PUT /tasks/{taskID}/assignees` — assign a user to a task.
    pub async fn assign_user(&self, task_id: i64, user_id: i64) -> Result<TaskAssignee, Error> {
        self.send_json(
            "assignees.add",
            Method::PUT,
            &format!("/tasks/{task_id}/assignees"),
            &serde_json::json!({ "user_id": user_id }),
        )
        .await
    }

    /// `DELETE /tasks/{taskID}/assignees/{userID}`.
    pub async fn unassign_user(&self, task_id: i64, user_id: i64) -> Result<Message, Error> {
        self.send_empty(
            "assignees.remove",
            Method::DELETE,
            &format!("/tasks/{task_id}/assignees/{user_id}"),
        )
        .await
    }

    // ----- Comments ---------------------------------------------------------

    /// `GET /tasks/{taskID}/comments` (not paginated by the API).
    pub async fn list_task_comments(&self, task_id: i64) -> Result<Vec<TaskComment>, Error> {
        let comments: Option<Vec<TaskComment>> = self
            .get_json("comments.list", &format!("/tasks/{task_id}/comments"), &[])
            .await?;
        Ok(comments.unwrap_or_default())
    }

    /// `PUT /tasks/{taskID}/comments` — add a comment.
    pub async fn create_task_comment(
        &self,
        task_id: i64,
        comment: &str,
    ) -> Result<TaskComment, Error> {
        self.send_json(
            "comments.create",
            Method::PUT,
            &format!("/tasks/{task_id}/comments"),
            &serde_json::json!({ "comment": comment }),
        )
        .await
    }

    /// `POST /tasks/{taskID}/comments/{commentID}` — edit a comment.
    pub async fn update_task_comment(
        &self,
        task_id: i64,
        comment_id: i64,
        comment: &str,
    ) -> Result<TaskComment, Error> {
        self.send_json(
            "comments.update",
            Method::POST,
            &format!("/tasks/{task_id}/comments/{comment_id}"),
            &serde_json::json!({ "comment": comment }),
        )
        .await
    }

    /// `DELETE /tasks/{taskID}/comments/{commentID}`.
    pub async fn delete_task_comment(
        &self,
        task_id: i64,
        comment_id: i64,
    ) -> Result<Message, Error> {
        self.send_empty(
            "comments.delete",
            Method::DELETE,
            &format!("/tasks/{task_id}/comments/{comment_id}"),
        )
        .await
    }

    // ----- Attachments ------------------------------------------------------

    /// `GET /tasks/{id}/attachments`.
    pub async fn list_task_attachments(
        &self,
        task_id: i64,
        params: PageParams,
    ) -> Result<Page<super::models::TaskAttachment>, Error> {
        self.get_page(
            "attachments.list",
            &format!("/tasks/{task_id}/attachments"),
            &[],
            params,
        )
        .await
    }

    /// `PUT /tasks/{id}/attachments` — upload one file as multipart
    /// form-data (field name `files`).
    pub async fn upload_attachment(
        &self,
        task_id: i64,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Message, Error> {
        let endpoint = "attachments.upload";
        debug!(endpoint, task_id, file_name, size = bytes.len(), "upload");
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string());
        let form = reqwest::multipart::Form::new().part("files", part);
        let builder = self
            .http
            .put(self.api_url(&format!("/tasks/{task_id}/attachments")))
            .multipart(form);
        let response = self.execute(endpoint, builder, false).await?;
        Self::decode(endpoint, response).await
    }

    /// `GET /tasks/{id}/attachments/{attachmentID}` — download the file
    /// into memory, failing with [`Error::TooLarge`] before buffering more
    /// than `max_bytes` (via `Content-Length` when the server sends it, and
    /// a hard cap while streaming otherwise).
    pub async fn download_attachment(
        &self,
        task_id: i64,
        attachment_id: i64,
        max_bytes: u64,
    ) -> Result<AttachmentContent, Error> {
        let endpoint = "attachments.download";
        debug!(endpoint, task_id, attachment_id, max_bytes, "download");
        let builder = self
            .http
            .get(self.api_url(&format!("/tasks/{task_id}/attachments/{attachment_id}")));
        let mut response = self.execute(endpoint, builder, true).await?;
        let content_type = extract_content_type(&response);
        if let Some(length) = response.content_length()
            && length > max_bytes
        {
            return Err(Error::TooLarge {
                endpoint,
                size: Some(length),
                limit: max_bytes,
            });
        }
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Error::from_reqwest(endpoint, e))?
        {
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(Error::TooLarge {
                    endpoint,
                    size: None,
                    limit: max_bytes,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(AttachmentContent {
            bytes,
            content_type,
        })
    }

    /// `GET /tasks/{id}/attachments/{attachmentID}` — stream the file to
    /// `path` chunk by chunk, without buffering it in memory. Returns the
    /// number of bytes written and the reported content type.
    pub async fn download_attachment_to_file(
        &self,
        task_id: i64,
        attachment_id: i64,
        path: &str,
    ) -> Result<(u64, Option<String>), Error> {
        use tokio::io::AsyncWriteExt;

        let endpoint = "attachments.download";
        debug!(endpoint, task_id, attachment_id, path, "download to file");
        let builder = self
            .http
            .get(self.api_url(&format!("/tasks/{task_id}/attachments/{attachment_id}")));
        let mut response = self.execute(endpoint, builder, true).await?;
        let content_type = extract_content_type(&response);
        let mut file = tokio::fs::File::create(path).await.map_err(|e| Error::Io {
            detail: format!("could not create {path}: {e}"),
        })?;
        let mut written: u64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Error::from_reqwest(endpoint, e))?
        {
            file.write_all(&chunk).await.map_err(|e| Error::Io {
                detail: format!("could not write {path}: {e}"),
            })?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(|e| Error::Io {
            detail: format!("could not write {path}: {e}"),
        })?;
        Ok((written, content_type))
    }

    /// `DELETE /tasks/{id}/attachments/{attachmentID}`.
    pub async fn delete_attachment(
        &self,
        task_id: i64,
        attachment_id: i64,
    ) -> Result<Message, Error> {
        self.send_empty(
            "attachments.delete",
            Method::DELETE,
            &format!("/tasks/{task_id}/attachments/{attachment_id}"),
        )
        .await
    }

    // ----- Users ------------------------------------------------------------

    /// `GET /users?s=` — search users for assignment. The API requires a
    /// search term unless the instance allows listing all users.
    pub async fn search_users(&self, search: Option<&str>) -> Result<Vec<User>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        let users: Option<Vec<User>> = self.get_json("users.search", "/users", &query).await?;
        Ok(users.unwrap_or_default())
    }

    // ----- Teams ------------------------------------------------------------

    /// `GET /teams` — list teams the user is part of.
    pub async fn list_teams(
        &self,
        params: PageParams,
        search: Option<&str>,
    ) -> Result<Page<Team>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        self.get_page("teams.list", "/teams", &query, params).await
    }

    /// `GET /projects/{id}/teams` — teams with access to a project,
    /// including their `permission` level.
    pub async fn list_project_teams(
        &self,
        project_id: i64,
        params: PageParams,
        search: Option<&str>,
    ) -> Result<Page<Team>, Error> {
        let mut query = Vec::new();
        if let Some(search) = search {
            query.push(("s", search.to_string()));
        }
        self.get_page(
            "teams.list_for_project",
            &format!("/projects/{project_id}/teams"),
            &query,
            params,
        )
        .await
    }

    // ----- Bulk helpers (used by MCP resources) ------------------------------

    /// Fetches up to `max_pages` pages of projects and concatenates them.
    pub async fn list_all_projects(&self, max_pages: u32) -> Result<Vec<Project>, Error> {
        let mut all = Vec::new();
        let mut page = 1u32;
        loop {
            let result = self
                .list_projects(PageParams::new(Some(page), None), None, None)
                .await?;
            all.extend(result.items);
            if result.info.has_more != Some(true) || page >= max_pages {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    /// Fetches up to `max_pages` pages of tasks and concatenates them,
    /// discarding the pagination metadata.
    pub async fn list_all_tasks(&self, max_pages: u32) -> Result<Vec<Task>, Error> {
        let result = self
            .list_all_tasks_with_options(&TaskListOptions::default(), max_pages)
            .await?;
        Ok(result.tasks)
    }

    /// Fetches up to `max_pages` pages of tasks matching `options`,
    /// concatenating them and reporting how far pagination got. Any `page`
    /// set on `options` is overridden while walking the pages. At least one
    /// page is always fetched, so `pages_read` is at least 1 even when
    /// `max_pages` is 0.
    pub async fn list_all_tasks_with_options(
        &self,
        options: &TaskListOptions,
        max_pages: u32,
    ) -> Result<BoundedTasks, Error> {
        let mut tasks = Vec::new();
        let mut page = 1u32;
        loop {
            let result = self
                .list_tasks(&TaskListOptions {
                    page: Some(page),
                    ..options.clone()
                })
                .await?;
            tasks.extend(result.items);
            let has_more = result.info.has_more == Some(true);
            if !has_more || page >= max_pages {
                return Ok(BoundedTasks {
                    tasks,
                    pages_read: page,
                    truncated: has_more,
                });
            }
            page += 1;
        }
    }

    /// Lightweight connectivity probe used by the status resource: requests
    /// the first project page and reports HTTP-level success.
    pub async fn probe(&self) -> Result<StatusCode, Error> {
        let builder = self
            .http
            .get(self.api_url("/projects"))
            .query(&[("per_page", "1")]);
        let response = self.execute("status.probe", builder, true).await?;
        Ok(response.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client(base: &str) -> VikunjaClient {
        let cli = <crate::config::Cli as clap::Parser>::try_parse_from([
            "vikunja-rust-mcp",
            "--vikunja-url",
            base,
            "--api-token",
            "test-token",
        ])
        .unwrap();
        let config = Config::from_cli(&cli).unwrap();
        VikunjaClient::new(&config).unwrap()
    }

    #[test]
    fn api_url_appends_api_v1() {
        let client = test_client("https://vikunja.example.com");
        assert_eq!(
            client.api_url("/tasks/7").as_str(),
            "https://vikunja.example.com/api/v1/tasks/7"
        );
    }

    #[test]
    fn api_url_preserves_subpath_installations() {
        let client = test_client("https://example.com/vikunja");
        assert_eq!(
            client.api_url("/projects").as_str(),
            "https://example.com/vikunja/api/v1/projects"
        );
    }

    #[test]
    fn combined_filter_merges_project_and_filter() {
        let options = TaskListOptions {
            filter: Some("done = false".into()),
            project_id: Some(5),
            ..Default::default()
        };
        assert_eq!(
            options.combined_filter().unwrap(),
            "(done = false) && project_id = 5"
        );

        let only_project = TaskListOptions {
            project_id: Some(5),
            ..Default::default()
        };
        assert_eq!(only_project.combined_filter().unwrap(), "project_id = 5");

        let only_filter = TaskListOptions {
            filter: Some("priority >= 3".into()),
            ..Default::default()
        };
        assert_eq!(only_filter.combined_filter().unwrap(), "priority >= 3");

        assert_eq!(TaskListOptions::default().combined_filter(), None);
    }

    #[test]
    fn client_with_invalid_token_chars_fails() {
        let cli = <crate::config::Cli as clap::Parser>::try_parse_from([
            "vikunja-rust-mcp",
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "bad\ntoken",
        ])
        .unwrap();
        // \n is stripped by trim, so build a config with an inner newline.
        let mut config = Config::from_cli(&cli).unwrap();
        config.api_token = crate::config::ApiToken::new("bad\u{0}token");
        assert!(matches!(
            VikunjaClient::new(&config),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn client_debug_does_not_leak_token() {
        let client = test_client("https://example.com");
        let debug = format!("{client:?}");
        assert!(!debug.contains("test-token"));
    }
}
