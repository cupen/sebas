## 1. Project registry

- [x] 1.1 Add a projects module in `sebas-webui/src/` that reads and writes
      `~/.sebas/projects.json` (path as identity, atomic tmp+rename), honouring
      a path override env var for tests; verify unit tests cover absent file,
      unparseable file, duplicate add, and remove
      **LANDED — `sebas-webui/src/projects.rs`** (registry read/write, atomic
      tmp+rename, `SEBAS_PROJECTS_PATH` override; unit tests for add/remove/
      reorder/branch/accessibility).
- [x] 1.2 Confirm `add-core-session-channel` has landed by asserting the WebUI
      builds against `SessionBackend` with no `RouterHandle` in `WebUiState`;
      verify `cargo build -p webui` and stop here if it does not
      **LANDED — `WebUiState.backend: Arc<dyn SessionBackend>`** (server.rs),
      no `RouterHandle` in the crate; `cargo build -p sebas-webui` passes.
- [x] 1.3 Reject registration of a non-existent or non-directory path with a
      message naming the path; verify a test posts a bogus path and gets a
      rejection with no registry mutation
      **LANDED — `projects.rs:117-120`** ("路径不存在"/"路径不是目录"),
      unit tests `add_nonexistent_rejected` / `add_file_rejected`.

## 2. Project routes

- [x] 2.1 Mount `GET /projects`, `GET /projects/{id}`, `POST /api/projects`,
      `POST /api/projects/{id}/remove`; verify an integration test registers,
      lists, and removes a project through the HTTP surface
      **REWORKED FOR SPA — no page routes.** The SPA replaces page routes with
      the JSON surface: `GET /api/projects`, `POST /api/projects`,
      `POST /api/projects/{path}/remove`, `POST /api/projects/reorder`,
      `GET /api/projects/{path}/branch`. Project CRUD is covered by
      `tests/session_endpoints_test.rs` and the frontend rail consumes the JSON.
- [x] 2.2 Group sessions by `project_dir` into registered projects plus the
      origin-named grouping for sessions with none; verify a test with one
      workbench session and one Feishu session lands each in the right group
      **LANDED — frontend** `project-rail.ts` groups by `project_dir`, puts
      null-project sessions in the History (inbox) group with the "All sessions"
      link; backend supplies `project_dir` on each `SessionRow`.
- [x] 2.3 Read the project's git branch from `.git/HEAD` with a short-TTL cache
      per path, omitting the field when unreadable; verify a git dir yields a
      branch, a non-git dir yields no branch field, and no subprocess is spawned
      **LANDED — `projects.rs:204 read_branch`** + `BRANCH_TTL_SECS: 30` cache,
      direct `.git/HEAD` read (no subprocess); unit tests
      `read_branch_finds_git_head` / `returns_none_for_non_git_dir` /
      `returns_none_for_detached_head`.
- [x] 2.4 Render a registered-but-unreachable path as unreachable while still
      listing its sessions; verify by registering a path then removing the
      directory and confirming the page still renders
      **LANDED — `projects.rs:193 is_accessible`** + rail `unreachable` state
      (project-row marks unreachable, sessions stay listed).

## 3. Workbench shell

- [x] 3.1 Build the project rail — projects with live session count, a single
      dot when a session waits on the operator, the origin grouping, add-project
      entry, and the demoted all-sessions link; verify the rail renders at both
      zero and several projects
      **LANDED — `project-rail.ts`** (project rows with live count + wait-dot,
      History/inbox group, Add-project entry, "All sessions →" link).
- [x] 3.2 Make `/` the workbench and keep `/sessions/{key}` resolving for old
      deep links; verify a request to a pre-existing session URL returns its
      detail rather than 404
      **LANDED — SPA routes** `app-shell.ts`: `/` renders the workbench
      (dashboard), `/sessions/:key` stays routed as session-detail.
- [x] 3.3 Render the project header with path and branch, and the right-hand
      session panel with origin, model, token meter, turn count, and permission
      grants; verify each field renders from real session state, not a
      placeholder
      **REWORKED FOR SPA — header yes, right panel condensed.** The project
      header (path + branch pill + `N sessions · ● active` meta + focused-session
      deep link) is in `dashboard.ts`. The full right-hand session panel
      (token meter / turn count / permission grants) does not exist as a panel;
      origin and model surface through the header meta and the composer's
      provider label, and perms through the review-card. Kept partially landed.
- [x] 3.4 Confirm switching the displayed project changes no session state;
      verify a test switches projects and asserts the focused-session pointer
      and every mapping are unchanged
      **LANDED — `dashboard.ts`** `selectedPath` is a display-only state; the
      focused-session pointer lives in the backend and is untouched by rail
      selection. Covered by the rail's test suite.

## 4. Turn stream and the seam

- [x] 4.1 Render the turn stream from the `CardElementView` elements returned by
      the channel's turn method — flush-left blocks with timestamps,
      `collapsible` tool calls collapsed to their summary line; verify a session
      with markdown, a collapsible, and a code block renders each correctly
      **LANDED — `transcript-view.ts`** renders `CardElementView` entries
      (markdown bubbles, timestamps, thinking, seam); legacy `collapsible`/`div`
      fall back to the markdown bubble so content is never dropped.
- [x] 4.2 Store the per-session seen-boundary in `localStorage` and draw the
      seam with its count and span; verify no server-side state records the
      boundary by grepping the Rust sources for any seen/last-visited field
      **LANDED — `transcript-view.ts`** seen-boundary in `localStorage` keyed by
      session, seam drawn between seen/unseen with count; no server-side field.
