//! Error types shared by the Vikunja client and the MCP tool layer.
//!
//! [`Error`] captures what failed (endpoint category), why (HTTP status and
//! the Vikunja error code/message when available) and how to map it onto an
//! MCP error. Authorization headers and tokens are never included.

use rmcp::ErrorData as McpError;
use serde_json::json;

/// Failure category derived from the HTTP status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorKind {
    /// 401: the API token is missing, expired or invalid.
    Auth,
    /// 403: the token lacks permission or scope for this operation.
    Forbidden,
    /// 404: the requested entity does not exist.
    NotFound,
    /// 400/412/422: Vikunja rejected the request payload.
    Validation,
    /// 409: the request conflicts with existing state, e.g. a task relation
    /// that already exists.
    Conflict,
    /// 429: too many requests.
    RateLimited,
    /// 5xx: Vikunja-side failure.
    Server,
    /// Anything else.
    Other,
}

impl ApiErrorKind {
    pub fn from_status(status: u16) -> Self {
        match status {
            401 => Self::Auth,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            400 | 412 | 422 => Self::Validation,
            409 => Self::Conflict,
            429 => Self::RateLimited,
            500..=599 => Self::Server,
            _ => Self::Other,
        }
    }
}

/// Errors returned by the Vikunja API client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Vikunja answered with a non-success HTTP status.
    #[error("Vikunja API error in {endpoint}: HTTP {status} ({kind:?}): {message}")]
    Api {
        /// Endpoint category, e.g. `tasks.update`.
        endpoint: &'static str,
        status: u16,
        kind: ApiErrorKind,
        /// Vikunja-specific error code from the response body, if present.
        code: Option<i64>,
        message: String,
    },

    /// The request never produced an HTTP response (DNS, TLS, connect, ...).
    #[error("network error calling Vikunja ({endpoint}): {detail}")]
    Network {
        endpoint: &'static str,
        detail: String,
    },

    /// The request exceeded the configured timeout.
    #[error("timed out calling Vikunja ({endpoint})")]
    Timeout { endpoint: &'static str },

    /// Vikunja answered 2xx but the body could not be decoded.
    #[error("invalid response from Vikunja ({endpoint}): {detail}")]
    InvalidResponse {
        endpoint: &'static str,
        detail: String,
    },

    /// Local file I/O failed while handling an attachment.
    #[error("attachment file error: {detail}")]
    Io { detail: String },

    /// A response body exceeded the caller-imposed size limit and was not
    /// buffered.
    #[error("response from Vikunja ({endpoint}) exceeds the {limit}-byte limit")]
    TooLarge {
        endpoint: &'static str,
        /// Size announced via `Content-Length`, when the server sent it.
        size: Option<u64>,
        limit: u64,
    },

    /// A tool argument was rejected before any request was made.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

impl Error {
    /// Builds an [`Error`] from a transport-level `reqwest` failure.
    /// The resulting message contains no headers and therefore no token.
    pub fn from_reqwest(endpoint: &'static str, err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return Self::Timeout { endpoint };
        }
        let detail = if err.is_connect() {
            "connection failed (is VIKUNJA_URL reachable?)".to_string()
        } else if err.is_request() {
            "request could not be sent".to_string()
        } else if err.is_body() || err.is_decode() {
            "response body could not be read".to_string()
        } else {
            "transport error".to_string()
        };
        Self::Network { endpoint, detail }
    }

    /// Builds an API error from a status code and the (already read)
    /// response body, decoding Vikunja's `{"code": .., "message": ..}` shape
    /// when possible.
    pub fn from_status(endpoint: &'static str, status: u16, body: &[u8]) -> Self {
        let (code, message) = match serde_json::from_slice::<VikunjaErrorBody>(body) {
            Ok(parsed) => (parsed.code, parsed.message),
            Err(_) => {
                let text = String::from_utf8_lossy(body);
                let trimmed = text.trim();
                let message = if trimmed.is_empty() {
                    default_status_message(status).to_string()
                } else {
                    truncate(trimmed, 300)
                };
                (None, message)
            }
        };
        Self::Api {
            endpoint,
            status,
            kind: ApiErrorKind::from_status(status),
            code,
            message,
        }
    }

    /// Converts this error into an MCP error with safe, structured details.
    pub fn to_mcp(&self) -> McpError {
        match self {
            Self::Api {
                endpoint,
                status,
                kind,
                code,
                message,
            } => {
                let data = Some(json!({
                    "endpoint": endpoint,
                    "http_status": status,
                    "kind": kind,
                    "vikunja_error_code": code,
                }));
                let text = match kind {
                    ApiErrorKind::Auth => format!(
                        "Vikunja rejected the API token (HTTP 401) while calling {endpoint}: {message}. Check VIKUNJA_API_TOKEN."
                    ),
                    ApiErrorKind::Forbidden => format!(
                        "Vikunja denied access (HTTP 403) while calling {endpoint}: {message}. The API token may be missing a scope for this operation."
                    ),
                    ApiErrorKind::NotFound => {
                        format!("Not found (HTTP 404) while calling {endpoint}: {message}")
                    }
                    ApiErrorKind::Validation => {
                        format!(
                            "Vikunja rejected the request (HTTP {status}) in {endpoint}: {message}"
                        )
                    }
                    ApiErrorKind::Conflict => {
                        format!("Vikunja reported a conflict (HTTP 409) in {endpoint}: {message}")
                    }
                    ApiErrorKind::RateLimited => format!(
                        "Vikunja rate limit hit (HTTP 429) while calling {endpoint}: {message}. Retry later."
                    ),
                    ApiErrorKind::Server | ApiErrorKind::Other => format!(
                        "Vikunja returned HTTP {status} while calling {endpoint}: {message}"
                    ),
                };
                match kind {
                    ApiErrorKind::NotFound | ApiErrorKind::Validation | ApiErrorKind::Conflict => {
                        McpError::invalid_params(text, data)
                    }
                    _ => McpError::internal_error(text, data),
                }
            }
            Self::InvalidArgument(message) => McpError::invalid_params(message.clone(), None),
            Self::Network { endpoint, .. } | Self::Timeout { endpoint } => {
                McpError::internal_error(
                    self.to_string(),
                    Some(json!({ "endpoint": endpoint, "kind": "network" })),
                )
            }
            Self::InvalidResponse { endpoint, .. } => McpError::internal_error(
                self.to_string(),
                Some(json!({ "endpoint": endpoint, "kind": "invalid_response" })),
            ),
            Self::TooLarge {
                endpoint,
                size,
                limit,
            } => McpError::invalid_params(
                self.to_string(),
                Some(json!({
                    "endpoint": endpoint,
                    "kind": "too_large",
                    "size": size,
                    "limit": limit,
                })),
            ),
            Self::Io { .. } => McpError::internal_error(self.to_string(), None),
        }
    }
}

