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

## Release Artifacts

Tagged releases publish platform packages for Linux, macOS and Windows. Each
release also includes `vikunja-rust-mcp-sbom.cdx.json`, a CycloneDX 1.5 SBOM
generated from the locked Cargo dependency graph with all features and targets
included.

## Configuration

Configuration comes from CLI flags or environment variables (flags win):

| Environment variable | Flag | Required | Default | Description |
|---|---|---|---|---|
| `VIKUNJA_URL` | `--vikunja-url` | yes | – | Base URL of the instance, e.g. `https://try.vikunja.io`. A trailing `/` or `/api/v1` suffix is tolerated; sub-path installs (`https://host/vikunja`) work. |
| `VIKUNJA_API_TOKEN` | `--api-token` | yes | – | Vikunja API token. Sent as `Authorization: Bearer <token>`. |
| `MCP_TRANSPORT` | `--transport` | no | `stdio` | `stdio` or `http`. |
| `MCP_HTTP_BIND` | `--bind` | no | `127.0.0.1:8077` | Bind address for the HTTP transport. |
| `MCP_HTTP_ALLOWED_HOSTS` | `--allowed-hosts` | no | – | Extra `Host` header values to accept (comma separated). `localhost`, `127.0.0.1`, `::1` and the bind IP are always accepted. |
| `MCP_HTTP_AUTH_TOKEN` | `--http-auth-token` | for non-loopback HTTP | – | Bearer token clients must send (`Authorization: Bearer <token>`) to reach `/mcp`. Required when binding beyond loopback. |
| `MCP_HTTP_ALLOW_UNAUTHENTICATED` | `--http-allow-unauthenticated` | no | `false` | Explicitly serve `/mcp` without authentication on non-loopback binds (only behind an authenticating reverse proxy). |
| `VIKUNJA_TIMEOUT_SECS` | `--timeout-secs` | no | `30` | Per-request timeout against the Vikunja API. |
| `VIKUNJA_DEFAULT_PAGE_SIZE` | `--default-page-size` | no | `50` | `per_page` used when a tool call does not specify one (1–250; the Vikunja server also caps it). |
| `VIKUNJA_DATE_DEFAULT_TIME` | `--date-default-time` | no | `09:00` | Time of day (`HH:MM`) applied when a date shortcut resolves to a calendar day (see *Smart date shortcuts*). |
| `VIKUNJA_DATE_END_OF_DAY_TIME` | `--date-end-of-day-time` | no | `23:59` | Time of day (`HH:MM`) used by the `end of week` date shortcut. |

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
`GET /healthz` answers `ok` for liveness probes and never requires
authentication.

When binding to a **non-loopback** address the server refuses to start
unless you either set `MCP_HTTP_AUTH_TOKEN` (clients then must send
`Authorization: Bearer <token>` to `/mcp`) or explicitly pass
`--http-allow-unauthenticated` because an authenticating reverse proxy
fronts the server:

```bash
VIKUNJA_URL=https://try.vikunja.io \
VIKUNJA_API_TOKEN=tk_... \
MCP_HTTP_AUTH_TOKEN=$(openssl rand -hex 32) \
vikunja-rust-mcp --transport http --bind 0.0.0.0:8077 \
  --allowed-hosts mcp.example.com
```

