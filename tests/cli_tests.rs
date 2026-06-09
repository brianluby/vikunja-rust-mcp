//! Smoke tests for the compiled binary: CLI surface, configuration
//! validation, the stdio transport handshake and the HTTP transport.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn binary() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vikunja-rust-mcp"));
    // Isolate from any ambient configuration.
    cmd.env_remove("VIKUNJA_URL")
        .env_remove("VIKUNJA_API_TOKEN")
        .env_remove("MCP_TRANSPORT")
        .env_remove("MCP_HTTP_BIND")
        .env_remove("MCP_HTTP_ALLOWED_HOSTS")
        .env_remove("MCP_HTTP_AUTH_TOKEN")
        .env_remove("MCP_HTTP_ALLOW_UNAUTHENTICATED")
        .env_remove("VIKUNJA_TIMEOUT_SECS")
        .env_remove("VIKUNJA_DEFAULT_PAGE_SIZE");
    cmd
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Waits up to 10s for the child to exit on its own (so coverage data is
/// flushed); falls back to the `KillOnDrop` backstop otherwise.
fn wait_for_exit(child: &mut Child) -> Option<std::process::ExitStatus> {
    for _ in 0..100 {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn help_lists_options() {
    let output = binary().arg("--help").output().expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for needle in [
        "--transport",
        "--vikunja-url",
        "--api-token",
        "--bind",
        "--http-auth-token",
        "VIKUNJA_URL",
        "VIKUNJA_API_TOKEN",
        "MCP_HTTP_AUTH_TOKEN",
    ] {
        assert!(stdout.contains(needle), "--help should mention {needle}");
    }
}

#[test]
fn version_prints() {
    let output = binary().arg("--version").output().expect("run --version");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("vikunja-rust-mcp"));
}

#[test]
fn missing_url_fails_with_helpful_error() {
    let output = binary()
        .args(["--api-token", "t"])
        .output()
        .expect("run binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("VIKUNJA_URL"), "stderr was: {stderr}");
}

#[test]
fn missing_token_fails_with_helpful_error() {
    let output = binary()
        .args(["--vikunja-url", "https://example.com"])
        .output()
        .expect("run binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("VIKUNJA_API_TOKEN"), "stderr was: {stderr}");
}

#[test]
fn invalid_url_fails_with_helpful_error() {
    let output = binary()
        .args(["--vikunja-url", "not a url", "--api-token", "t"])
        .output()
        .expect("run binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a valid URL"), "stderr was: {stderr}");
}

#[test]
fn token_never_appears_in_output() {
    let token = "tk_extremely_secret_token";
    let output = binary()
        .args([
            "--vikunja-url",
            "ftp://bad-scheme.example",
            "--api-token",
            token,
        ])
        .env("RUST_LOG", "trace")
        .output()
        .expect("run binary");
    assert!(!output.status.success());
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!all.contains(token), "token leaked in output");
}

/// Initialize handshake over stdio: write one JSON-RPC request, read the
/// response, confirm the server identifies itself and lists tools.
#[test]
fn stdio_transport_answers_initialize() {
    let child = binary()
        .args(["--vikunja-url", "http://127.0.0.1:1", "--api-token", "t"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut child = KillOnDrop(child);

    let mut stdin = child.0.stdin.take().expect("stdin");
    let stdout = child.0.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0"}
        }
    });
    writeln!(stdin, "{initialize}").expect("write initialize");
    stdin.flush().expect("flush");

    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");
    let response: serde_json::Value = serde_json::from_str(&line).expect("valid JSON-RPC");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "vikunja-rust-mcp");

    // Complete the handshake, then list tools.
    let initialized = serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized"
    });
    writeln!(stdin, "{initialized}").expect("write initialized");
    let list_tools = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list"
    });
    writeln!(stdin, "{list_tools}").expect("write tools/list");
    stdin.flush().expect("flush");

    let mut line = String::new();
    reader.read_line(&mut line).expect("read tools response");
    let response: serde_json::Value = serde_json::from_str(&line).expect("valid JSON-RPC");
    let tools = response["result"]["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|t| t["name"] == "vikunja_projects_list"));

    // Closing stdin ends the session; the server should exit cleanly.
    drop(stdin);
    let status = wait_for_exit(&mut child.0);
    assert!(status.is_some(), "server did not exit after stdin closed");
}