impl From<Error> for McpError {
    fn from(err: Error) -> Self {
        err.to_mcp()
    }
}

#[derive(serde::Deserialize)]
struct VikunjaErrorBody {
    code: Option<i64>,
    message: String,
}

fn default_status_message(status: u16) -> &'static str {
    match status {
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not found",
        429 => "too many requests",
        500..=599 => "server error",
        _ => "request failed",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_derived_from_status() {
        assert_eq!(ApiErrorKind::from_status(401), ApiErrorKind::Auth);
        assert_eq!(ApiErrorKind::from_status(403), ApiErrorKind::Forbidden);
        assert_eq!(ApiErrorKind::from_status(404), ApiErrorKind::NotFound);
        assert_eq!(ApiErrorKind::from_status(400), ApiErrorKind::Validation);
        assert_eq!(ApiErrorKind::from_status(412), ApiErrorKind::Validation);
        assert_eq!(ApiErrorKind::from_status(422), ApiErrorKind::Validation);
        assert_eq!(ApiErrorKind::from_status(409), ApiErrorKind::Conflict);
        assert_eq!(ApiErrorKind::from_status(429), ApiErrorKind::RateLimited);
        assert_eq!(ApiErrorKind::from_status(500), ApiErrorKind::Server);
        assert_eq!(ApiErrorKind::from_status(503), ApiErrorKind::Server);
        assert_eq!(ApiErrorKind::from_status(418), ApiErrorKind::Other);
    }

    #[test]
    fn vikunja_error_body_is_decoded() {
        let err = Error::from_status(
            "tasks.get",
            404,
            br#"{"code":4002,"message":"The task does not exist."}"#,
        );
        match &err {
            Error::Api {
                endpoint,
                status,
                kind,
                code,
                message,
            } => {
                assert_eq!(*endpoint, "tasks.get");
                assert_eq!(*status, 404);
                assert_eq!(*kind, ApiErrorKind::NotFound);
                assert_eq!(*code, Some(4002));
                assert_eq!(message, "The task does not exist.");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn non_json_body_falls_back_to_text() {
        let err = Error::from_status("projects.list", 502, b"Bad Gateway");
        match &err {
            Error::Api { message, kind, .. } => {
                assert_eq!(message, "Bad Gateway");
                assert_eq!(*kind, ApiErrorKind::Server);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn empty_body_uses_default_message() {
        let err = Error::from_status("projects.list", 401, b"");
        match &err {
            Error::Api { message, .. } => assert_eq!(message, "unauthorized"),
            other => panic!("unexpected error: {other:?}"),
        }
        let err = Error::from_status("projects.list", 429, b"");
        match &err {
            Error::Api { message, .. } => assert_eq!(message, "too many requests"),
            other => panic!("unexpected error: {other:?}"),
        }
        let err = Error::from_status("projects.list", 404, b"");
        match &err {
            Error::Api { message, .. } => assert_eq!(message, "not found"),
            other => panic!("unexpected error: {other:?}"),
        }
        let err = Error::from_status("projects.list", 403, b"");
        match &err {
            Error::Api { message, .. } => assert_eq!(message, "forbidden"),
            other => panic!("unexpected error: {other:?}"),
        }
        let err = Error::from_status("projects.list", 418, b"");
        match &err {
            Error::Api { message, .. } => assert_eq!(message, "request failed"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn long_bodies_are_truncated() {
        let body = "x".repeat(1000);
        let err = Error::from_status("tasks.list", 500, body.as_bytes());
        match &err {
            Error::Api { message, .. } => {
                assert!(message.len() <= 303);
                assert!(message.ends_with("..."));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "aaaa\u{1F600}bbbb";
        let out = truncate(s, 5);
        assert!(out.starts_with("aaaa"));
        assert!(out.ends_with("..."));
    }

    #[test]
    fn not_found_and_validation_map_to_invalid_params() {
        for status in [404u16, 400, 412, 422] {
            let mcp = Error::from_status("tasks.get", status, b"{}").to_mcp();
            assert_eq!(
                mcp.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "status {status}"
            );
        }
    }

    #[test]
    fn auth_and_server_errors_map_to_internal_error() {
        for status in [401u16, 403, 429, 500, 418] {
            let mcp = Error::from_status("tasks.get", status, b"{}").to_mcp();
            assert_eq!(
                mcp.code,
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "status {status}"
            );
        }
    }

    #[test]
    fn mcp_error_data_carries_safe_details() {
        let err = Error::from_status(
            "labels.create",
            400,
            br#"{"code":3001,"message":"bad label"}"#,
        );
        let mcp = err.to_mcp();
        let data = mcp.data.expect("data should be set");
        assert_eq!(data["endpoint"], "labels.create");
        assert_eq!(data["http_status"], 400);
        assert_eq!(data["vikunja_error_code"], 3001);
        assert!(mcp.message.contains("bad label"));
    }

    #[test]
    fn auth_error_message_mentions_token_env_var() {
        let mcp = Error::from_status("projects.list", 401, b"").to_mcp();
        assert!(mcp.message.contains("VIKUNJA_API_TOKEN"));
        let mcp = Error::from_status("projects.list", 403, b"").to_mcp();
        assert!(mcp.message.contains("scope"));
    }

    #[test]
    fn invalid_argument_maps_to_invalid_params() {
        let mcp = Error::InvalidArgument("page must be >= 1".into()).to_mcp();
        assert_eq!(mcp.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(mcp.message.contains("page must be >= 1"));
    }

    #[test]
    fn too_large_maps_to_invalid_params_with_details() {
        let mcp = Error::TooLarge {
            endpoint: "attachments.download",
            size: Some(5_000_000),
            limit: 2_097_152,
        }
        .to_mcp();
        assert_eq!(mcp.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(mcp.message.contains("2097152"));
        let data = mcp.data.expect("data should be set");
        assert_eq!(data["kind"], "too_large");
        assert_eq!(data["size"], 5_000_000);
        assert_eq!(data["limit"], 2_097_152);
    }

    #[test]
    fn transport_errors_map_to_internal_error() {
        let timeout = Error::Timeout {
            endpoint: "tasks.list",
        }
        .to_mcp();
        assert_eq!(timeout.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert!(timeout.message.contains("tasks.list"));

        let network = Error::Network {
            endpoint: "tasks.list",
            detail: "connection failed".into(),
        }
        .to_mcp();
        assert_eq!(network.code, rmcp::model::ErrorCode::INTERNAL_ERROR);

        let invalid = Error::InvalidResponse {
            endpoint: "tasks.list",
            detail: "expected JSON".into(),
        }
        .to_mcp();
        assert_eq!(invalid.code, rmcp::model::ErrorCode::INTERNAL_ERROR);

        let io = Error::Io {
            detail: "permission denied".into(),
        }
        .to_mcp();
        assert_eq!(io.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn from_error_for_mcp_error_works() {
        let mcp: McpError = Error::InvalidArgument("nope".into()).into();
        assert_eq!(mcp.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }
}
