//! Shared helpers for integration tests.
//!
//! Each file in `tests/` is its own crate, so not every helper is used by
//! every test crate.
#![allow(dead_code)]

use clap::Parser;
use vikunja_rust_mcp::config::{Cli, Config};
use vikunja_rust_mcp::vikunja::VikunjaClient;

pub const TEST_TOKEN: &str = "tk_test_token";

/// Builds a validated [`Config`] pointing at the given base URL.
pub fn test_config(base_url: &str, timeout_secs: u64) -> Config {
    let cli = Cli::try_parse_from([
        "vikunja-rust-mcp",
        "--vikunja-url",
        base_url,
        "--api-token",
        TEST_TOKEN,
        "--timeout-secs",
        &timeout_secs.to_string(),
    ])
    .expect("test CLI args should parse");
    Config::from_cli(&cli).expect("test config should validate")
}

/// Builds a [`VikunjaClient`] pointing at the given base URL.
pub fn test_client(base_url: &str) -> VikunjaClient {
    VikunjaClient::new(&test_config(base_url, 5)).expect("client should build")
}

/// Builds a [`VikunjaClient`] with a short timeout for timeout tests.
pub fn test_client_with_timeout(base_url: &str, timeout_secs: u64) -> VikunjaClient {
    VikunjaClient::new(&test_config(base_url, timeout_secs)).expect("client should build")
}