/// HTTP transport: /healthz answers and /mcp accepts an initialize POST.
#[test]
fn http_transport_serves_mcp() {
    // Reserve a port, then free it for the child process.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);

    let child = binary()
        .args([
            "--transport",
            "http",
            "--bind",
            &addr.to_string(),
            "--vikunja-url",
            "http://127.0.0.1:1",
            "--api-token",
            "t",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut server = KillOnDrop(child);

    // Wait for the server to come up.
    let health_url = format!("http://{addr}/healthz");
    let mut healthy = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(response) = ureq_get(&health_url)
            && response.contains("ok")
        {
            healthy = true;
            break;
        }
    }
    assert!(healthy, "server did not become healthy at {health_url}");

    // POST an initialize request to /mcp.
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0"}
        }
    });
    let response = http_post_mcp(
        &format!("http://{addr}/mcp"),
        &initialize.to_string(),
        "vikunja-rust-mcp",
        None,
    );
    assert!(
        response.contains("vikunja-rust-mcp"),
        "unexpected /mcp response: {response}"
    );
    assert!(
        response.contains("mcp-session-id"),
        "missing session header: {response}"
    );

    // SIGINT triggers the graceful shutdown path.
    let pid = server.0.id();
    let _ = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status();
    let status = wait_for_exit(&mut server.0);
    assert!(status.is_some(), "server did not exit after SIGINT");
}

/// With MCP_HTTP_AUTH_TOKEN set, /mcp requires the bearer token while
/// /healthz stays open.
#[test]
fn http_transport_enforces_bearer_auth() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);

    let child = binary()
        .args([
            "--transport",
            "http",
            "--bind",
            &addr.to_string(),
            "--vikunja-url",
            "http://127.0.0.1:1",
            "--api-token",
            "t",
            "--http-auth-token",
            "mcp-secret",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut server = KillOnDrop(child);

    let health_url = format!("http://{addr}/healthz");
    let mut healthy = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(response) = ureq_get(&health_url)
            && response.contains("ok")
        {
            healthy = true;
            break;
        }
    }
    assert!(healthy, "server did not become healthy at {health_url}");

    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0"}
        }
    })
    .to_string();
    let mcp_url = format!("http://{addr}/mcp");

    // No token: rejected before reaching the MCP service.
    let response = http_post_mcp(&mcp_url, &initialize, "401", None);
    assert!(response.contains("401"), "expected 401, got: {response}");

    // Wrong token: rejected.
    let response = http_post_mcp(&mcp_url, &initialize, "401", Some("wrong-token"));
    assert!(response.contains("401"), "expected 401, got: {response}");

    // Correct token: the MCP handshake answers.
    let response = http_post_mcp(
        &mcp_url,
        &initialize,
        "vikunja-rust-mcp",
        Some("mcp-secret"),
    );
    assert!(
        response.contains("vikunja-rust-mcp"),
        "unexpected /mcp response: {response}"
    );

    let pid = server.0.id();
    let _ = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status();
    let status = wait_for_exit(&mut server.0);
    assert!(status.is_some(), "server did not exit after SIGINT");
}

/// Binding beyond loopback without authentication must refuse to start
/// unless --http-allow-unauthenticated is passed explicitly.
#[test]
fn http_non_loopback_bind_requires_auth_or_explicit_opt_out() {
    let output = binary()
        .args([
            "--transport",
            "http",
            "--bind",
            "0.0.0.0:0",
            "--vikunja-url",
            "http://127.0.0.1:1",
            "--api-token",
            "t",
        ])
        .output()
        .expect("run binary");
    assert!(!output.status.success(), "server should refuse to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MCP_HTTP_AUTH_TOKEN"),
        "stderr was: {stderr}"
    );
}

/// Minimal HTTP GET (avoids extra dev-dependencies for two requests).
fn ureq_get(url: &str) -> Result<String, std::io::Error> {
    use std::io::Read;
    let address = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or_default();
    let path = format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default());
    let mut stream = std::net::TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

/// Minimal HTTP POST for the MCP endpoint. The response is an SSE stream
/// that stays open, so this reads until `needle` shows up (or 5s pass)
/// instead of waiting for EOF.
fn http_post_mcp(url: &str, body: &str, needle: &str, bearer: Option<&str>) -> String {
    use std::io::Read;
    let address = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or_default();
    let path = format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default());
    let mut stream = std::net::TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set timeout");
    let auth_header = bearer
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{auth_header}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .expect("write request");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut response = String::new();
    let mut buf = [0u8; 4096];
    while std::time::Instant::now() < deadline && !response.contains(needle) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
    response
}
