//! Configuration parsing and validation.
//!
//! Configuration is read from CLI flags with environment variable fallbacks
//! (`VIKUNJA_URL`, `VIKUNJA_API_TOKEN`, ...). The API token is wrapped in
//! [`ApiToken`] so it can never leak through `Debug` or `Display` formatting.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use url::Url;

use crate::dates::{DateConfig, parse_time_of_day};
use crate::sandbox::{AttachmentSandbox, InvalidRoot};

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

    /// Bearer token clients must present (`Authorization: Bearer <token>`)
    /// to reach the HTTP MCP endpoint. Required for non-loopback binds
    /// unless --http-allow-unauthenticated is set.
    #[arg(long, env = "MCP_HTTP_AUTH_TOKEN", hide_env_values = true)]
    pub http_auth_token: Option<String>,

    /// Serve HTTP without authentication even on non-loopback binds.
    /// Only use this behind an authenticating reverse proxy.
    #[arg(long, env = "MCP_HTTP_ALLOW_UNAUTHENTICATED", default_value_t = false)]
    pub http_allow_unauthenticated: bool,

    /// Expose Prometheus metrics at `GET /metrics` on the HTTP transport.
    /// The metrics carry only fixed low-cardinality labels (no ids, titles
    /// or tokens). Requires --transport http.
    #[arg(long, env = "MCP_HTTP_ENABLE_METRICS", default_value_t = false)]
    pub http_enable_metrics: bool,

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

    /// Time of day (HH:MM) applied when a date shortcut resolves to a
    /// calendar day, e.g. `tomorrow` -> tomorrow at this time.
    #[arg(long, default_value = "09:00", env = "VIKUNJA_DATE_DEFAULT_TIME")]
    pub date_default_time: String,

    /// Time of day (HH:MM) used by the `end of week` date shortcut.
    #[arg(long, default_value = "23:59", env = "VIKUNJA_DATE_END_OF_DAY_TIME")]
    pub date_end_of_day_time: String,

    /// Directories attachment uploads may read files from (repeat the flag
    /// or separate with commas). When unset, any server-local path is
    /// allowed for vikunja_task_attachments_upload's file_path.
    #[arg(
        long = "attachment-upload-root",
        env = "VIKUNJA_ATTACHMENT_UPLOAD_ROOTS",
        value_delimiter = ','
    )]
    pub attachment_upload_roots: Vec<PathBuf>,

    /// Directories attachment downloads may write files to (repeat the
    /// flag or separate with commas). When unset, any server-local path is
    /// allowed for vikunja_task_attachments_download's save_path.
    #[arg(
        long = "attachment-download-root",
        env = "VIKUNJA_ATTACHMENT_DOWNLOAD_ROOTS",
        value_delimiter = ','
    )]
    pub attachment_download_roots: Vec<PathBuf>,
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
    #[error("{flag}: {reason}")]
    InvalidTimeOfDay { flag: &'static str, reason: String },
    #[error("MCP_HTTP_AUTH_TOKEN is set but empty")]
    EmptyHttpAuthToken,
    #[error(
        "{0}: pass an existing directory for --attachment-upload-root/--attachment-download-root"
    )]
    InvalidAttachmentRoot(#[source] InvalidRoot),
    #[error(
        "refusing to serve unauthenticated HTTP on non-loopback address {bind}: set MCP_HTTP_AUTH_TOKEN (recommended), or pass --http-allow-unauthenticated if an authenticating reverse proxy fronts this server"
    )]
    UnauthenticatedNonLoopback { bind: SocketAddr },
    #[error(
        "MCP_HTTP_ENABLE_METRICS/--http-enable-metrics requires --transport http: the stdio transport has no HTTP endpoint to serve /metrics from"
    )]
    MetricsRequireHttp,
}

/// Validates that Prometheus metrics are only enabled together with the
/// HTTP transport (the stdio transport has no endpoint to serve them from,
/// so a metrics flag there is a misconfiguration and fails fast).
pub fn validate_metrics_transport(
    transport: Transport,
    enable_metrics: bool,
) -> Result<(), ConfigError> {
    if enable_metrics && transport != Transport::Http {
        return Err(ConfigError::MetricsRequireHttp);
    }
    Ok(())
}

