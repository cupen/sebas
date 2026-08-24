## 1. watchdog

- [x] 1.1 Write `specs/watchdog/spec.md` from the research inventory (supervision, crash backoff, auto-rollback, RPC auth + request surface, upgrade/rollback execution, timeline, confirmation, service lifecycle, bare-core)
- [x] 1.2 Record unwired surfaces as current behavior (ServiceSet/ServiceRestart → `service_unavailable`); omit dead config fields (`max_retries`/`retry_delay_secs`/`check_on_start`)

## 2. webui

- [x] 2.1 Write `specs/webui/spec.md` (routes, loopback-only, optional admin auth, mutation posture, dashboard/focus, close, standalone detached semantics, admin-via-RPC, lifecycle ownership)
- [x] 2.2 Document the detached-mode truth: standalone session ops are local-only (outbound channel dropped); CSRF-token path noted as non-operative vs loopback-origin path

## 3. cli-service

- [x] 3.1 Write `specs/cli-service/spec.md` (subcommand tree, bare-`sebas` parse error, unit generation, exit codes, start/uninstall, config discovery, env-only bootstrap, precedence, control client)
- [x] 3.2 Correct proposal: flag is `--user`, not `--run-as`

## 4. replay-debug

- [x] 4.1 Write `specs/replay-debug/spec.md` (recording format/scope, invocation, fidelity, filter divergence, dedup, side-effect boundary, fault tolerance)
- [x] 4.2 Note filename unit: code uses unix-nanos (doc comments claim ms) — spec records nanos; `sebas record` excluded as dev fixture tooling

## 5. Validation

- [x] 5.1 `openspec validate bootstrap-specs-batch4 --strict` passes
- [x] 5.2 Discrepancies collected for the final report (dead config fields, doc-vs-code socket path, unwired idempotency/assertions, `/agent` dead route, global login limiter)
