//! Configuration parsing and validation.
//!
//! Configuration is read from CLI flags with environment variable fallbacks
//! (`VIKUNJA_URL`, `VIKUNJA_API_TOKEN`, ...). The API token is wrapped in
//! [`ApiToken`] so it can never leak through `Debug` or `Display` formatting.

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use url::Url;

/// Transport over which the MCP server is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Transport {
    /// Serve MCP over stdin/stdout for local clients (Claude Desktop, etc.).
    Stdio,
    /// Serve MCP over streamable HTTP for remote or hosted use.
    Http,
}

/// Command line interface for the server binary.
#[derive(Debug, Parser)]
#[command(
    name = "vikunja-rust-mcp",
    version,
    about = "Model Context Protocol server for Vikunja"
)]
pub struct Cli {
    /// Transport to serve MCP over.
    #[arg(long, value_enum, default_value_t = Transport::Stdio, env = "MCP_TRANSPORT")]
    pub transport: Transport,

    /// Socket address to bind when using the HTTP transport.
    #[arg(long, default_value = "127.0.0.1:8077", env = "MCP_HTTP_BIND")]
    pub bind: SocketAddr,

    /// Extra `Host` header values accepted by the HTTP transport
    /// (comma separated). `localhost`, `127.0.0.1` and `::1` are always
    /// accepted.
    #[arg(long, env = "MCP_HTTP_ALLOWED_HOSTS", value_delimiter = ',')]
    pub allowed_hosts: Vec<String>,

    /// Base URL of the Vikunja instance, e.g. `https://try.vikunja.io`.
    #[arg(long, env = "VIKUNJA_URL")]
    pub vikunja_url: Option<String>,

    /// Vikunja API token (create one under Settings -> API tokens).
    #[arg(long, env = "VIKUNJA_API_TOKEN", hide_env_values = true)]
    pub api_token: Option<String>,

    /// Timeout in seconds for requests to the Vikunja API.
    #[arg(long, default_value_t = 30, env = "VIKUNJA_TIMEOUT_SECS")]
    pub timeout_secs: u64,

    /// Page size used when a tool call does not specify `per_page`.
    /// Vikunja instances cap this server-side (50 by default).
    #[arg(long, default_value_t = 50, env = "VIKUNJA_DEFAULT_PAGE_SIZE")]
    pub default_page_size: u32,
}

/// A Vikunja API token that redacts itself in all formatting output.
#[derive(Clone)]
pub struct ApiToken(String);

impl ApiToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Returns the raw token. Only the HTTP client constructing the
    /// `Authorization` header should call this.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

impl fmt::Display for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Errors produced while validating configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "VIKUNJA_URL is not set: pass --vikunja-url or set the VIKUNJA_URL environment variable (e.g. https://try.vikunja.io)"
    )]
    MissingUrl,
    #[error("VIKUNJA_URL `{url}` is not a valid URL: {reason}")]
    InvalidUrl { url: String, reason: String },
    #[error("VIKUNJA_URL `{url}` must use http or https")]
    UnsupportedScheme { url: String },
    #[error(
        "VIKUNJA_API_TOKEN is not set: pass --api-token or set the VIKUNJA_API_TOKEN environment variable"
    )]
    MissingToken,
    #[error("VIKUNJA_API_TOKEN is empty")]
    EmptyToken,
    #[error("timeout must be at least 1 second")]
    InvalidTimeout,
    #[error("default page size must be between 1 and 250")]
    InvalidPageSize,
}

/// Validated runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the Vikunja instance, without the `/api/v1` suffix.
    pub vikunja_url: Url,
    pub api_token: ApiToken,
    pub timeout: Duration,
    pub default_page_size: u32,
}

impl Config {
    /// Builds and validates a [`Config`] from parsed CLI arguments.
    pub fn from_cli(cli: &Cli) -> Result<Self, ConfigError> {
        let raw_url = cli
            .vikunja_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::MissingUrl)?;

        let vikunja_url = parse_base_url(raw_url)?;

        let token = cli
            .api_token
            .as_deref()
            .ok_or(ConfigError::MissingToken)?
            .trim();
        if token.is_empty() {
            return Err(ConfigError::EmptyToken);
        }

        if cli.timeout_secs == 0 {
            return Err(ConfigError::InvalidTimeout);
        }
        if cli.default_page_size == 0 || cli.default_page_size > 250 {
            return Err(ConfigError::InvalidPageSize);
        }

