//! In-process tests for the HTTP transport router: liveness (`/healthz`),
//! readiness (`/readyz`), optional Prometheus metrics (`/metrics`) and the
//! interaction with bearer authentication on `/mcp`.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use vikunja_rust_mcp::config::ApiToken;
use vikunja_rust_mcp::http::{allowed_hosts, build_router};
use vikunja_rust_mcp::mcp::VikunjaMcpServer;
use vikunja_rust_mcp::metrics::Metrics;

use common::TEST_TOKEN;

/// A title that must never show up in metrics or probe responses.
const SENSITIVE_TITLE: &str = "SECRET-TASK-TITLE-DO-NOT-LEAK";

fn mcp_server(base_url: &str, metrics: Option<&Arc<Metrics>>) -> VikunjaMcpServer {
    let mut client = common::test_client(base_url);
    if let Some(metrics) = metrics {
        client = client.with_metrics(Arc::clone(metrics));
    }
    VikunjaMcpServer::new(client)
}

/// Serves `router` on an ephemeral loopback port.
async fn spawn_router(router: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve router");
    });
    addr
}

/// An address nothing listens on (reserved, then released).
fn unreachable_base_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    format!("http://{addr}")
}

fn default_hosts() -> Vec<String> {
    allowed_hosts("127.0.0.1".parse().unwrap(), Vec::new())
}

/// Mounts a Vikunja project-list mock answering the readiness probe.
async fn mock_vikunja_ok() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 1, "title": SENSITIVE_TITLE }
        ])))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn healthz_answers_ok_without_auth() {
    let vikunja = mock_vikunja_ok().await;
    let server = mcp_server(&vikunja.uri(), None);
    let router = build_router(
        server,
        default_hosts(),
        Some(ApiToken::new("mcp-secret")),
        None,
    );
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("GET /healthz");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.expect("body"), "ok");
}

#[tokio::test]
async fn readyz_reports_ready_when_vikunja_responds() {
    let vikunja = mock_vikunja_ok().await;
    let server = mcp_server(&vikunja.uri(), None);
    let router = build_router(server, default_hosts(), None, None);
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");
    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("body");
    assert!(body.contains("ready"), "body: {body}");
    assert!(!body.contains(TEST_TOKEN), "token leaked: {body}");
    assert!(!body.contains(SENSITIVE_TITLE), "task text leaked: {body}");
}

#[tokio::test]
async fn readyz_reports_unready_when_vikunja_is_unreachable() {
    let server = mcp_server(&unreachable_base_url(), None);
    let router = build_router(server, default_hosts(), None, None);
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");
    assert_eq!(response.status(), 503);
    let body = response.text().await.expect("body");
    assert!(body.contains("not ready"), "body: {body}");
    // Only the coarse error class is reported, never tokens or URLs.
    assert!(!body.contains(TEST_TOKEN), "token leaked: {body}");
    assert!(!body.contains("Bearer"), "header leaked: {body}");
}

#[tokio::test]
async fn readyz_reports_unready_on_vikunja_server_error() {
    let vikunja = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&vikunja)
        .await;
    let server = mcp_server(&vikunja.uri(), None);
    let router = build_router(server, default_hosts(), None, None);
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");
    assert_eq!(response.status(), 503);
    let body = response.text().await.expect("body");
    assert!(body.contains("not ready"), "body: {body}");
    assert!(!body.contains(TEST_TOKEN), "token leaked: {body}");
}

#[tokio::test]
async fn metrics_endpoint_is_disabled_by_default() {
    let vikunja = mock_vikunja_ok().await;
    let server = mcp_server(&vikunja.uri(), None);
    let router = build_router(server, default_hosts(), None, None);
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("GET /metrics");
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_text_when_enabled() {
    let vikunja = mock_vikunja_ok().await;
    let metrics = Arc::new(Metrics::default());
    let server = mcp_server(&vikunja.uri(), Some(&metrics));
    let router = build_router(server, default_hosts(), None, Some(Arc::clone(&metrics)));
    let addr = spawn_router(router).await;

    // Generate some traffic first so counters exist.
    reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("GET /healthz");
    reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");

    let response = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("GET /metrics");
    assert_eq!(response.status(), 200);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/plain"),
        "content-type: {content_type}"
    );

    let body = response.text().await.expect("body");
    for needle in [
        "# TYPE vikunja_mcp_http_requests_total counter",
        "# TYPE vikunja_mcp_http_request_duration_seconds histogram",
        "# TYPE vikunja_mcp_vikunja_requests_total counter",
        "# TYPE vikunja_mcp_vikunja_retries_total counter",
        "route=\"/healthz\"",
        "route=\"/readyz\"",
        "vikunja_mcp_vikunja_requests_total{endpoint=\"status.probe\",outcome=\"ok\"}",
    ] {
        assert!(body.contains(needle), "missing {needle} in:\n{body}");
    }
    // Metrics must never carry secrets or user-provided text.
    assert!(!body.contains(TEST_TOKEN), "token leaked:\n{body}");
    assert!(!body.contains(SENSITIVE_TITLE), "task text leaked:\n{body}");
}

