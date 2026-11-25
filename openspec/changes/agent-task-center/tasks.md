# Tasks: Agent Task Center

## 1. Schema migration & beads client skeleton

- [x] 1.1 Add `task_id TEXT` column to `session_map` model in `sebas-models`, bump `sebas_db::schema` date version, and verify the migration runs against an existing database with `ALTER TABLE session_map ADD COLUMN task_id TEXT` (existing rows keep data, new column is NULL).
- [x] 1.2 Wire `task_id` through `SessionInfo` in `sebas-domain`, `sebas-ipc::protocol`, and the create-session RPC; verify round-trip via `cargo test -p sebas-ipc` and the IPC contract golden fixture (`tests/ipc_protocol_contract_test.rs`).
- [x] 1.3 Create `sebas-webui/src/beads_client.rs` exposing `async fn` wrappers around `bd` CLI (`create`, `show`, `list`, `update`, `claim`, `unclaim`, `assign`, `close`, `comment`, `comments`, `events_tail`, `init`, `config_set`) using `tokio::process::Command` with `--json`; verify each wrapper parses its stdout into a typed Rust struct (unit-test against fixture JSON captured in spike).
- [x] 1.4 Add `thiserror`-based `BeadsError` enum (not installed, non-zero exit, JSON parse failure, timeout); verify each variant maps to a sensible HTTP status code in `task_routes` (404/400/409/500/503).

## 2. Beads initialization & lazy bootstrap

- [x] 2.1 Implement `ensure_beads_initialized(project_dir)` in `beads_client`: detect `.beads/` absence, run `bd init --prefix <first 4 chars of basename>` + `bd config set events-journal true`; if `.beads/` exists, only ensure `events-journal = true`. Unit-test against a fixture project dir.
- [x] 2.2 Wire `ensure_beads_initialized` into the first hit of `GET /api/projects/{id}/tasks`; verify second hit does not re-run init (test by checking invocation count via a mock).
- [x] 2.3 Surface init failure as a structured 503 with the underlying `bd` stderr; verify the WebUI task panel renders the failure inline (manual browser check).

## 3. Task HTTP API

- [x] 3.1 Implement `GET /api/projects/{id}/tasks` (list, filterable by `?status=` / `?assignee=`) → calls `bd list --json`; verify returns full task list with assignee/status/labels/comments-count.
- [x] 3.2 Implement `POST /api/projects/{id}/tasks` (create) → calls `bd create --json` with title/description/optional assignee/optional reviewer (stored in `--metadata`); verify validation rejects unknown assignee/reviewer with 400.
- [x] 3.3 Implement `GET /api/projects/{id}/tasks/{task_id}` (show) → calls `bd show --include-comments --json`; verify returns full task including comments.
- [x] 3.4 Implement `POST /api/projects/{id}/tasks/{task_id}/claim` → calls `bd update --claim`; verify idempotent 200 for same actor, 409 with current holder name on race.
- [x] 3.5 Implement `POST /api/projects/{id}/tasks/{task_id}/release` → calls `bd unclaim`; verify task returns to `open`.
- [x] 3.6 Implement `POST /api/projects/{id}/tasks/{task_id}/assign` → calls `bd assign`; verify assignee vocabulary check rejects unknown principal with 400.
- [x] 3.7 Implement `POST /api/projects/{id}/tasks/{task_id}/review` → calls `bd update --add-label in-review`; verify the label appears in subsequent reads.
- [x] 3.8 Implement `POST /api/projects/{id}/tasks/{task_id}/close` with reviewer-gate: refuse 403 if caller is a non-reviewer agent kind; allow otherwise. Verify both branches.
- [x] 3.9 Implement `POST /api/projects/{id}/tasks/{task_id}/comment` → calls `bd comment`; verify comment appears with caller as author.
- [x] 3.10 Add integration test covering the full CRUD+claim+review+close flow against a sandbox project dir; verify each step's HTTP status and persisted beads state.

## 4. Real-time events pump

- [x] 4.1 Implement `task_events.rs`: a per-project tokio task that calls `bd events tail --since <last_seq>` every 300ms with a 200ms timeout, tracking `last_seq` and publishing new events to a `tokio::sync::broadcast` channel. Unit-test the loop with a fake `bd` binary that emits fixture events.
- [x] 4.2 Bridge the broadcast channel to WebSocket: add `task_updated` message type in `webui-ws-rpc`, carrying the full beads issue snapshot; verify a connected client receives one WS message per journal event.
- [x] 4.3 Add a manual "Refresh tasks" endpoint (`POST /api/projects/{id}/tasks/refresh`) that performs a full `bd list --json` reload and resets `last_seq` to the journal head; verify it works after a simulated `bd dolt pull` (manual `cp -r` of `.beads/` from another dir).
- [x] 4.4 Verify the polling loop survives transient `bd` failures (kill `bd` mid-poll): the loop logs and retries with the same `last_seq`, no events skipped once `bd` recovers.