Also pass the hostname clients will use via `--allowed-hosts` — requests
with other `Host` headers are rejected to prevent DNS-rebinding.

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
| `vikunja_tasks_create` | Create a task in a project (RFC 3339 dates or `*_shortcut` date shortcuts). |
| `vikunja_tasks_update` | Partially update a task (incl. moving projects; dates also via `*_shortcut`). |
| `vikunja_tasks_delete` | Delete a task. |
| `vikunja_tasks_complete` | Mark a task done. |
| `vikunja_tasks_reopen` | Mark a task not done. |
| `vikunja_tasks_assign` | Assign a user to a task. |
| `vikunja_tasks_unassign` | Remove a user from a task. |
| `vikunja_tasks_bulk_complete` | Mark several tasks done (explicit ids, per-task results). |
| `vikunja_tasks_bulk_reopen` | Mark several tasks not done (explicit ids, per-task results). |
| `vikunja_tasks_bulk_update` | Apply one partial update to several tasks (explicit ids, per-task results). |
| `vikunja_tasks_bulk_move` | Move several tasks to another project (explicit ids, per-task results). |
| `vikunja_tasks_bulk_assign` | Assign one user to several tasks (explicit ids, per-task results). |
| `vikunja_tasks_bulk_unassign` | Remove one user from several tasks (explicit ids, per-task results). |
| `vikunja_task_labels_bulk_add` | Add one label to several tasks (explicit ids, per-task results). |
| `vikunja_task_labels_bulk_remove` | Remove one label from several tasks (explicit ids, per-task results). |
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
| `vikunja_filters_list` | List saved filters (durable, named task queries). |
| `vikunja_filters_get` | Get one saved filter incl. its stored query. |
| `vikunja_filters_create` | Create a saved filter from a filter expression + sort order. |
| `vikunja_filters_update` | Partially update a saved filter (query merged field by field). |
| `vikunja_filters_delete` | Delete a saved filter (tasks are unaffected). |
| `vikunja_filters_tasks` | List the tasks a saved filter currently matches (paginated). |
| `vikunja_dates_resolve` | Preview how a date shortcut resolves to RFC 3339 (read-only, never calls Vikunja). |

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

### Smart date shortcuts

`vikunja_tasks_create`, `vikunja_tasks_update` and `vikunja_tasks_bulk_update`
accept optional `due_date_shortcut`, `start_date_shortcut` and
`end_date_shortcut` fields next to the RFC 3339 ones. RFC 3339 values keep
working exactly as before (`"due_date": "2026-07-01T12:00:00Z"`); a shortcut
is resolved to RFC 3339 on the server before anything is sent to Vikunja.
Providing both a date field and its shortcut in one call is rejected.

Supported expressions (case-insensitive; only this grammar, no free-form
natural language):

| Expression | Meaning |
|---|---|
| `today` / `tomorrow` / `yesterday` | That day at the default task time. |
| `in N days` / `in N weeks` / `in N months` | N (positive integer) from today; months are calendar-aware (Jan 31 + 1 month clamps to end of February). |
| `monday` … `sunday` | Next occurrence of that weekday — today counts only while the default time is still ahead. |
| `next monday` … `next sunday` | That weekday strictly after today (always 1–7 days out). |
| `end of week` | The upcoming Sunday at the end-of-day time. |
| `YYYY-MM-DD` | That date at the default task time. |
| `clear` / `none` / `unset` / `no due date` | Clear the date: updates send the zero date `0001-01-01T00:00:00Z`; creates simply omit the field. |

Timezone and time behavior is explicit and deterministic:

- Resolution uses the **server's local timezone** (the machine running this
  MCP server). The resolved RFC 3339 value carries the local UTC offset.
- Shortcuts resolve to the date at `VIKUNJA_DATE_DEFAULT_TIME` (default
  `09:00`); `end of week` uses `VIKUNJA_DATE_END_OF_DAY_TIME` (default
  `23:59`).
- `vikunja_dates_resolve` previews a resolution without writing: pass
  `expression` (plus optional `reference_time` to resolve against a fixed
  RFC 3339 instant, whose offset then defines the timezone) and get back
  `resolved`, `clears_date`, `timezone_description` and `default_time_used`.

Examples: `{"due_date_shortcut": "next friday"}` resolves to the coming
Friday at 09:00 server time; `{"due_date_shortcut": "clear"}` on update
clears the due date; `{"due_date": "2026-07-01T12:00:00Z"}` still sets an
exact timestamp.

### Bulk semantics (explicit ids, partial failure)

The `*_bulk_*` tools are deliberately conservative:

- They require an **explicit list of task ids** — there is no filter-based
  bulk write, so a tool call can never touch more tasks than it names.