        Ok(Self {
            vikunja_url,
            api_token: ApiToken::new(token),
            timeout: Duration::from_secs(cli.timeout_secs),
            default_page_size: cli.default_page_size,
        })
    }
}

/// Parses the instance base URL, tolerating a trailing slash or an
/// accidentally included `/api/v1` suffix.
fn parse_base_url(raw: &str) -> Result<Url, ConfigError> {
    let mut url = Url::parse(raw).map_err(|e| ConfigError::InvalidUrl {
        url: raw.to_string(),
        reason: e.to_string(),
    })?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ConfigError::UnsupportedScheme {
            url: raw.to_string(),
        });
    }
    if url.host_str().is_none() {
        return Err(ConfigError::InvalidUrl {
            url: raw.to_string(),
            reason: "missing host".to_string(),
        });
    }

    let trimmed = url
        .path()
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/')
        .to_string();
    url.set_path(&trimmed);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("vikunja-rust-mcp").chain(args.iter().copied()))
            .expect("CLI args should parse")
    }

    #[test]
    fn valid_config_parses() {
        let cli = cli(&[
            "--vikunja-url",
            "https://try.vikunja.io",
            "--api-token",
            "tk_secret",
        ]);
        let config = Config::from_cli(&cli).unwrap();
        // `Url` normalizes a root URL with a trailing slash.
        assert_eq!(config.vikunja_url.as_str(), "https://try.vikunja.io/");
        assert_eq!(config.api_token.reveal(), "tk_secret");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.default_page_size, 50);
    }

    #[test]
    fn trailing_slash_and_api_suffix_are_stripped() {
        for raw in [
            "https://vikunja.example.com/",
            "https://vikunja.example.com/api/v1",
            "https://vikunja.example.com/api/v1/",
        ] {
            let cli = cli(&["--vikunja-url", raw, "--api-token", "t"]);
            let config = Config::from_cli(&cli).unwrap();
            assert_eq!(
                config.vikunja_url.as_str(),
                "https://vikunja.example.com/",
                "raw: {raw}"
            );
        }
    }

    #[test]
    fn subpath_installations_are_preserved() {
        let cli = cli(&[
            "--vikunja-url",
            "https://example.com/vikunja/",
            "--api-token",
            "t",
        ]);
        let config = Config::from_cli(&cli).unwrap();
        assert_eq!(config.vikunja_url.as_str(), "https://example.com/vikunja");
    }

    #[test]
    fn missing_url_is_an_error() {
        let cli = cli(&["--api-token", "t"]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::MissingUrl)
        ));
    }

    #[test]
    fn invalid_url_is_an_error() {
        let cli = cli(&["--vikunja-url", "not a url", "--api-token", "t"]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn non_http_scheme_is_an_error() {
        let cli = cli(&["--vikunja-url", "ftp://example.com", "--api-token", "t"]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::UnsupportedScheme { .. })
        ));
    }

    #[test]
    fn missing_token_is_an_error() {
        let cli = cli(&["--vikunja-url", "https://example.com"]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::MissingToken)
        ));
    }

    #[test]
    fn blank_token_is_an_error() {
        let cli = cli(&["--vikunja-url", "https://example.com", "--api-token", "  "]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::EmptyToken)
        ));
    }

    #[test]
    fn zero_timeout_is_an_error() {
        let cli = cli(&[
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "t",
            "--timeout-secs",
            "0",
        ]);
        assert!(matches!(
            Config::from_cli(&cli),
            Err(ConfigError::InvalidTimeout)
        ));
    }

    #[test]
    fn out_of_range_page_size_is_an_error() {
        for size in ["0", "251"] {
            let cli = cli(&[
                "--vikunja-url",
                "https://example.com",
                "--api-token",
                "t",
                "--default-page-size",
                size,
            ]);
            assert!(matches!(
                Config::from_cli(&cli),
                Err(ConfigError::InvalidPageSize)
            ));
        }
    }

    #[test]
    fn token_is_redacted_in_debug_and_display() {
        let token = ApiToken::new("tk_super_secret");
        assert!(!format!("{token:?}").contains("super_secret"));
        assert!(!format!("{token}").contains("super_secret"));

        let cli = cli(&[
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "tk_super_secret",
        ]);
        let config = Config::from_cli(&cli).unwrap();
        assert!(!format!("{config:?}").contains("super_secret"));
    }

    #[test]
    fn transport_and_bind_have_defaults() {
        let cli = cli(&[]);
        assert_eq!(cli.transport, Transport::Stdio);
        assert_eq!(cli.bind.port(), 8077);
        assert!(cli.allowed_hosts.is_empty());
    }
}