## 5. Session-terminal hook

- [x] 5.1 Add a `on_session_terminal` hook in the session lifecycle manager that fires when a session reaches `Closed` or `Exited`; verify it does not fire for `Failed`/`Terminated`.
- [x] 5.2 Implement the hook body: if the session's `task_id` is set AND the session is the task's most-recently-created session AND the last completed turn has `stop_reason = end_turn` (not `cancelled`, not error), call `bd update <task_id> --add-label in-review`. Verify via integration test: a fake-claude turn that ends normally triggers the label.
- [x] 5.3 Verify the hook is fire-and-forget (async spawn, error logged not propagated) and does not block the session lifecycle main loop.

## 6. Session creation with task_id

- [x] 6.1 Wire `task_id` through `POST /api/sessions` request body → core create-session RPC → `session_map` row. Verify `task_id` round-trips and is queryable via `/api/sessions/{key}`.
- [x] 6.2 Add a `WHERE task_id = ?` lookup (`sebas-models::session_map::find_by_task_id`) returning all sessions for a given task, ordered `created_at DESC`; unit-test the query.

## 7. Frontend — tabs and board

- [x] 7.1 Add `Conversation` / `Tasks` tabs to the stage header in `sebas-webui/frontend`; verify per-project tab state persists across project switches (component test).
- [x] 7.2 Build a `<TaskBoard>` component: three columns (`Unassigned`, `In progress`, `In review`) + collapsed `Done / Cancelled` region, each column header showing count; verify renders correctly with fixture data.
- [x] 7.3 Build a `<TaskCard>` component: title (2-line truncation + tooltip), assignee avatar (human letter / agent icon / unassigned dashed), `agent` chip when applicable, comment count, session count, relative updated timestamp; verify visually with storybook/fixture.
- [x] 7.4 Wire drag-and-drop between columns → call the corresponding API endpoint; verify a drag from `Unassigned` to `In progress` triggers claim and the card moves on success (Playwright test).
- [x] 7.5 Wire WebSocket `task_updated` events → update local task state by replacing the issue snapshot (no `bd show` re-fetch); verify the board updates within ~500ms of a backend mutation (manual or Playwright assertion).

## 8. Frontend — task detail page

- [x] 8.1 Build `<TaskDetail>`: left main column (title, markdown description, comment stream) + right sidebar (metadata, action buttons, session list); verify layout matches GitHub issue convention.
- [x] 8.2 Wire action buttons (Claim / Release / Enter review / Mark done / Cancel) to their respective endpoints; verify conditional rendering by state and caller identity (component test with mocked API).
- [x] 8.3 Build `<CommentStream>`: list of comments with author avatar (human/agent convention), timestamp, markdown body; verify posting a comment refreshes the stream.
- [x] 8.4 Build `<LinkedSessionList>`: shows sessions from `find_by_task_id`, each row title/agent/phase/timestamp; verify clicking a row switches the stage to the conversation view focused on that session.
- [x] 8.5 Wire "Start a session for this task" button: opens the existing `<SessionCreationDialog>` retitled to "Advance task bd-XX: <title>", confirming calls `POST /api/sessions` with `task_id`; verify cancel creates nothing.

## 9. Frontend — session back-link

- [x] 9.1 Add a "Task bd-XX" back-link to the session detail header when `task_id` is set; verify clicking switches stage to `Tasks` tab and opens the task detail.
- [x] 9.2 Verify rail session rows do NOT display the task link (density preserved).

## 10. Integration & acceptance

- [x] 10.1 Add a process-level e2e test (`tests/testsuite_e2e_test.rs`) covering: project create → first task panel visit (auto-init) → task create → human claim → session create from task → fake-claude turn completes → task auto-advances to in-review → reviewer closes. Verify each step's observable state.
- [x] 10.2 Add an acceptance journey (`tests/acceptance/`) that runs the full board lifecycle including a two-actor claim race (one human, one agent); verify the loser sees 409 and the board shows the winner.
- [x] 10.3 Manual sandbox smoke: `invoke testsuite-webui-sandbox --auth`, log in as admin, register a project, open Tasks tab, create a task, claim it, open a session from it, send a message (fake-claude), watch the board auto-advance to in-review, close as reviewer. Verify sub-second board updates throughout.
- [x] 10.4 Update `AGENTS.md` with a short "Task center" section: how to install `bd` ≥ 1.3.4, sandbox recipe addition, pointer to spike report.

## 11. Documentation & handoff

- [x] 11.1 Update `config/config.toml.example` with `[service.webui] task_center_enabled = true` (default true) and document the `bd` CLI dependency.
- [x] 11.2 Add a "Troubleshooting" note: what to do when `bd events tail` reports schema drift, and the manual "Refresh tasks" button as the re-baseline affordance after `bd dolt pull`.
- [x] 11.3 Confirm no operator-facing path touches real `~/.sebas` or the operator's `bd` data: all sandbox runs use throwaway project dirs.
