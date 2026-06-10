//! Binary entry point: parses configuration, initializes logging and serves
//! the MCP server over stdio or streamable HTTP.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::Request;
use axum::http::{StatusCode, header::AUTHORIZATION};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tracing::info;

use vikunja_rust_mcp::config::{ApiToken, Cli, Config, Transport, validate_http_auth};
use vikunja_rust_mcp::mcp::VikunjaMcpServer;
use vikunja_rust_mcp::vikunja::VikunjaClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Logs must go to stderr: stdout carries the MCP protocol in stdio mode.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = Config::from_cli(&cli).context("invalid configuration")?;
    info!(
        vikunja_url = %config.vikunja_url,
        transport = ?cli.transport,
        "starting vikunja-rust-mcp"
    );

    let client = VikunjaClient::new(&config).context("failed to initialize Vikunja client")?;
    let server = VikunjaMcpServer::new(client)
        .with_date_config(config.dates)
        .with_attachment_sandbox(config.attachment_sandbox);

    match cli.transport {
        Transport::Stdio => {
            info!("serving MCP over stdio");
            let service = server
                .serve(stdio())
                .await
                .context("failed to start stdio transport")?;
            service.waiting().await?;
        }
        Transport::Http => {
            let auth_token = validate_http_auth(
                &cli.bind,
                cli.http_auth_token.as_deref(),
                cli.http_allow_unauthenticated,
            )
            .context("invalid HTTP transport configuration")?;
            serve_http(server, cli.bind, cli.allowed_hosts, auth_token).await?;
        }
    }
    Ok(())
}

/// Serves the MCP server over streamable HTTP at `/mcp`, with a `/healthz`
/// endpoint for liveness checks. When `auth_token` is set, `/mcp` requires
/// `Authorization: Bearer <token>`; `/healthz` stays open.
async fn serve_http(
    server: VikunjaMcpServer,
    bind: SocketAddr,
    extra_allowed_hosts: Vec<String>,
    auth_token: Option<ApiToken>,
) -> anyhow::Result<()> {
    let mut allowed_hosts: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        bind.ip().to_string(),
    ];
    allowed_hosts.extend(
        extra_allowed_hosts
            .into_iter()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty()),
    );

    let http_config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        http_config,
    );

    let mut mcp_router = axum::Router::new().nest_service("/mcp", service);
    match &auth_token {
        Some(token) => {
            let token = Arc::new(token.clone());
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
    let router = mcp_router.route("/healthz", axum::routing::get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    info!(%bind, "serving MCP over streamable HTTP at /mcp");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed")?;
    Ok(())
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

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        // If the signal handler cannot be installed, fall back to running
        // until the process is killed externally.
        std::future::pending::<()>().await;
    }
    info!("shutting down");
}
