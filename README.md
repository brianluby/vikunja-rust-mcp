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
| `VIKUNJA_ATTACHMENT_UPLOAD_ROOTS` | `--attachment-upload-root` | no | – | Directories attachment uploads may read `file_path` files from (repeat the flag or separate with commas). Unset = any server-local path (see *Security notes*). |
| `VIKUNJA_ATTACHMENT_DOWNLOAD_ROOTS` | `--attachment-download-root` | no | – | Directories attachment downloads may write `save_path` files to (repeat the flag or separate with commas). Unset = any server-local path (see *Security notes*). |

Configuration is validated at startup; a missing/invalid URL or token fails
fast with an actionable message, and configured attachment roots must exist
(a typo cannot silently disable the sandbox).

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
| `vikunja_projects_list` | List/search projects (paginated, `is_archived` filter, optional auto-pagination). |
| `vikunja_projects_get` | Get one project. |
| `vikunja_projects_create` | Create a project. |
| `vikunja_projects_update` | Partially update a project (incl. archive/unarchive). |
| `vikunja_projects_delete` | Delete a project and its tasks. |
| `vikunja_tasks_list` | List/search tasks; optional `project_id`, Vikunja `filter` expression, `sort_by`/`order_by`, auto-pagination. |
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
| `vikunja_labels_list` | List/search labels (paginated, optional auto-pagination). |
| `vikunja_labels_create` | Create a label. |
| `vikunja_labels_update` | Partially update a label. |
| `vikunja_labels_delete` | Delete a label. |
| `vikunja_task_labels_add` | Add a label to a task. |
| `vikunja_task_labels_remove` | Remove a label from a task. |
| `vikunja_task_relations_list` | List a task's relations grouped by kind (subtask, blocking, ...). |
| `vikunja_task_relations_create` | Relate two tasks (blocking, subtask, precedes, ...). |
| `vikunja_task_relations_delete` | Remove a relation between two tasks. |
| `vikunja_task_reminders_list` | List a task's reminders. |
| `vikunja_task_reminders_add` | Add one reminder (absolute time, date shortcut, or relative to a task date). |
| `vikunja_task_reminders_set` | Replace all reminders; an empty list clears them. |
| `vikunja_task_comments_list` | List a task's comments. |
| `vikunja_task_comments_create` | Comment on a task. |
| `vikunja_task_comments_update` | Edit a comment. |
| `vikunja_task_comments_delete` | Delete a comment. |
| `vikunja_task_attachments_list` | List a task's attachments (paginated, optional auto-pagination). |
| `vikunja_task_attachments_upload` | Upload an attachment (base64 content or a server-local file path). |
| `vikunja_task_attachments_download` | Download an attachment (inline base64 up to 2 MiB, or save to a server-local path). |
| `vikunja_task_attachments_delete` | Delete an attachment. |
| `vikunja_users_search` | Search users (for assignment). |
| `vikunja_teams_list` | List teams; with `project_id`, list the teams that can access that project including their permission level. Optional auto-pagination. |
| `vikunja_filters_list` | List saved filters (durable, named task queries). |
| `vikunja_filters_get` | Get one saved filter incl. its stored query. |
| `vikunja_filters_create` | Create a saved filter from a filter expression + sort order. |
| `vikunja_filters_update` | Partially update a saved filter (query merged field by field). |
| `vikunja_filters_delete` | Delete a saved filter (tasks are unaffected). |
| `vikunja_filters_tasks` | List the tasks a saved filter currently matches (paginated). |
| `vikunja_dates_resolve` | Preview how a date shortcut resolves to RFC 3339 (read-only, never calls Vikunja). |
| `vikunja_project_views_list` | List a project's views (list/gantt/table/kanban) with their ids. |
| `vikunja_buckets_list` | List a project's Kanban buckets (lanes) with names and the tasks in each (read-only). |
| `vikunja_export_tasks` | Export tasks to JSON, Markdown checklist or CSV (read-only, bounded, deterministic). |
| `vikunja_export_project` | Export one project's metadata and optionally its tasks to JSON/Markdown/CSV (read-only). |
| `vikunja_import_tasks_markdown` | Import tasks from a Markdown checklist (dry-run by default; explicit write mode). |
| `vikunja_import_tasks_csv` | Import tasks from a CSV backlog (dry-run by default; explicit write mode). |

