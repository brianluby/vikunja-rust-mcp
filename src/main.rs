//! Binary entry point: parses configuration, initializes logging and serves
//! the MCP server over stdio or streamable HTTP.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tracing::info;

use vikunja_rust_mcp::config::{Cli, Config, Transport};
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
    let server = VikunjaMcpServer::new(client);

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
            serve_http(server, cli.bind, cli.allowed_hosts).await?;
        }
    }
    Ok(())
}

/// Serves the MCP server over streamable HTTP at `/mcp`, with a `/healthz`
/// endpoint for liveness checks.
async fn serve_http(
    server: VikunjaMcpServer,
    bind: SocketAddr,
    extra_allowed_hosts: Vec<String>,
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

    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .route("/healthz", axum::routing::get(|| async { "ok" }));

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