/// Validates the HTTP transport's authentication configuration and returns
/// the trimmed bearer token, if any.
///
/// The MCP endpoint exposes destructive Vikunja actions (and file
/// read/write via the attachment tools), so serving it unauthenticated on a
/// non-loopback address is refused unless explicitly allowed for
/// reverse-proxy deployments.
pub fn validate_http_auth(
    bind: &SocketAddr,
    auth_token: Option<&str>,
    allow_unauthenticated: bool,
) -> Result<Option<ApiToken>, ConfigError> {
    match auth_token {
        Some(token) => {
            let token = token.trim();
            if token.is_empty() {
                return Err(ConfigError::EmptyHttpAuthToken);
            }
            Ok(Some(ApiToken::new(token)))
        }
        None => {
            if !bind.ip().is_loopback() && !allow_unauthenticated {
                return Err(ConfigError::UnauthenticatedNonLoopback { bind: *bind });
            }
            Ok(None)
        }
    }
}

/// Validated runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the Vikunja instance, without the `/api/v1` suffix.
    pub vikunja_url: Url,
    pub api_token: ApiToken,
    pub timeout: Duration,
    pub default_page_size: u32,
    /// Times of day applied by date shortcuts.
    pub dates: DateConfig,
    /// Path restrictions for the attachment tools' file operations.
    pub attachment_sandbox: AttachmentSandbox,
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

        let dates = DateConfig {
            default_time: parse_time_of_day(&cli.date_default_time).map_err(|reason| {
                ConfigError::InvalidTimeOfDay {
                    flag: "VIKUNJA_DATE_DEFAULT_TIME",
                    reason,
                }
            })?,
            end_of_day_time: parse_time_of_day(&cli.date_end_of_day_time).map_err(|reason| {
                ConfigError::InvalidTimeOfDay {
                    flag: "VIKUNJA_DATE_END_OF_DAY_TIME",
                    reason,
                }
            })?,
        };

        let attachment_sandbox =
            AttachmentSandbox::new(&cli.attachment_upload_roots, &cli.attachment_download_roots)
                .map_err(ConfigError::InvalidAttachmentRoot)?;

        Ok(Self {
            vikunja_url,
            api_token: ApiToken::new(token),
            timeout: Duration::from_secs(cli.timeout_secs),
            default_page_size: cli.default_page_size,
            dates,
            attachment_sandbox,
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
    fn date_times_default_to_nine_and_end_of_day() {
        let cli = cli(&["--vikunja-url", "https://example.com", "--api-token", "t"]);
        let config = Config::from_cli(&cli).unwrap();
        assert_eq!(config.dates, crate::dates::DateConfig::default());
    }

    #[test]
    fn custom_date_times_are_parsed() {
        let cli = cli(&[
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "t",
            "--date-default-time",
            "08:30",
            "--date-end-of-day-time",
            "22:00",
        ]);
        let config = Config::from_cli(&cli).unwrap();
        assert_eq!(
            config.dates.default_time,
            chrono::NaiveTime::from_hms_opt(8, 30, 0).unwrap()
        );
        assert_eq!(
            config.dates.end_of_day_time,
            chrono::NaiveTime::from_hms_opt(22, 0, 0).unwrap()
        );
    }

    #[test]
    fn invalid_date_times_are_rejected() {
        for (flag, value) in [
            ("--date-default-time", "9am"),
            ("--date-default-time", "25:00"),
            ("--date-end-of-day-time", "12:60"),
            ("--date-end-of-day-time", "midnight"),
        ] {
            let cli = cli(&[
                "--vikunja-url",
                "https://example.com",
                "--api-token",
                "t",
                flag,
                value,
            ]);
            let err = Config::from_cli(&cli).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidTimeOfDay { .. }),
                "{flag} {value}: {err:?}"
            );
            assert!(err.to_string().contains("HH:MM"), "{err}");
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
    fn attachment_roots_default_to_permissive_sandbox() {
        let cli = cli(&["--vikunja-url", "https://example.com", "--api-token", "t"]);
        assert!(cli.attachment_upload_roots.is_empty());
        assert!(cli.attachment_download_roots.is_empty());
        let config = Config::from_cli(&cli).unwrap();
        assert!(config.attachment_sandbox.is_permissive());
    }

    #[test]
    fn attachment_roots_are_parsed_and_validated() {
        let upload = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let cli = cli(&[
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "t",
            "--attachment-upload-root",
            upload.path().to_str().unwrap(),
            "--attachment-download-root",
            download.path().to_str().unwrap(),
        ]);
        let config = Config::from_cli(&cli).unwrap();
        assert!(!config.attachment_sandbox.is_permissive());
    }

    #[test]
    fn nonexistent_attachment_root_is_an_error() {
        let cli = cli(&[
            "--vikunja-url",
            "https://example.com",
            "--api-token",
            "t",
            "--attachment-upload-root",
            "/definitely/not/a/real/directory",
        ]);
        let err = Config::from_cli(&cli).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidAttachmentRoot(_)),
            "{err:?}"
        );
        assert!(err.to_string().contains("attachment root"), "{err}");
    }

    #[test]
    fn transport_and_bind_have_defaults() {
        let cli = cli(&[]);
        assert_eq!(cli.transport, Transport::Stdio);
        assert_eq!(cli.bind.port(), 8077);
        assert!(cli.allowed_hosts.is_empty());
        assert!(cli.http_auth_token.is_none());
        assert!(!cli.http_allow_unauthenticated);
        assert!(!cli.http_enable_metrics);
    }

    #[test]
    fn metrics_flag_is_parsed() {
        let cli = cli(&["--http-enable-metrics"]);
        assert!(cli.http_enable_metrics);
    }

    #[test]
    fn metrics_with_http_transport_is_valid() {
        for enabled in [true, false] {
            assert!(validate_metrics_transport(Transport::Http, enabled).is_ok());
        }
    }

    #[test]
    fn metrics_without_metrics_flag_is_valid_on_stdio() {
        assert!(validate_metrics_transport(Transport::Stdio, false).is_ok());
    }

    #[test]
    fn metrics_with_stdio_transport_is_rejected() {
        let err = validate_metrics_transport(Transport::Stdio, true).unwrap_err();
        assert!(matches!(err, ConfigError::MetricsRequireHttp));
        assert!(err.to_string().contains("--transport http"), "{err}");
    }

    #[test]
    fn http_auth_token_is_accepted_and_trimmed() {
        let bind: SocketAddr = "0.0.0.0:8077".parse().unwrap();
        let token = validate_http_auth(&bind, Some("  tk_http  "), false)
            .unwrap()
            .expect("token should be present");
        assert_eq!(token.reveal(), "tk_http");
    }

    #[test]
    fn blank_http_auth_token_is_an_error() {
        let bind: SocketAddr = "127.0.0.1:8077".parse().unwrap();
        assert!(matches!(
            validate_http_auth(&bind, Some("   "), false),
            Err(ConfigError::EmptyHttpAuthToken)
        ));
    }

    #[test]
    fn loopback_bind_may_skip_authentication() {
        for bind in ["127.0.0.1:8077", "[::1]:8077"] {
            let bind: SocketAddr = bind.parse().unwrap();
            assert!(validate_http_auth(&bind, None, false).unwrap().is_none());
        }
    }

    #[test]
    fn non_loopback_bind_without_token_is_refused() {
        for bind in ["0.0.0.0:8077", "192.168.1.10:8077", "[::]:8077"] {
            let bind: SocketAddr = bind.parse().unwrap();
            assert!(
                matches!(
                    validate_http_auth(&bind, None, false),
                    Err(ConfigError::UnauthenticatedNonLoopback { .. })
                ),
                "bind {bind} should be refused"
            );
        }
    }

    #[test]
    fn non_loopback_bind_allowed_with_explicit_opt_out() {
        let bind: SocketAddr = "0.0.0.0:8077".parse().unwrap();
        assert!(validate_http_auth(&bind, None, true).unwrap().is_none());
    }
}
