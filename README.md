# vikunja-rust-mcp

A production-quality [Model Context Protocol](https://modelcontextprotocol.io) server for
[Vikunja](https://vikunja.io), written in Rust on top of the official
[`rmcp`](https://crates.io/crates/rmcp) SDK. It lets MCP clients (Claude
Desktop, Claude Code, and others) manage Vikunja projects, tasks, labels,
assignees, comments, attachments and teams through typed, schema-described
tools.

- **Transports:** stdio (local clients) and streamable HTTP (remote/hosted).
- **API client:** dedicated async Vikunja REST client (`reqwest` + `serde`)
  with pagination, structured errors and safe retries.
- **Targets Vikunja >= 1.0** (the API documented at
  `https://<your-instance>/api/v1/docs`).

## Requirements

- Rust 1.85+ (edition 2024) to build.
- A Vikunja instance (1.0 or later) and an **API token**: in Vikunja go to
  *Settings → API Tokens* and create a token with the scopes you want this
  server to use (projects, tasks, labels, task comments, attachments, users,
  teams). Operations whose scope is missing fail with a clear `403` error.

## Build

```bash
cargo build --release
# binary at target/release/vikunja-rust-mcp
```

## Configuration

Configuration comes from CLI flags or environment variables (flags win):

| Environment variable | Flag | Required | Default | Description |
|---|---|---|---|---|
| `VIKUNJA_URL` | `--vikunja-url` | yes | – | Base URL of the instance, e.g. `https://try.vikunja.io`. A trailing `/` or `/api/v1` suffix is tolerated; sub-path installs (`https://host/vikunja`) work. |
| `VIKUNJA_API_TOKEN` | `--api-token` | yes | – | Vikunja API token. Sent as `Authorization: Bearer <token>`. |
| `MCP_TRANSPORT` | `--transport` | no | `stdio` | `stdio` or `http`. |
| `MCP_HTTP_BIND` | `--bind` | no | `127.0.0.1:8077` | Bind address for the HTTP transport. |
| `MCP_HTTP_ALLOWED_HOSTS` | `--allowed-hosts` | no | – | Extra `Host` header values to accept (comma separated). `localhost`, `127.0.0.1`, `::1` and the bind IP are always accepted. |
| `VIKUNJA_TIMEOUT_SECS` | `--timeout-secs` | no | `30` | Per-request timeout against the Vikunja API. |
| `VIKUNJA_DEFAULT_PAGE_SIZE` | `--default-page-size` | no | `50` | `per_page` used when a tool call does not specify one (1–250; the Vikunja server also caps it). |

Configuration is validated at startup; a missing/invalid URL or token fails
fast with an actionable message.

### stdio: example MCP client configuration

For Claude Desktop (`claude_desktop_config.json`) or any stdio MCP client:

```json
{
  "mcpServers": {
    "vikunja": {
      "command": "/path/to/vikunja-rust-mcp",
      "args": [],
      "env": {
        "VIKUNJA_URL": "https://try.vikunja.io",
        "VIKUNJA_API_TOKEN": "tk_..."
      }
    }
  }
}
```

For Claude Code:

```bash
claude mcp add vikunja \
  --env VIKUNJA_URL=https://try.vikunja.io \
  --env VIKUNJA_API_TOKEN=tk_... \
  -- /path/to/vikunja-rust-mcp
```

### HTTP: example startup

```bash
VIKUNJA_URL=https://try.vikunja.io \
VIKUNJA_API_TOKEN=tk_... \
vikunja-rust-mcp --transport http --bind 127.0.0.1:8077
```

The MCP endpoint is `http://127.0.0.1:8077/mcp` (MCP streamable HTTP);
`GET /healthz` answers `ok` for liveness probes. When binding to a
non-loopback address, also pass the hostname clients will use, e.g.
`--allowed-hosts mcp.example.com` — requests with other `Host` headers are
rejected to prevent DNS-rebinding.

Logging goes to **stderr** (stdout carries the protocol in stdio mode) and is
controlled with `RUST_LOG`, e.g. `RUST_LOG=vikunja_rust_mcp=debug`.

## Tools

All tools return **structured JSON** (with a published output schema), use
numeric Vikunja ids, hex colors *without* `#`, and RFC 3339 timestamps
(`2026-07-01T12:00:00Z`).

| Tool | Description |
|---|---|
| `vikunja_projects_list` | List/search projects (paginated, `is_archived` filter). |
| `vikunja_projects_get` | Get one project. |
| `vikunja_projects_create` | Create a project. |
| `vikunja_projects_update` | Partially update a project (incl. archive/unarchive). |
| `vikunja_projects_delete` | Delete a project and its tasks. |
| `vikunja_tasks_list` | List/search tasks; optional `project_id`, Vikunja `filter` expression, `sort_by`/`order_by`. |
| `vikunja_tasks_get` | Get one task with labels and assignees. |
| `vikunja_tasks_create` | Create a task in a project. |
| `vikunja_tasks_update` | Partially update a task (incl. moving projects). |
| `vikunja_tasks_delete` | Delete a task. |
| `vikunja_tasks_complete` | Mark a task done. |
| `vikunja_tasks_reopen` | Mark a task not done. |
| `vikunja_tasks_assign` | Assign a user to a task. |
| `vikunja_tasks_unassign` | Remove a user from a task. |
| `vikunja_labels_list` | List/search labels (paginated). |
| `vikunja_labels_create` | Create a label. |
| `vikunja_labels_update` | Partially update a label. |
| `vikunja_labels_delete` | Delete a label. |
| `vikunja_task_labels_add` | Add a label to a task. |
| `vikunja_task_labels_remove` | Remove a label from a task. |
| `vikunja_task_comments_list` | List a task's comments. |
| `vikunja_task_comments_create` | Comment on a task. |
| `vikunja_task_comments_update` | Edit a comment. |
| `vikunja_task_comments_delete` | Delete a comment. |
| `vikunja_task_attachments_list` | List a task's attachments. |
| `vikunja_task_attachments_upload` | Upload an attachment (base64 content or a server-local file path). |
| `vikunja_task_attachments_download` | Download an attachment (inline base64 up to 2 MiB, or save to a server-local path). |
| `vikunja_task_attachments_delete` | Delete an attachment. |
| `vikunja_users_search` | Search users (for assignment). |
| `vikunja_teams_list` | List teams; with `project_id`, list the teams that can access that project including their permission level. |

List tools return `{ items..., "pagination": { page, per_page, total_pages,
result_count, has_more } }` built from Vikunja's `x-pagination-total-pages`
and `x-pagination-result-count` headers; pass `page`/`per_page` to walk
further pages. Task filters use [Vikunja filter syntax](https://vikunja.io/docs/filters),
e.g. `done = false && due_date < now/d+7d`.

### Update semantics (read-merge-write)

Vikunja's update endpoints **reset fields that are omitted** from the
payload. To make partial updates safe, this server first `GET`s the current
entity, overlays only the fields you provided, and writes the merged object
back. Fields you don't pass keep their values. To clear a date field, pass
the zero value `0001-01-01T00:00:00Z` explicitly.

## Resources

| URI | Description |
|---|---|
| `vikunja://status` | Server name/version, configured instance URL, connectivity probe — never the token. |
| `vikunja://projects` | All projects (auto-paginated, capped at 10 pages). |
| `vikunja://tasks` | Tasks across projects (auto-paginated, capped at 10 pages). |
| `vikunja://projects/{id}` | One project (resource template). |
| `vikunja://tasks/{id}` | One task (resource template). |

## Error handling

Vikunja errors are mapped to MCP errors carrying the HTTP status, the
endpoint category (e.g. `tasks.update`), and Vikunja's error code/message —
never tokens or headers. `404`/validation failures map to *invalid params*
(the model can correct itself); auth, rate-limit, network and server errors
map to *internal error* with guidance (e.g. "check `VIKUNJA_API_TOKEN`").
Idempotent `GET` requests are retried once on timeout/connection failure;
writes are never retried automatically.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

### Coverage

Coverage uses [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```bash
cargo install cargo-llvm-cov --locked
rustup component add llvm-tools
cargo llvm-cov --all-features --workspace --fail-under-lines 90
```

The suite currently reports **~97% line coverage**. Tests include unit tests
for config/error/pagination/models, [`wiremock`](https://crates.io/crates/wiremock)-mocked
HTTP tests of the API client (request building, pagination headers,
merge-update bodies, error mapping, retry behavior), end-to-end MCP tests
driving every tool through a real rmcp client over an in-memory transport,
and smoke tests of the compiled binary over both stdio and HTTP.

### Optional live integration tests

`tests/live_integration.rs` runs a full project/task/label/comment/attachment
lifecycle against a real instance **only** when both env vars are set, and is
skipped silently otherwise:

```bash
VIKUNJA_TEST_URL=https://vikunja.example.com \
VIKUNJA_TEST_API_TOKEN=tk_... \
cargo test --test live_integration -- --nocapture
```

Use a disposable account/instance: the test creates and then deletes real
entities (cleanup is best-effort).

## Security notes

- The API token is read from the environment or a CLI flag, held in a
  self-redacting wrapper, and attached only as a `Bearer` header that is
  marked *sensitive* in the HTTP client. It is never logged, never appears in
  `Debug` output, error messages, MCP results or the status resource, and a
  test asserts it cannot leak through CLI output.
- Do not commit `.env` files; `.gitignore` already excludes them. Prefer the
  environment over `--api-token` (flags can show up in `ps` output).
- `vikunja_task_attachments_upload`/`_download` accept `file_path`/`save_path`
  arguments that read/write files **on the machine running this server** with
  the server's privileges. If that is undesirable in your deployment, restrict
  these tools in your MCP client's permission settings.
- The HTTP transport has no built-in authentication; bind it to loopback or
  put it behind an authenticating reverse proxy. The `Host` allow-list guards
  against DNS-rebinding only.

## Vikunja API capabilities intentionally omitted

- **Pre-1.0 instances:** Vikunja < 1.0 used `GET /tasks/all`; this server
  targets the current stable API (`GET /tasks`).
- **Kanban views/buckets, saved filters, task relations, reminders as
  first-class tools, reactions, link/user shares, webhooks, notifications,
  migrations, bulk endpoints** (`/tasks/bulk`, label/assignee bulk): out of
  scope for the core resource set this server exposes. Reminders/relations
  still appear in task JSON where Vikunja returns them.
- **Team create/update/delete and membership management:** only team
  listing (global and per-project) is exposed, per the intended tool surface.
- **Listing task assignees as a separate tool:** assignees are already
  included in `vikunja_tasks_get`; the client layer supports it for
  completeness.
- `vikunja_task_comments_update`, `vikunja_tasks_assign` and
  `vikunja_tasks_unassign` are small additions beyond the baseline tool list,
  implemented because the API supports them directly.