- Batches are capped at **100 task ids per call**; larger lists are rejected
  up front so one MCP call cannot flood the Vikunja instance. Split bigger
  jobs into multiple calls.
- All arguments are validated up front (`task_ids` non-empty, positive and
  within the cap, `label_id`/`user_id`/`project_id` positive); nothing is
  written if validation fails.
- Each task is processed through the same safe per-task calls as the
  single-task tools (including read-merge-write for updates and moves), and
  writes are **never retried automatically**.
- Results are **per task**: after validation passes, one failing task does
  not fail the call. The result reports `ok` (true only when nothing
  failed), `total`/`succeeded`/`failed` counts, and a `results` entry per
  task with the updated task or a confirmation message on success, or a
  structured error (`kind`, HTTP status, Vikunja error code, message) on
  failure.

## Resources

| URI | Description |
|---|---|
| `vikunja://status` | Server name/version, configured instance URL, connectivity probe — never the token. |
| `vikunja://projects` | All projects (auto-paginated, capped at 10 pages). |
| `vikunja://tasks` | Tasks across projects (auto-paginated, capped at 10 pages). |
| `vikunja://tasks/today` | Open tasks due today (task view). |
| `vikunja://tasks/overdue` | Open tasks due before today (task view). |
| `vikunja://tasks/upcoming` | Open tasks due in the next 7 days (task view). |
| `vikunja://tasks/high-priority` | Open tasks with priority >= 3 (task view). |
| `vikunja://tasks/inbox` | Open tasks without a due date (task view). |
| `vikunja://tasks/recently-updated` | All tasks, most recently updated first (task view). |
| `vikunja://filters` | All saved filters, with their filter ids and pseudo-project ids. |
| `vikunja://projects/{id}` | One project (resource template). |
| `vikunja://tasks/{id}` | One task (resource template). |
| `vikunja://filters/{id}` | One saved filter incl. its stored query (resource template). |
| `vikunja://filters/{id}/tasks` | Tasks matching a saved filter (resource template, capped at 10 pages). |

### Task view resources

The `vikunja://tasks/<view>` resources are convenience read-only views over
Vikunja filter expressions, so agents can read common planning views without
constructing filter syntax themselves. Each view applies a fixed filter and
sort order through the regular `GET /tasks` endpoint:

| View | Filter | Sort |
|---|---|---|
| `today` | `done = false && due_date >= now/d && due_date < now/d+1d` | `due_date` asc |
| `overdue` | `done = false && due_date < now/d && due_date != null` | `due_date` asc |
| `upcoming` | `done = false && due_date >= now/d && due_date < now/d+7d` | `due_date` asc |
| `high-priority` | `done = false && priority >= 3` | `priority` desc |
| `inbox` | `done = false && due_date = null` | `updated` desc |
| `recently-updated` | *(none)* | `updated` desc |

Notes on the definitions:

- `now/d` is Vikunja date math for "start of today"; `due_date = null` /
  `due_date != null` is Vikunja (>= 1.0) filter syntax for tasks without /
  with a due date.
- `inbox` uses a conservative cross-instance definition of "unplanned":
  open tasks that have no due date, sorted by last update (descending).
- `upcoming` starts at today, so it never overlaps with `overdue` but does
  include everything in `today`.
- `recently-updated` intentionally applies no `done` filter — it is a recent
  activity feed including completed tasks.

Like the other list resources, each view is auto-paginated and capped at 10
pages. The JSON body embeds the view definition and pagination metadata:

```json
{
  "view": "today",
  "description": "Open tasks due today, sorted by due date (ascending).",
  "filter": "done = false && due_date >= now/d && due_date < now/d+1d",
  "sort_by": "due_date",
  "order_by": "asc",
  "page_cap": 10,
  "pages_read": 1,
  "truncated": false,
  "count": 2,
  "tasks": []
}
```

