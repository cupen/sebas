## 1. Router session event source

- [ ] 1.1 Add `SessionEvent` to the router crate covering created / status or
      phase changed / removed, carrying the fields the session snapshot exposes;
      verify with a unit test that each variant round-trips through serde
- [ ] 1.2 Add a bounded `broadcast::Sender<SessionEvent>` to `RouterHandle` and
      publish on every mapping mutation in `state.rs`; verify with a test that
      subscribes, drives a create → status change → remove, and asserts the exact
      event sequence
- [ ] 1.3 Add a snapshot accessor returning every known session in the shape
      `SessionRow` needs; verify a test asserts applying 1.2's events to 1.3's
      snapshot reproduces the router's own state

## 2. The backend seam

- [ ] 2.1 Define `trait SessionBackend` in the WebUI crate — snapshot, subscribe,
      spawn, message, close, turns, plus a reachability report; verify it compiles
      with no dependency on the sebas binary crate
- [ ] 2.2 Write the in-process implementation over `RouterHandle` +
      `SessionManager`; verify it satisfies the trait and returns "reachable"
      unconditionally
- [ ] 2.3 Write a fake backend for tests, with settable session sets and an
      unreachable mode; verify it can drive every trait method without a child
      process or socket

## 3. Cut the WebUI over to the seam

- [ ] 3.1 Change `webui::run*` and `WebUiState` to hold `Arc<dyn SessionBackend>`
      instead of `RouterHandle`; verify `cargo build -p webui` passes
- [ ] 3.2 Port `routes.rs` session reads to the backend snapshot; verify every
      session route renders against the fake backend
- [ ] 3.3 Port `routes.rs` session mutations (create, message, close) to backend
      calls, mapping typed rejections onto the existing status codes; verify
      unknown-key cases still return 404
- [ ] 3.4 Point `sse.rs` at the backend subscription instead of the WebUI-local
      broadcast; verify a fake-backend event appears on `/events`
- [ ] 3.5 Rewrite `webui/tests/session_endpoints_test.rs` against the fake
      backend; verify the suite passes with no `RouterHandle` construction in it
- [ ] 3.6 Wire `run.rs` to pass the in-process backend; verify `run --webui`
      behaves as before by driving a session and watching the board update

## 4. Channel protocol

- [ ] 4.1 Define the request and response types with a `cmd` tag, mirroring
      `RpcControlRequest`'s serde shape, plus a typed rejection enum; verify each
      variant round-trips through serde
- [ ] 4.2 Define the stream frame for pushed session events; verify a snapshot
      frame followed by event frames parses back into the same sequence

## 5. Channel server in the core

- [ ] 5.1 Bind the Unix listener at the configured path with mode 0600, reclaiming
      a stale socket file; verify a test binds twice in a row and the second
      succeeds
- [ ] 5.2 Enforce peer uid equality via `SO_PEERCRED` and reject mismatches before
      reading any request; verify with a test asserting rejection happens before
      request parsing
- [ ] 5.3 Enforce the `SEBAS_CORE_SECRET` handshake line; verify absent, empty,
      and wrong secrets are each rejected and the connection closes
- [ ] 5.4 Implement snapshot and subscribe, emitting the snapshot before any
      event; verify a test that mutates during subscribe setup sees no gap and no
      duplicate
- [ ] 5.5 Implement spawn, canonicalizing and stat'ing `project_dir` before any
      spawn; verify a non-directory path is rejected with no child spawned and no
      existence disclosure in the message
- [ ] 5.6 Implement message and close, returning typed rejections for unknown
      keys; verify nothing is mutated on rejection
- [ ] 5.7 Implement turn retrieval with a monotonic position; verify a second call
      at the returned position yields only newer content
- [ ] 5.8 Drop a lagging subscriber rather than delivering a gap; verify a test
      with a deliberately stalled reader gets disconnected and re-snapshots
- [ ] 5.9 Start the server from `run.rs` and remove the socket file on graceful
      shutdown; verify the socket appears while running and is gone after exit

## 6. Socket client backend

- [ ] 6.1 Implement `SessionBackend` over the socket in the sebas binary crate;
      verify against a test server that each method reaches the right handler
- [ ] 6.2 Add reconnect with backoff and a fresh snapshot on reconnect; verify a
      test that kills and restarts the server converges without client restart
- [ ] 6.3 Report unreachable with its cause — socket absent, refused, secret
      rejected, dropped; verify each cause surfaces distinctly

## 7. Standalone WebUI cutover

- [ ] 7.1 Delete the `restore_session_map` call, the throwaway `SessionManager`,
      and the `RouterHandle` construction from `webui_cmd.rs`; verify no reference
      to the session state file remains in that file
- [ ] 7.2 Pass the socket backend to `webui::run_with_admin_adapter`; verify
      `sebas webui` starts with the core running and lists live sessions
- [ ] 7.3 Render the not-connected state on the board and disable the composer
      with its reason; verify by starting the WebUI with no core and confirming the
      cause is stated and no control reports success

## 8. Verification

- [ ] 8.1 With the core running, create a session from the standalone WebUI;
      verify a real ACP child appears and the session shows in both the WebUI and
      the core's own view
- [ ] 8.2 Send a message from the standalone WebUI; verify it reaches the real
      session rather than only local state
- [ ] 8.3 Restart the core under the watchdog while a page is open; verify the
      WebUI survives, reconnects, and converges with no manual reload
- [ ] 8.4 Compare the same pages under `sebas webui` and `run --webui`; verify
      session data and control availability are equivalent
- [ ] 8.5 Connect to the socket as a different uid and with a wrong secret; verify
      both are refused and nothing is mutated
- [ ] 8.6 Run the full workspace suite; verify `cargo test` passes and no test
      constructs a `RouterHandle` inside the WebUI crate
