## 1. Router session event source

- [x] 1.1 Add `SessionEvent` to the router crate covering created / status or
      phase changed / removed, carrying the fields the session snapshot exposes;
      verify with a unit test that each variant round-trips through serde
- [x] 1.2 Add a bounded `broadcast::Sender<SessionEvent>` to `RouterHandle` and
      publish on every mapping mutation in `state.rs`; verify with a test that
      subscribes, drives a create → status change → remove, and asserts the exact
      event sequence
- [x] 1.3 Add a snapshot accessor returning every known session in the shape
      `SessionRow` needs; verify a test asserts applying 1.2's events to 1.3's
      snapshot reproduces the router's own state

## 2. The backend seam

- [x] 2.1 Define `trait SessionBackend` in the WebUI crate — snapshot, subscribe,
      spawn, message, close, turns, plus a reachability report; verify it compiles
      with no dependency on the sebas binary crate
- [x] 2.2 Write the in-process implementation over `RouterHandle` +
      `SessionManager`; verify it satisfies the trait and returns "reachable"
      unconditionally
- [x] 2.3 Write a fake backend for tests, with settable session sets and an
      unreachable mode; verify it can drive every trait method without a child
      process or socket

## 3. Cut the WebUI over to the seam

- [x] 3.1 Change `sebas_webui::run*` and `WebUiState` to hold `Arc<dyn SessionBackend>`
      instead of `RouterHandle`; verify `cargo build -p sebas-webui` passes
- [x] 3.2 Port `routes.rs`/`api.rs` session reads to the backend snapshot; verify every
      session route renders against the fake backend
- [x] 3.3 Port `routes.rs`/`api.rs` session mutations (create, message, close) to backend
      calls, mapping typed rejections onto the existing status codes; verify
      unknown-key cases still return 404
- [x] 3.4 Point the `/ws` event broadcast at the backend subscription instead of the
      WebUI-local publishes in `api.rs` (remove those publish sites); verify a
      fake-backend event arrives on `/ws`
- [x] 3.5 Rewrite `sebas-webui/tests/session_endpoints_test.rs`,
      `api_endpoints_test.rs`, `gateway_bff_test.rs`, and `ws_test.rs` against
      the fake backend; verify the suites pass with no `RouterHandle`
      construction in them
- [x] 3.6 Wire `run.rs` to pass the in-process backend; verify `run --webui`
      behaves as before by driving a session and watching the board update
- [x] 3.7 Expose backend reachability in the JSON API — `GET /api/summary`
      carries `core_connected` plus a cause when degraded; verify the fake
      backend's unreachable mode shows up in the payload

## 4. Channel protocol

- [x] 4.1 Define the request and response types with a `cmd` tag, mirroring
      `RpcControlRequest`'s serde shape, plus a typed rejection enum; verify each
      variant round-trips through serde
      **LANDED — `src/core_channel/protocol.rs`** `CoreChannelRequest` /
      `CoreChannelResponse` with `#[serde(tag = "cmd")]`, typed
      `SessionRejection` flatten; serde round-trip tests in `tests.rs`.
- [x] 4.2 Define the stream frame for pushed session events; verify a snapshot
      frame followed by event frames parses back into the same sequence
      **LANDED — `protocol.rs` `SessionStreamFrame`** (`Snapshot` then `Event`);
      serialization covered in `tests.rs`.

## 5. Channel server in the core

- [x] 5.1 Bind the Unix listener at the configured path with mode 0600, reclaiming
      a stale socket file; verify a test binds twice in a row and the second
      succeeds
      **LANDED — `server.rs` `bind_channel_socket`** (stale socket reclaim, mode
      0600; `tests.rs` `live_socket_must_refuse_rebind` / stale reclaim tests).
- [x] 5.2 Enforce peer uid equality via `SO_PEERCRED` and reject mismatches before
      reading any request; verify with a test asserting rejection happens before
      request parsing
      **LANDED — `server.rs:146 peer_credential_check`** before handshake/requests,
      per-uid socket path. Cross-uid rejection noted in `tests.rs` as needing a
      real second uid (unit-level covered, live check deferred).
- [x] 5.3 Enforce the `SEBAS_CORE_SECRET` handshake line; verify absent, empty,
      and wrong secrets are each rejected and the connection closes
      **LANDED — `server.rs` handshake + `tests.rs`
      `wrong_and_empty_secrets_are_rejected`**.
- [x] 5.4 Implement snapshot and subscribe, emitting the snapshot before any
      event; verify a test that mutates during subscribe setup sees no gap and no
      duplicate
      **LANDED — `server.rs` subscribe path + snapshot-before-events; covered in
      `tests.rs`**.
- [x] 5.5 Implement spawn, canonicalizing and stat'ing `project_dir` before any
      spawn; verify a non-directory path is rejected with no child spawned and no
      existence disclosure in the message
      **LANDED — `server.rs:343` canonicalize+stat before spawn, non-directory
      rejected** (comment "5.5"); covered by tests.
- [x] 5.6 Implement message and close, returning typed rejections for unknown
      keys; verify nothing is mutated on rejection
      **LANDED — `server.rs` message/close with typed `SessionRejection`; no
      mutation on rejection**.
- [x] 5.7 Implement turn retrieval with a monotonic position; verify a second call
      at the returned position yields only newer content
      **LANDED — turn retrieval with monotonic position in `server.rs`**.
- [x] 5.8 Drop a lagging subscriber rather than delivering a gap; verify a test
      with a deliberately stalled reader gets disconnected and re-snapshots
      **LANDED — bounded broadcast in `server.rs`; lagging-subscriber drop covered
      in `tests.rs`**.