`truncated` is `true` when the page cap was hit while the server still
reported more pages. Every view can be replicated (and customized, e.g.
restricted to one project or paginated past the cap) by calling the
`vikunja_tasks_list` tool with the same `filter`, `sort_by` and `order_by`.

### Saved filters

Saved filters are durable, named task queries stored in Vikunja itself —
unlike the fixed task views above, users define and edit them. A few
Vikunja quirks this server hides:

- **Filters look like projects.** Vikunja has no `GET /filters` list
  endpoint; each saved filter instead appears in the project list as a
  pseudo-project with the negative id `-filter_id - 1` (filter 1 is
  project `-2`). `vikunja_filters_list` and `vikunja://filters` resolve
  that mapping and report both ids; the filter tools always take the
  positive `filter_id`. Expect these pseudo-projects to also show up in
  `vikunja_projects_list` output.
- **Task results are computed via `GET /tasks`.** `vikunja_filters_tasks`
  and `vikunja://filters/{id}/tasks` fetch the saved filter and evaluate
  its stored query through the regular task listing: the filter
  expression, `filter_timezone` and `filter_include_nulls` carry over
  directly, and the **first** stored `sort_by`/`order_by` pair is applied
  (the task listing takes one sort field). Secondary sort keys stored on
  the filter are not applied.
- **Updates merge field by field.** Changing only the filter expression
  keeps the stored sort order, timezone and null handling; top-level
  fields behave like every other update tool (read-merge-write).
- **Validation is syntactic.** Empty titles, empty filter expressions,
  unbalanced parentheses and unterminated quotes are rejected before any
  write; full filter grammar errors are reported by Vikunja itself and
  surface as validation errors.
- A saved filter's relative dates (e.g. `now/d`) are resolved using the
  filter's stored `filter_timezone`; if none is stored, the Vikunja
  server's timezone applies.

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
- The HTTP transport authenticates `/mcp` with a bearer token
  (`MCP_HTTP_AUTH_TOKEN`, compared in constant time) and refuses to start on
  non-loopback binds without one unless `--http-allow-unauthenticated` is
  passed for reverse-proxy deployments. The `Host` allow-list additionally
  guards against DNS-rebinding. Use TLS (via a reverse proxy) for any
  non-local deployment so the bearer token is not sent in cleartext.
- Attachment uploads are capped at 20 MiB and rejected before the file is
  read into memory; inline downloads are capped at 2 MiB and aborted without
  buffering; `save_path` downloads stream to disk.
- Bulk tools only operate on explicitly listed task ids (at most 100 per
  call) — filter-based bulk writes are not supported — and report partial
  failures per task instead of retrying or aborting the batch (see *Bulk
  semantics* above).

## Vikunja API capabilities intentionally omitted

- **Pre-1.0 instances:** Vikunja < 1.0 used `GET /tasks/all`; this server
  targets the current stable API (`GET /tasks`).
- **Kanban views/buckets, task relations, reminders as first-class tools,
  reactions, link/user shares, webhooks, notifications, migrations**: out
  of scope for the core resource set this server exposes.
  Reminders/relations still appear in task JSON where Vikunja returns them.
  (Saved filters *are* supported — see the `vikunja_filters_*` tools.)
- **Vikunja's native bulk endpoints** (`/tasks/bulk`, label/assignee bulk):
  not used. Bulk task operations *are* available as the `*_bulk_*` tools
  above, but they fan out over the same per-task endpoints as the
  single-task tools and require explicit task ids — Vikunja's filter-based
  bulk endpoints are intentionally avoided so a single call cannot perform
  unbounded writes.
- **Team create/update/delete and membership management:** only team
  listing (global and per-project) is exposed, per the intended tool surface.
- **Listing task assignees as a separate tool:** assignees are already
  included in `vikunja_tasks_get`; the client layer supports it for
  completeness.
- `vikunja_task_comments_update`, `vikunja_tasks_assign` and
  `vikunja_tasks_unassign` are small additions beyond the baseline tool list,
  implemented because the API supports them directly.
