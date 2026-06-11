//! Binary entry point: parses configuration, initializes logging and serves
//! the MCP server over stdio or streamable HTTP.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use tracing::info;

use vikunja_rust_mcp::config::{
    ApiToken, Cli, Config, Transport, validate_http_auth, validate_metrics_transport,
};
use vikunja_rust_mcp::http::{allowed_hosts, build_router};
use vikunja_rust_mcp::mcp::VikunjaMcpServer;
use vikunja_rust_mcp::metrics::Metrics;
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
    validate_metrics_transport(cli.transport, cli.http_enable_metrics)
        .context("invalid configuration")?;
    info!(
        vikunja_url = %config.vikunja_url,
        transport = ?cli.transport,
        "starting vikunja-rust-mcp"
    );

    let metrics = cli
        .http_enable_metrics
        .then(|| Arc::new(Metrics::default()));
    let mut client = VikunjaClient::new(&config).context("failed to initialize Vikunja client")?;
    if let Some(metrics) = &metrics {
        client = client.with_metrics(Arc::clone(metrics));
    }
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
            serve_http(server, cli.bind, cli.allowed_hosts, auth_token, metrics).await?;
        }
    }
    Ok(())
}

/// Serves the MCP server over streamable HTTP at `/mcp`, with `/healthz`
/// (liveness), `/readyz` (Vikunja readiness) and — when enabled — a
/// `/metrics` Prometheus endpoint. When `auth_token` is set, `/mcp` requires
/// `Authorization: Bearer <token>`; the probe endpoints stay open.
async fn serve_http(
    server: VikunjaMcpServer,
    bind: SocketAddr,
    extra_allowed_hosts: Vec<String>,
    auth_token: Option<ApiToken>,
    metrics: Option<Arc<Metrics>>,
) -> anyhow::Result<()> {
    let hosts = allowed_hosts(bind.ip(), extra_allowed_hosts);
    let router = build_router(server, hosts, auth_token, metrics);

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

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        // If the signal handler cannot be installed, fall back to running
        // until the process is killed externally.
        std::future::pending::<()>().await;
    }
    info!("shutting down");
}