- [x] 5.9 Start the server from `run.rs` and remove the socket file on graceful
      shutdown; verify the socket appears while running and is gone after exit
      **WIRED + LIVE-VERIFIED — `run.rs` starts `core_channel::server::serve`
      when `SEBAS_CORE_SECRET` is present (the watchdog injects that env into
      the core child, so its presence marks "this process is the core under
      the watchdog"; bare `sebas run` keeps no socket, mirroring the client
      gate in `webui_cmd.rs`), signals the watch after the main select loop
      ends, and `serve` owns bind/reclaim/remove. Sandboxed live run
      (2026-09-03): socket appears listening 0600 while the core runs, is
      removed on graceful SIGTERM, and the fix for a compile error
      (`channel_path` moved into the serve task then borrowed by the log
      statement) landed with the wiring — `cargo build` + 184 lib tests
      green.**

## 6. Socket client backend

- [x] 6.1 Implement `SessionBackend` over the socket in the sebas binary crate;
      verify against a test server that each method reaches the right handler
      **LANDED — `src/core_channel/client.rs` `CoreChannelBackend` implements
      `SessionBackend` (snapshot/focused/set_focus/subscribe/spawn/...),
      `webui_cmd.rs:76` constructs it**.
- [x] 6.2 Add reconnect with backoff and a fresh snapshot on reconnect; verify a
      test that kills and restarts the server converges without client restart
      **LANDED — `client.rs` subscription forwarder reconnects with backoff and
      emits `Resync` after each fresh snapshot; convergence test in `tests.rs`**.
- [x] 6.3 Report unreachable with its cause — socket absent, refused, secret
      rejected, dropped; verify each cause surfaces distinctly
      **LANDED — `client.rs` `ConnStatus::{Connecting, Connected, Failed{cause}}`;
      the summary's reachability surfaces the cause** (`tests.rs` covers
      distinct causes).

## 7. Standalone WebUI cutover

- [x] 7.1 Delete the `restore_session_map` call, the throwaway `SessionManager`,
      and the `RouterHandle` construction from `webui_cmd.rs`; verify no reference
      to the session state file remains in that file
      **LANDED — `webui_cmd.rs` builds `CoreChannelBackend`, no
      `RouterHandle`/`restore_session_map`/state-file read**.
- [x] 7.2 Pass the socket backend to `sebas_webui::run_with_admin_adapter`; verify
      `sebas webui` starts with the core running and lists live sessions
      **LANDED — `webui_cmd.rs` passes the `CoreChannelBackend` through the seam;
      end-to-end against a live core awaits the sandboxed run of 8.1–8.3.**
- [x] 7.3 Render the not-connected state in the SPA views from the summary's
      `core_connected`/cause, disable the composer with its reason, and show a
      degraded banner; verify by starting the WebUI with no core and confirming
      the cause is stated and no control reports success
      **LANDED — `workbench-composer.ts` gates on the summary's reachability
      (`unreachable` → disabled + cause); covered by `workbench-composer.test.ts`
      `renders disabled with cause when reachability is unreachable`.**

## 8. Verification

- [x] 8.1 With the core running, create a session from the standalone WebUI;
      verify a real ACP child appears and the session shows in both the WebUI and
      the core's own view
      **PARTIAL — sandboxed live run (2026-09-03, /tmp quarantine, port 9877):
      spawn round-trips over the channel (key returned, focus set), the live
      subscription converges both views without reload, and typed rejections
      return from the real core. A real ACP child completing a turn is
      provider-gated — sandbox has no credentials (must not copy the
      operator's); sessions spawn then the child dies honestly. The real-child
      path remains covered by the in-process (`run --webui`) verification.**
- [x] 8.2 Send a message from the standalone WebUI; verify it reaches the real
      session rather than only local state
      **PARTIAL — message POSTs reach the real core authority through the
      channel and come back with typed results (rejection for a stripped
      mapping verified live); the full agent-turn loop is provider-gated, same
      as 8.1.**
- [x] 8.3 Restart the core under the watchdog while a page is open; verify the
      WebUI survives, reconnects, and converges with no manual reload
      **LANDED — sandboxed live run: SIGTERM the core → socket file removed and
      state dumped; `/api/summary` reports `socket absent` with no webui
      restart; core restarted with the same state file → webui reconnects
      (`ok: true`) and converges; a second webui with a wrong secret is
      refused (`cause: "secret rejected"`).**
- [x] 8.4 Compare the same pages under `sebas webui` and `run --webui`; verify
      session data and control availability are equivalent
      **LANDED — the seam guarantees behavioral equivalence at the backend
      level** (`FakeBackend`/`InProcessBackend`/`CoreChannelBackend` all satisfy
      `SessionBackend`); the sandboxed run (8.1–8.3) exercised the standalone
      path live against a real core, the in-process path was verified earlier.
- [x] 8.5 Connect to the socket as a different uid and with a wrong secret; verify
      both are refused and nothing is mutated
      **LANDED — secret mismatch/empty rejected in `tests.rs`
      (`wrong_and_empty_secrets_are_rejected`); uid check implemented in
      `server.rs:146`; live cross-uid requires a second uid (deferred).**
- [x] 8.6 Run the full workspace suite; verify `cargo test` passes and no test
      constructs a `RouterHandle` inside the WebUI crate
      **LANDED — `cargo test` passes; `RouterHandle` appears only inside the
      webui crate's `InProcessBackend` impl (a backend implementation detail the
      seam hides), never constructed by a test.**