List tools return `{ items..., "pagination": { page, per_page, total_pages,
result_count, has_more } }` built from Vikunja's `x-pagination-total-pages`
and `x-pagination-result-count` headers; pass `page`/`per_page` to walk
further pages. Task filters use [Vikunja filter syntax](https://vikunja.io/docs/filters),
e.g. `done = false && due_date < now/d+7d`.

### Auto-pagination (bounded list-all)

The paginated list tools — `vikunja_projects_list`, `vikunja_tasks_list`,
`vikunja_labels_list`, `vikunja_task_attachments_list` and
`vikunja_teams_list` (including its `project_id` project-teams form) — also
accept:

- `auto_paginate` (bool): fetch multiple pages starting at page 1 and return
  them as one result. Cannot be combined with `page` (use `per_page` to
  control the page size that is walked).
- `max_pages` (1–50, default **10**): how many pages may be fetched at most.
  Requires `auto_paginate: true`; values outside 1–50 are rejected before
  any request is made. There is no unbounded mode — the **absolute cap is 50
  pages** per call.

All other arguments (`search`, `filter`, `project_id`, `sort_by`,
`order_by`, `is_archived`, ...) apply unchanged to every fetched page.
Auto-paginated responses add an `auto_pagination` block and report the last
fetched page in `pagination`:

```json
{
  "tasks": [ ... ],
  "pagination": { "page": 3, "total_pages": 7, "has_more": true },
  "auto_pagination": { "pages_read": 3, "page_cap": 3, "truncated": true, "count": 150 }
}
```

`truncated: true` means the page cap was hit while the server still reported
more pages — the result is incomplete; re-run with a narrower filter or a
higher `max_pages`. It is only set when the server explicitly reported more
pages via its pagination headers; when a server sends no pagination headers,
the walk stops after the first page with `truncated: false`. Without
`auto_paginate`, responses keep their original one-page shape and contain no
`auto_pagination` block.

Not auto-paginated: `vikunja_task_comments_list` (Vikunja does not paginate
comments — it always returns the full list) and `vikunja_users_search` (the
users endpoint is a search, not a paginated listing).

### Update semantics (read-merge-write)

Vikunja's update endpoints **reset fields that are omitted** from the
payload. To make partial updates safe, this server first `GET`s the current
entity, overlays only the fields you provided, and writes the merged object
back. Fields you don't pass keep their values. To clear a date field, pass
the zero value `0001-01-01T00:00:00Z` explicitly.

### Task relations

Relations are **directional**: the `relation_kind` describes the link as
seen from `task_id`. For example
`{"task_id": 5, "other_task_id": 9, "relation_kind": "blocking"}` means
*task 5 blocks task 9* (Vikunja stores the inverse `blocked` relation on
task 9 automatically). Supported kinds: `subtask`, `parenttask`, `related`,
`duplicateof`, `duplicates`, `blocking`, `blocked`, `precedes`, `follows`,
`copiedfrom`, `copiedto`. Both task ids must be positive and distinct, and
the kind is validated against this list before any request is sent; creating
a relation that already exists or naming a missing task surfaces the Vikunja
error (HTTP status and error code) unchanged.

### Task reminders

Vikunja has no dedicated reminder endpoints: reminders live on the task and
are written by replacing the task's `reminders` array through the same safe
read-merge-write update as other task fields. The tools cover the full
lifecycle: `vikunja_task_reminders_list` reads them,
`vikunja_task_reminders_add` appends one, and `vikunja_task_reminders_set`
replaces the list (`"reminders": []` removes every reminder).

Each reminder is either **absolute** or **relative**:

- Absolute: `{"reminder": "2026-07-01T09:00:00Z"}`, or a date shortcut
  (`{"reminder_shortcut": "next friday"}`) resolved exactly like the task
  date shortcuts below. The `clear` shortcut is rejected here — clear
  reminders with `vikunja_task_reminders_set` and an empty list.
- Relative: `{"relative_to": "due_date", "relative_period_seconds": -3600}`
  fires one hour before the due date (negative = before, positive = after;
  anchors: `due_date`, `start_date`, `end_date`). Vikunja computes the
  absolute time itself and re-computes it when the anchor date moves.

Timezone assumptions: absolute RFC 3339 reminders are passed through to
Vikunja unchanged, offset included. Reminder shortcuts resolve in the
**server's local timezone** at `VIKUNJA_DATE_DEFAULT_TIME` (see *Smart date
shortcuts*). Relative reminders have no timezone of their own — they follow
the anchor date stored on the task.

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

