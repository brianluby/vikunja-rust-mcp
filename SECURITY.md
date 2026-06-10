# Security Policy

## Supported Versions

| Version | Supported |
|---|---|
| 0.3.x (latest release) | Yes |
| < 0.3 | No |

Only the latest release receives security fixes. Update to the newest
version before reporting an issue you can no longer reproduce there.

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Report vulnerabilities privately through GitHub's private vulnerability
reporting:

1. Go to <https://github.com/brianluby/vikunja-rust-mcp/security/advisories/new>
   (or the repository's **Security** tab, then **Report a vulnerability**).
2. Describe the issue: affected version or commit, configuration
   (transport, bind address, relevant flags or environment variables),
   reproduction steps, and the impact you believe it has.
3. Include a proof of concept if you have one. Redact real API tokens and
   instance URLs.

Reports are acknowledged within 7 days. You will receive updates in the
advisory thread as the report is triaged, fixed, and released. Please
allow up to 90 days for a coordinated fix before any public disclosure;
credit is given in the advisory unless you ask otherwise.

## Scope

In scope, for example:

- Vikunja API token leakage through logs, error messages, or tool output.
- Authentication or host-validation bypass of the HTTP transport
  (`MCP_HTTP_AUTH_TOKEN`, allowed-hosts / DNS-rebinding protections).
- Path traversal or arbitrary file read/write through attachment
  upload/download paths.
- Server-side request forgery or request smuggling through the Vikunja
  client.
- Memory-safety issues or panics reachable from untrusted MCP input or
  Vikunja API responses.

Out of scope:

- Vulnerabilities in Vikunja itself — report those to the
  [Vikunja project](https://vikunja.io).
- Vulnerabilities in third-party dependencies without a demonstrated
  impact on this server (dependency advisories are tracked via Dependabot
  and `cargo audit`).
- Deployments that disable the documented protections, e.g.
  `--http-allow-unauthenticated` without an authenticating reverse proxy.
- Denial of service requiring authenticated access with valid tokens.

## Hardening Guidance

See the README sections on HTTP transport authentication, allowed hosts,
and API token scopes for deployment recommendations. Releases include a
CycloneDX SBOM (`vikunja-rust-mcp-sbom.cdx.json`) for dependency auditing.
