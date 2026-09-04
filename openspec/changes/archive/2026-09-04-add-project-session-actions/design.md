## Context

See `proposal.md` — Why. The existing groundwork:

- **Project registry exists** (`sebas-webui/src/projects.rs`) with `list`/`add`/`remove`/`reorder`, atomically persisted to `~/.sebas/projects.json`, git branch probe from `.git/HEAD` + 30 s TTL, and `is_accessible` check.
- **SessionBackend trait** (`session_backend.rs`) drives `spawn`/`message`/`close` for both detached and in-process paths. `spawn` currently requires a `prompt: String` — the caller must supply the first message.
- **`POST /api/sessions`** (`api.rs:212`) accepts `CreateSessionRequest { prompt, project_dir, backend }` with `prompt` as a required field.
- **project-rail.ts** uses `project_dir` matching to group sessions; sessions with `null` project_dir go into the History group. There is no archive concept.
- **`WebUiConfig`** (`server.rs` or `config.rs`) currently has no `archive_retention_days` field.
- **The workbench change is still unarchived** — its delta specs live in `openspec/changes/add-project-workbench/specs/`. The `webui` route-surface delta and `agent-workbench` delta there are the baseline we modify.

## Goals / Non-Goals

**Goals:**

- The "Add Project" `+` button opens a dialog with a directory browser, not a bare text input.
- The project row `+` button creates a 0-turn placeholder session (no prompt required).
- Every session row has an archive button; archived sessions go to History, read-only, with configurable retention.
- History group = archive only; Feishu inbox becomes "Inbox" group.
- Backward compatible: `POST /api/sessions` with `prompt` still works as before.

**Non-Goals:**

- No changes to the session channel protocol, router state, or Feishu bridge.
- No per-turn origin tracking (already out of scope in the workbench change).
- No file browser, diff review, or terminal inside a project.
- No archive UI in the session detail page (archive stays in the sidebar rail).
- No batch archive or multi-select.

## Decisions

### D1: Zero-prompt spawn via an optional `prompt` field

`POST /api/sessions` changes `prompt` from `String` to `Option<String>`. When `prompt` is `None`:

1. The backend calls `spawn` with an empty prompt `""` — the underlying `RouterHandle::web_spawn` already inserts a placeholder mapping and returns a key immediately.
2. The placeholder session appears in `snapshot` with status `Spawning` and zero turns.
3. The ACP child is not actually spawned until the first `POST /api/sessions/{key}/message` arrives.

**Why this approach over a separate "create empty session" API endpoint:** The `spawn` method already creates the mapping and returns a key. An empty prompt is just a mapping without a child — the same code path, one less API surface. The child is deferred because the in-process backend's `web_spawn` creates the ACP child inline; with an empty prompt, the router treates it as a pending session and the child is created on the first message. The detached backend (channel) already has a similar pattern: it creates the mapping locally and the child is on the core side.

### D2: Directory browser uses `GET /api/fs/browse`

A new endpoint at `GET /api/fs/browse?path=<path>` returns a JSON list of child entries (`{ name, is_dir }`) for the given path. The endpoint canonicalises the path and rejects navigation outside the operator's home directory (path traversal guard). The frontend dialog fetches entries on demand, showing a navigable tree.

**Why not `showDirectoryPicker`:** The browser API returns only the directory name, not the absolute path. The backend needs the real path to register it and to pass to `spawn` as `project_dir`.

**Why not shell out to `ls` or `find`:** Pure Rust `std::fs::read_dir` is already available, cross-platform, and needs no subprocess.

**Why home-directory scoped:** Safety — prevents the dialog from being used to explore the entire filesystem. The scope can be relaxed later if needed.

### D3: Archive is a WebUI-owned file (`~/.sebas/archive.json`)

Same pattern as `projects.json` — a separate file, written atomically (tmp + rename), owned by the WebUI process. Each entry stores:

```json
{
  "session_key": "sess_abc123",
  "project_path": "/home/user/work/sebas",
  "label": "Session 1",
  "archived_at": 1700000000,
  "retention_deadline": 1702592000
}
```

**Why not a field on the session row:** The core doesn't know about archive state — it's a WebUI-level concept. Adding it to the core's state would require channel protocol changes and cross-writer problems.

**Why not `localStorage`:** Archive state should survive browser clearing, device changes, and be visible to the WebUI process for expiry cleanup.

### D4: Archive expiry runs at startup and on list requests

On WebUI startup and on every `GET /api/archive` or `GET /api/sessions`, the archive module checks each entry's `retention_deadline` against the current time and removes expired entries. This is a simple synchronous check — no background task, no cron.

**Why not a background timer:** The archive is small (hundreds of entries at most). A synchronous check on every list request adds negligible latency. A background task adds lifecycle complexity for no benefit.

### D5: History/Inbox split in the sidebar

The current `project-rail.ts` `historySessions()` method filters sessions with `project_dir === null` into the "History" group. This changes to:

- **History group**: archived sessions from `archive.json`. Collapsible, shows count.
- **Inbox group**: sessions with `project_dir === null` (Feishu-originated). Collapsible, shows count.

The `section-label` for projects stays the same. The frontend fetches archived sessions from `GET /api/archive` and merges them into the sidebar state.

## Risks / Trade-offs

- **Risk**: Zero-prompt placeholder sessions may confuse the operator if they appear in the session list but have no content. **Mitigation**: The session row shows a placeholder label like "New session" and the turn stream shows the empty-state prompt ("Start a conversation").
- **Risk**: The directory browser exposes the filesystem (even if scoped to home). **Mitigation**: The endpoint canonicalises paths and rejects traversal. The frontend only shows directories, not files.
- **Risk**: Archive expiry permanently removes sessions. **Mitigation**: The retention period is configurable and defaults to 30 days. The archive list shows the deadline per entry.
- **Risk**: The workbench change is unarchived — its delta specs may conflict at archive time. **Mitigation**: This change's delta specs for `webui` and `agent-workbench` are additive to the workbench change's deltas. Both will be merged during archive.