#[tokio::test]
async fn metrics_record_vikunja_error_class_and_retries() {
    // Unreachable instance: the idempotent probe is retried once and the
    // failure is recorded under the `network` outcome.
    let metrics = Arc::new(Metrics::default());
    let server = mcp_server(&unreachable_base_url(), Some(&metrics));
    let router = build_router(server, default_hosts(), None, Some(Arc::clone(&metrics)));
    let addr = spawn_router(router).await;

    let response = reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");
    assert_eq!(response.status(), 503);

    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("GET /metrics")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains(
            "vikunja_mcp_vikunja_requests_total{endpoint=\"status.probe\",outcome=\"network\"} 1"
        ),
        "missing network outcome in:\n{body}"
    );
    assert!(
        body.contains("vikunja_mcp_vikunja_retries_total{endpoint=\"status.probe\"} 1"),
        "missing retry count in:\n{body}"
    );
    assert!(
        body.contains("status=\"5xx\""),
        "missing 5xx readyz status in:\n{body}"
    );
}

#[tokio::test]
async fn metrics_record_vikunja_http_error_outcome() {
    let vikunja = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&vikunja)
        .await;
    let metrics = Arc::new(Metrics::default());
    let server = mcp_server(&vikunja.uri(), Some(&metrics));
    let router = build_router(server, default_hosts(), None, Some(Arc::clone(&metrics)));
    let addr = spawn_router(router).await;

    reqwest::get(format!("http://{addr}/readyz"))
        .await
        .expect("GET /readyz");
    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("GET /metrics")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains(
            "vikunja_mcp_vikunja_requests_total{endpoint=\"status.probe\",outcome=\"server\"} 1"
        ),
        "missing server outcome in:\n{body}"
    );
}

#[tokio::test]
async fn probes_and_metrics_stay_open_while_mcp_requires_auth() {
    let vikunja = mock_vikunja_ok().await;
    let metrics = Arc::new(Metrics::default());
    let server = mcp_server(&vikunja.uri(), Some(&metrics));
    let router = build_router(
        server,
        default_hosts(),
        Some(ApiToken::new("mcp-secret")),
        Some(Arc::clone(&metrics)),
    );
    let addr = spawn_router(router).await;

    for path in ["/healthz", "/readyz", "/metrics"] {
        let response = reqwest::get(format!("http://{addr}{path}"))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_ne!(response.status(), 401, "{path} must not require auth");
        assert!(
            response.status().is_success(),
            "{path}: {}",
            response.status()
        );
    }

    // /mcp without a bearer token is rejected before reaching the service.
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "http-test", "version": "0"}
            }
        }))
        .send()
        .await
        .expect("POST /mcp");
    assert_eq!(response.status(), 401);

    // Unauthorized requests are still observable in the metrics.
    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("GET /metrics")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains("route=\"/mcp\",method=\"POST\",status=\"4xx\""),
        "missing 401 sample in:\n{body}"
    );
}

#[test]
fn allowed_hosts_always_include_loopback_and_bind_ip() {
    let hosts = allowed_hosts(
        "192.168.1.10".parse().unwrap(),
        vec![" mcp.example.com ".to_string(), String::new()],
    );
    for needle in [
        "localhost",
        "127.0.0.1",
        "::1",
        "192.168.1.10",
        "mcp.example.com",
    ] {
        assert!(
            hosts.iter().any(|h| h == needle),
            "missing {needle} in {hosts:?}"
        );
    }
    // Blank entries are dropped, surrounding whitespace is trimmed.
    assert!(!hosts.iter().any(|h| h.is_empty()));
}