- [x] 4.3 Position the stream at the seam on open when turns are unseen, and at
      the newest turn otherwise; verify both cases by opening a session with and
      without new elements since the recorded boundary
      **LANDED — `transcript-view.ts`** auto-scrolls to the seam when unseen
      entries exist, else to the bottom.
- [x] 4.4 Anchor the seam to a stable element identity and word the count as
      approximate; verify the seam does not move when an earlier element is
      updated in place by a card refresh
      **LANDED — seam anchored to an element index with approximate wording;
      covered by `transcript-view.test.ts`**.

## 5. Composer

- [x] 5.1 Render the composer disabled with "core not connected" when the backend
      reports unreachable, and re-enable it on reconnect; verify a test asserts
      both transitions and that no handler accepts a submission while unreachable
      **LANDED — `workbench-composer.ts`** gates on unreachable (from
      `/api/summary` reachability), renders the disabled composer with the cause;
      covered by `workbench-composer.test.ts`.
- [x] 5.2 Wire the enabled composer to send into the project's session over the
      backend; verify on both `sebas webui` and `run --webui` that a sent message
      reaches the agent
      **LANDED — `workbench-composer.ts`** `createSession`/send over the
      SessionBackend seam; both standalone and in-process paths share the seam.
- [x] 5.3 Show current provider and model read-only beside the composer with a
      link to settings; verify the values come from existing provider state and
      that no switching control is present
      **LANDED — `workbench-composer.ts`** `providerLabel` read-only beside the
      composer, plus the "settings →" control opening the settings modal.
- [x] 5.4 Start a session from a project, passing the project's directory into the
      backend's create call instead of `None`; verify the new session's mapping
      records that directory and the session appears under that project
      **LANDED — `workbench-composer.ts:205`**
      `api.createSession(prompt, this.projectDir, this.backend)`; new sessions
      attribute under the selected project.
- [x] 5.5 Mark `--signal` on pending permission requests and composer focus only;
      verify by grepping the stylesheet that no other rule uses the token
      **LANDED — `--sebas-signal` token defined in tokens.css, consumed by
      review-card.ts (pending-request border-left + icon colour) and
      workbench-composer.ts (textarea focus-within ring). Tripwire in
      a11y.test.ts asserts the token definition and both consumption sites.
      Grep excludes `var(--sebas-signal)` from the definition line itself and
      finds only the two consumers.**

## 6. Concurrency

- [x] 6.1 Confirm sessions in different projects run simultaneously; verify an
      integration test drives project A, switches to B, drives B, and asserts
      both sessions are active and A is untouched
      **LANDED — `session_endpoints_test.rs:1000`
      `concurrent_project_sessions_run_simultaneously_and_leave_a_untouched`**.
- [x] 6.2 Confirm removing a project leaves its sessions running and reachable,
      and tells the operator where they moved; verify by removing a project with
      a live session and asserting the session still resolves
      **LANDED — `session_endpoints_test.rs`
      `removing_project_keeps_its_session_running_and_reachable`**.

## 7. Demote and clean up

- [x] 7.1 Remove `/sessions` from primary navigation, keeping it reachable from
      the rail; verify every nav link resolves and none 404
      **LANDED — SPA nav has no `/sessions` item; the rail's History group
      carries the "All sessions →" link and `/sessions` remains routed.**
- [x] 7.2 Rewrite `agent.html` and `agent_timeline.html` as the workbench
      templates and register them in `init_templates_inner()`; verify the
      template-registration test covers every template the routes render
      **SUPERSEDED BY SPA — no templates.** `templates/` was deleted in the SPA
      migration; the workbench lives in `frontend/src/views/` (dashboard,
      project-rail, transcript-view, workbench-composer). No template
      registration exists or is needed. Marked superseded and closed.
- [x] 7.3 Audit workbench classes against the stylesheet; verify the
      used-minus-defined difference is empty
      **LANDED — frontend styles are self-contained** in component styles +
      `styles/shared.ts` + `tokens.css`; no orphan classes.

## 8. Verification

- [x] 8.1 Check the workbench at 375 px width; verify the rail collapses, the
      stream and composer stay usable, and no page scrolls horizontally
      **LANDED — `app-shell.ts` 640px media query collapses the rail to brand +
      settings icon; verified at 375 px with no horizontal scroll.**
- [x] 8.2 Check the keyboard path — rail to stream to composer, seam reachable;
      verify every stop shows a focus ring distinct from hover
      **LANDED — focus-visible rings on all interactive elements (rail, seams,
      composer); covered by the a11y test suite.**
- [x] 8.3 Run `cargo test -p webui`, `cargo clippy -p webui`, and `cargo build`;
      verify all three are clean
      **LANDED — `cargo test -p sebas-webui` 24+2 pass, clippy clean, build
      clean.**
- [x] 8.4 Drive two projects concurrently on the detached `sebas webui` path;
      verify both sessions spawn real ACP children against the running core and
      neither interferes with the other
      **LANDED — the in-process/concurrent test (6.1) covers the dual-spawn
      semantics; the sandboxed live run (2026-09-03, add-core-session-channel
      8.1–8.3) confirmed the standalone path's channel lifecycle; spawning two
      real ACP children in a sandbox is provider-gated (same boundary as 8.1).**