## 1. Project registry

- [ ] 1.1 Add a projects module in `webui/src/` that reads and writes
      `~/.sebas/projects.json` (path as identity, atomic tmp+rename), honouring
      a path override env var for tests; verify unit tests cover absent file,
      unparseable file, duplicate add, and remove
- [ ] 1.2 Confirm `add-core-session-channel` has landed by asserting the WebUI
      builds against `SessionBackend` with no `RouterHandle` in `WebUiState`;
      verify `cargo build -p webui` and stop here if it does not
- [ ] 1.3 Reject registration of a non-existent or non-directory path with a
      message naming the path; verify a test posts a bogus path and gets a
      rejection with no registry mutation

## 2. Project routes

- [ ] 2.1 Mount `GET /projects`, `GET /projects/{id}`, `POST /api/projects`,
      `POST /api/projects/{id}/remove`; verify an integration test registers,
      lists, and removes a project through the HTTP surface
- [ ] 2.2 Group sessions by `project_dir` into registered projects plus the
      origin-named grouping for sessions with none; verify a test with one
      workbench session and one Feishu session lands each in the right group
- [ ] 2.3 Read the project's git branch from `.git/HEAD` with a short-TTL cache
      per path, omitting the field when unreadable; verify a git dir yields a
      branch, a non-git dir yields no branch field, and no subprocess is spawned
- [ ] 2.4 Render a registered-but-unreachable path as unreachable while still
      listing its sessions; verify by registering a path then removing the
      directory and confirming the page still renders

## 3. Workbench shell

- [ ] 3.1 Build the project rail — projects with live session count, a single
      dot when a session waits on the operator, the origin grouping, add-project
      entry, and the demoted all-sessions link; verify the rail renders at both
      zero and several projects
- [ ] 3.2 Make `/` the workbench and keep `/sessions/{key}` resolving for old
      deep links; verify a request to a pre-existing session URL returns its
      detail rather than 404
- [ ] 3.3 Render the project header with path and branch, and the right-hand
      session panel with origin, model, token meter, turn count, and permission
      grants; verify each field renders from real session state, not a
      placeholder
- [ ] 3.4 Confirm switching the displayed project changes no session state;
      verify a test switches projects and asserts the focused-session pointer
      and every mapping are unchanged

## 4. Turn stream and the seam

- [ ] 4.1 Render the turn stream from the `CardElementView` elements returned by
      the channel's turn method — flush-left blocks with timestamps,
      `collapsible` tool calls collapsed to their summary line; verify a session
      with markdown, a collapsible, and a code block renders each correctly
- [ ] 4.2 Store the per-session seen-boundary in `localStorage` and draw the
      seam with its count and span; verify no server-side state records the
      boundary by grepping the Rust sources for any seen/last-visited field
- [ ] 4.3 Position the stream at the seam on open when turns are unseen, and at
      the newest turn otherwise; verify both cases by opening a session with and
      without new elements since the recorded boundary
- [ ] 4.4 Anchor the seam to a stable element identity and word the count as
      approximate; verify the seam does not move when an earlier element is
      updated in place by a card refresh

## 5. Composer

- [ ] 5.1 Render the composer disabled with "core not connected" when the backend
      reports unreachable, and re-enable it on reconnect; verify a test asserts
      both transitions and that no handler accepts a submission while unreachable
- [ ] 5.2 Wire the enabled composer to send into the project's session over the
      backend; verify on both `sebas webui` and `run --webui` that a sent message
      reaches the agent
- [ ] 5.3 Show current provider and model read-only beside the composer with a
      link to settings; verify the values come from existing provider state and
      that no switching control is present
- [ ] 5.4 Start a session from a project, passing the project's directory into the
      backend's create call instead of `None`; verify the new session's mapping
      records that directory and the session appears under that project
- [ ] 5.5 Mark `--signal` on pending permission requests and composer focus only;
      verify by grepping the stylesheet that no other rule uses the token

## 6. Concurrency

- [ ] 6.1 Confirm sessions in different projects run simultaneously; verify an
      integration test drives project A, switches to B, drives B, and asserts
      both sessions are active and A is untouched
- [ ] 6.2 Confirm removing a project leaves its sessions running and reachable,
      and tells the operator where they moved; verify by removing a project with
      a live session and asserting the session still resolves

## 7. Demote and clean up

- [ ] 7.1 Remove `/sessions` from primary navigation, keeping it reachable from
      the rail; verify every nav link resolves and none 404
- [ ] 7.2 Rewrite `agent.html` and `agent_timeline.html` as the workbench
      templates and register them in `init_templates_inner()`; verify the
      template-registration test covers every template the routes render
- [ ] 7.3 Audit workbench classes against the stylesheet; verify the
      used-minus-defined difference is empty

## 8. Verification

- [ ] 8.1 Check the workbench at 375 px width; verify the rail collapses, the
      stream and composer stay usable, and no page scrolls horizontally
- [ ] 8.2 Check the keyboard path — rail to stream to composer, seam reachable;
      verify every stop shows a focus ring distinct from hover
- [ ] 8.3 Run `cargo test -p webui`, `cargo clippy -p webui`, and `cargo build`;
      verify all three are clean
- [ ] 8.4 Drive two projects concurrently on the detached `sebas webui` path;
      verify both sessions spawn real ACP children against the running core and
      neither interferes with the other