### Import/export workflows

The export tools are **read-only** and produce deterministic documents for
migration and reporting; the import tools create tasks from simple backlog
files with a **dry-run preview by default**. Attachments and comments are
not part of either direction in this version.

#### Exporting

- `vikunja_export_tasks` — `{ format: json|markdown|csv, project_id?,
  filter?, search?, sort_by?, order_by?, max_pages? }`. Fetches up to
  `max_pages` pages (1–50, default **10**; never unbounded) and renders one
  document. The result carries `content`, `task_count` and an
  `auto_pagination` block; `truncated: true` means the cap was hit and the
  export is incomplete.
- `vikunja_export_project` — `{ project_id, include_tasks, format,
  max_pages? }`. Adds the project's metadata (id, identifier, title,
  description, archive state, parent). CSV has no place for project
  metadata, so `format: csv` requires `include_tasks: true`.

Determinism: tasks are sorted by **id ascending** unless you pass `sort_by`
(then the server order is kept); JSON field order and CSV column order are
fixed; unset dates (Vikunja's `0001-01-01T00:00:00Z`) are omitted in JSON
and empty in CSV; label titles and assignee usernames are sorted.

Formats:

- **json** — pretty-printed array of trimmed task objects (`id`,
  `identifier`, `title`, `description`, `done`, dates, `priority`,
  `percent_done`, `project_id`, `labels`, `assignees`). Project exports wrap
  it as `{ "project": {...}, "tasks": [...] }`.
- **markdown** — checklist: `- [ ] Title` / `- [x] Title`, description lines
  indented two spaces below their task. Other fields are not represented;
  use JSON/CSV for a complete export.
- **csv** — header
  `id,identifier,title,description,done,due_date,start_date,end_date,priority,percent_done,project_id,labels,assignees`;
  RFC 4180 escaping (fields with commas, quotes or newlines are quoted,
  quotes doubled), `\n` line endings, `labels`/`assignees` joined with `|`.

#### Importing

- `vikunja_import_tasks_markdown` — `{ project_id, markdown, dry_run? }`
- `vikunja_import_tasks_csv` — `{ project_id, csv, dry_run? }`

Both default to **`dry_run: true`**: every row is validated and the result
previews the exact create payloads without writing anything. Pass
`dry_run: false` to create tasks — write mode requires an explicit
`project_id` and only ever creates tasks in that project.

Markdown input:

```markdown
# Headings and blank lines are ignored

- [ ] Ship the release
  Description lines are indented
  and joined together.
- [x] Already done elsewhere
```

Tasks are `- [ ] Title` lines; indented lines below a task become its
description. The `[x]` marker is accepted but **imported tasks are always
created open** (Vikunja's create endpoint cannot create completed tasks) —
complete them afterwards with the bulk tools. Any other line is reported as
an invalid row.

CSV input requires a header; supported columns: `title` (required),
`description`, `due_date`, `priority`:

```csv
title,description,due_date,priority
Ship the release,"Multi-line, quoted",2026-07-01T12:00:00Z,3
Plan next sprint,,next friday,
```

`due_date` accepts RFC 3339 or the same [date shortcuts](#smart-date-shortcuts)
as the task tools; `priority` must be 0–5. Unknown or duplicate columns are
rejected — **labels are not imported** in this version, so a `labels` column
is an error rather than silently ignored.

Semantics and limits:

- All rows are validated first. In dry-run the result reports `ok`,
  `total`, `valid`, `invalid`, `would_create` and per-item previews
  (`proposed` is the exact create payload).
- In write mode, **nothing is written unless every row is valid** (valid
  rows are reported as `skipped` next to the `invalid` ones), so a fixed
  file can be re-imported without creating duplicates.
- Writes happen one task at a time with **per-item results** (`created` /
  `failed` with a structured error) and are never retried automatically.
- Caps: input up to **256 KiB** and **100 tasks** per call; larger imports
  must be split.

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
| `vikunja://projects/{id}/buckets` | Kanban buckets of a project's first kanban view, with their tasks (resource template). |
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

### Kanban buckets

Vikunja organizes Kanban boards as *project views*: every project has a set
of views (list, gantt, table, kanban), and the kanban view's lanes are
*buckets* (e.g. Backlog, Doing, Done). Bucket support here is **read-only**
and built for answering questions like "which tasks are in the Doing lane?"
by name instead of by hidden numeric ids:

- `vikunja_project_views_list` lists a project's views with their ids and
  kinds.
- `vikunja_buckets_list` (and the `vikunja://projects/{id}/buckets`
  resource) lists the buckets of a kanban view with each bucket's name,
  task-count, WIP `limit` (0 = none) and the tasks currently in it. Without
  `view_id`, the project's **first kanban view** is used; pass `view_id` for
  projects with several kanban views. When the view definition was fetched
  (auto-resolution), each bucket also reports `is_default_bucket` /
  `is_done_bucket`; with an explicit `view_id` those flags are unknown and
  omitted from the output.
- Task JSON includes `bucket_id` whenever Vikunja reports it (a `0`/absent
  value is omitted), so bucket ids can be resolved to names via the bucket
  listing. Existing task outputs are unchanged otherwise.

Limitations: moving tasks between buckets and creating/renaming/deleting
buckets are intentionally not exposed; the bucket listing returns one page
of tasks per bucket (raise `per_page` to see more).

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
  the server's privileges. For hosted or multi-user deployments, sandbox them
  with `--attachment-upload-root`/`--attachment-download-root`: paths are
  fully canonicalized before the check, so `..` traversal and symlinks cannot
  escape a configured root, writes through pre-existing symlinks are refused,
  and configured roots must exist at startup (fail closed). Rejections name
  the offending path but never enumerate the configured roots. With no roots
  configured, any server-local path is allowed (the historical behavior) —
  restrict the tools in your MCP client's permission settings if that is
  undesirable. Base64 uploads and inline (base64) downloads never touch the
  server's filesystem and are unaffected. Like all path-check sandboxes it
  cannot detect hard links that alias files outside a root, nor a symlink
  swapped in between the check and the write; both require an attacker who
  can already write inside the root, so dedicate the root directories to
  this server.
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
- **Reminders as first-class tools, reactions, link/user shares, webhooks,
  notifications, migrations**: out of scope for the core resource set this
  server exposes. Reminders still appear in task JSON where Vikunja returns
  them. (Saved filters, task relations and read-only Kanban buckets *are*
  supported — see the `vikunja_filters_*`, `vikunja_task_relations_*` and
  `vikunja_buckets_list` tools.)
- **Kanban bucket writes** (creating/renaming/deleting buckets, moving
  tasks between buckets): bucket support is deliberately read-only.
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
