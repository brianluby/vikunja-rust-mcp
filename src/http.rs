//! HTTP transport router: the MCP endpoint at `/mcp` (optionally behind
//! bearer authentication), unauthenticated `GET /healthz` (liveness) and
//! `GET /readyz` (Vikunja readiness), and an optional `GET /metrics`
//! Prometheus endpoint.
//!
//! `/healthz`, `/readyz` and `/metrics` are deliberately unauthenticated so
//! orchestrator probes and scrapers work without credentials: their
//! responses carry no secrets — `/readyz` reports only a coarse error class
//! and `/metrics` contains only fixed low-cardinality labels.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tracing::{debug, info, warn};

use crate::config::ApiToken;
use crate::mcp::VikunjaMcpServer;
use crate::metrics::{Metrics, method_label, route_label, status_class_label};
use crate::vikunja::VikunjaClient;

/// Content type of the Prometheus text exposition format.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// `Host` header values accepted by the HTTP transport: loopback names, the
/// bind IP and any extra hosts from configuration (trimmed, blanks dropped).
pub fn allowed_hosts(bind_ip: IpAddr, extra: Vec<String>) -> Vec<String> {
    let mut hosts: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        bind_ip.to_string(),
    ];
    hosts.extend(
        extra
            .into_iter()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty()),
    );
    hosts
}

/// Builds the HTTP transport router.
///
/// - `/mcp`: the MCP streamable HTTP endpoint; requires
///   `Authorization: Bearer <token>` when `auth_token` is set.
/// - `/healthz`: cheap unauthenticated liveness ("is the process up").
/// - `/readyz`: unauthenticated readiness — probes the configured Vikunja
///   instance via the existing safe probe and answers 200/503.
/// - `/metrics`: Prometheus text, present only when `metrics` is set.
///
/// Every request is recorded by a telemetry layer that emits a structured
/// log line and, when enabled, updates the metrics registry.
pub fn build_router(
    server: VikunjaMcpServer,
    allowed_hosts: Vec<String>,
    auth_token: Option<ApiToken>,
    metrics: Option<Arc<Metrics>>,
) -> Router {
    let client: VikunjaClient = server.client().clone();

    let http_config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        http_config,
    );

    let mut mcp_router = Router::new().nest_service("/mcp", service);
    match auth_token {
        Some(token) => {
            let token = Arc::new(token);
            mcp_router = mcp_router.layer(axum::middleware::from_fn(
                move |request: Request, next: Next| {
                    let token = Arc::clone(&token);
                    async move { require_bearer(&token, request, next).await }
                },
            ));
            info!("HTTP MCP endpoint requires bearer authentication");
        }
        None => {
            info!("HTTP MCP endpoint is served without authentication");
        }
    }

    let mut router = mcp_router
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .route(
            "/readyz",
            axum::routing::get(move || {
                let client = client.clone();
                async move { readyz(&client).await }
            }),
        );
    if let Some(metrics) = &metrics {
        let metrics = Arc::clone(metrics);
        router = router.route(
            "/metrics",
            axum::routing::get(move || {
                let metrics = Arc::clone(&metrics);
                async move { metrics_endpoint(&metrics) }
            }),
        );
        info!("Prometheus metrics enabled at /metrics");
    }

    router.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let metrics = metrics.clone();
            async move { telemetry(metrics, request, next).await }
        },
    ))
}

/// Readiness: verifies the configured Vikunja instance answers the existing
/// lightweight probe. The response body carries only `ready` or a coarse
/// error class — never tokens, headers, URLs or Vikunja response text.
async fn readyz(client: &VikunjaClient) -> Response {
    match client.probe().await {
        Ok(_) => (StatusCode::OK, "ready\n").into_response(),
        Err(err) => {
            let kind = err.metric_label();
            warn!(kind, "readiness probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("not ready: {kind}\n"),
            )
                .into_response()
        }
    }
}

/// Renders the metrics registry as Prometheus text.
fn metrics_endpoint(metrics: &Metrics) -> Response {
    ([(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], metrics.render()).into_response()
}

/// Telemetry layer: one structured log line per request (route category,
/// method, status class, duration — never raw paths, queries or headers)
/// plus metrics when enabled. Probe/scrape routes log at debug level so
/// orchestrator polling does not flood the logs.
async fn telemetry(metrics: Option<Arc<Metrics>>, request: Request, next: Next) -> Response {
    let route = route_label(request.uri().path());
    let method = method_label(request.method().as_str());
    let started = Instant::now();

    let response = next.run(request).await;

    let duration = started.elapsed();
    let status = response.status().as_u16();
    let status_class = status_class_label(status);
    if let Some(metrics) = &metrics {
        metrics.record_http_request(route, method, status, duration);
    }
    let duration_ms = duration.as_millis() as u64;
    if matches!(route, "/healthz" | "/readyz" | "/metrics") {
        debug!(
            route,
            method, status, status_class, duration_ms, "http request"
        );
    } else {
        info!(
            route,
            method, status, status_class, duration_ms, "http request"
        );
    }
    response
}

/// Rejects requests that do not carry `Authorization: Bearer <expected>`.
async fn require_bearer(expected: &ApiToken, request: Request, next: Next) -> Response {
    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|presented| {
            constant_time_eq(presented.as_bytes(), expected.reveal().as_bytes())
        });
    if authorized {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response()
    }
}

/// Compares two byte strings without short-circuiting on the first
/// mismatch, so the comparison time does not reveal how much of the token
/// matched. (Length is still observable, as with most token checks.)
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_compares_correctly() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn allowed_hosts_include_defaults_and_trimmed_extras() {
        let hosts = allowed_hosts(
            "10.0.0.5".parse().unwrap(),
            vec!["  mcp.example.com  ".to_string(), "   ".to_string()],
        );
        assert_eq!(
            hosts,
            vec![
                "localhost",
                "127.0.0.1",
                "::1",
                "10.0.0.5",
                "mcp.example.com"
            ]
        );
    }
}
