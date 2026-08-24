# watchdog Specification

## Purpose
Defines the watchdog daemon: supervision of the core child process, safe
execution of upgrades and rollbacks, the authenticated control RPC surface,
the dangerous-action confirmation flow, the in-memory event timeline, service
lifecycle, and bare-core degraded mode.

## Requirements

### Requirement: Core child supervision

The watchdog SHALL spawn the core as `current_exe() run --config <path>` —
the same binary, run subcommand, and the watchdog's own config — with piped
stdio, `kill_on_drop`, and env `SEBAS_IPC=1` plus a per-instance
`SEBAS_CONTROL_SECRET`. The core signals readiness over the pipe; a child
exit is classified and the watchdog loops to respawn after a fixed 1000 ms
delay. Spawn failures (missing binary, no stdio) retry rather than crash the
watchdog. Stopping the core uses SIGTERM with a 5 s grace period, then
SIGKILL.

#### Scenario: core exit respawns

- **WHEN** the core child exits without an upgrade having just completed
- **THEN** the watchdog restarts it after 1 s and the crash counter
  increments

#### Scenario: spawn failure retries

- **WHEN** spawning the core child fails
- **THEN** the watchdog logs the error, waits, and retries — the watchdog
  process itself stays alive

### Requirement: Crash backoff

The crash counter SHALL reset when the previous crash was more than 1 h ago.
Restarts proceed while the crash count is at or below the maximum (3); once
exceeded, the watchdog sleeps 30 s, resets the counter, and keeps looping —
the watchdog never exits due to a crashing core.

#### Scenario: crash limit cycling

- **WHEN** the core crashes a 4th time within the window
- **THEN** the watchdog sleeps 30 s, resets the counter, and continues
  supervising

### Requirement: New-binary auto-rollback

When an upgrade (non-dry-run, non-rollback) just completed and the freshly
started core exits BEFORE reporting ready, the watchdog SHALL classify the
exit as new-binary-not-ready and automatically roll back to the previous
version — without counting the exit against the crash counter. If no
rollback backup exists or the rollback itself fails, the watchdog logs the
error and keeps running.

#### Scenario: unready binary rolled back

- **WHEN** an upgraded core exits before its ready handshake
- **THEN** the watchdog rolls the `current` symlink back to the stored
  previous version and respawns

#### Scenario: rollback failure tolerated

- **WHEN** auto-rollback finds no backup
- **THEN** the watchdog logs the failure and continues its supervision loop

### Requirement: Control RPC transport and authentication

The control plane SHALL speak JSON-Lines over a Unix domain socket at
`$XDG_RUNTIME_DIR/sebas/control.sock` (fallback `$TMPDIR/sebas/uid<uid>/`),
mode 0600, one task per accepted stream. Every envelope SHALL carry
`version` (must be 1), a `secret` matching the watchdog's per-instance
startup secret, and an `actor` of either `Cli { uid }` or
`Feishu { open_id, chat_id }` — a `System` actor cannot be forged on the
wire. Wrong or missing secret → `unauthorized`; wrong version →
`unsupported_version`. The secret is generated at startup
(`{pid}-{timestamp}`) and never persisted, so restarting the watchdog
invalidates outstanding clients.

#### Scenario: wrong secret rejected

- **WHEN** a request envelope carries a secret that does not match the
  watchdog's instance secret
- **THEN** the response is `Rejected { code: "unauthorized" }`

#### Scenario: system actor unforgeable

- **WHEN** a client sends an envelope whose actor field claims `system`
- **THEN** deserialization rejects it before any handler runs

### Requirement: Control request surface

The RPC SHALL serve: `Status`, `EventsSince`, `Update`, `Rollback`,
`RestartCore`, `ServiceStatus`, `ServiceStatusFor`, `Confirm`, and `Cancel`.
`ServiceSet` and `ServiceRestart` SHALL be rejected with
`service_unavailable` (service management is not wired). `Confirm` and
`Cancel` SHALL be accepted only from a Feishu actor with a `chat_id`;
any other actor gets `unauthorized`.

#### Scenario: service set rejected

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "stopped" }`
- **THEN** the response is `Rejected { code: "service_unavailable" }`

#### Scenario: cli cannot confirm

- **WHEN** a `Cli` actor sends `Confirm { token }`
- **THEN** the response is `Rejected { code: "unauthorized" }`

### Requirement: Upgrade execution

Upgrades SHALL run in a managed subprocess (`sebas update --config <path>`
re-exec) with a mode-dependent timeout — dev builds (local
`cargo build --release`) default 1800 s, release downloads default 600 s,
both floored at 1 s; on timeout the subprocess is stopped SIGTERM → 5 s →
SIGKILL. The release path: acquire the upgrade lock, check the latest GitHub
release (no newer version → report up-to-date, no restart), optionally
dry-run (validate only, no install), download with SHA256 checksum
verification, then install — copy into `versions/v<ver>/`, back up the
previous binary to `rollback/`, flip the `current` symlink. Only a
successful non-dry-run upgrade requests a core restart; a failed upgrade
never restarts the core and settles its operation as failed; a panicking
runner is caught, settles failed, and releases the lock.

#### Scenario: distinct timeouts per mode

- **WHEN** a dev upgrade runs with default config
- **THEN** its subprocess timeout (1800 s) is far above the release timeout
  (600 s); the 5 s retry-delay config value is never used as a timeout

#### Scenario: up-to-date short-circuit

- **WHEN** the latest release equals the running version
- **THEN** the operation completes without download, install, or restart

#### Scenario: failed upgrade no restart

- **WHEN** the download step fails
- **THEN** the operation settles failed, the upgrade lock is released, and
  the core child keeps running

### Requirement: Rollback execution

Rollback SHALL re-point the `current` symlink to the stored
`versions/rollback/sebas` binary (backing up the current binary first).
With no stored backup, rollback is an error. Dry-run rollback prints the
would-be action without locking or writing. A rollback never marks the
subsequent child exit as new-binary-not-ready.

#### Scenario: rollback restores previous

- **WHEN** a rollback runs with a stored backup
- **THEN** `current` resolves to the previous version's binary and the
  pre-rollback binary is preserved in the temp backup during the swap

#### Scenario: no backup errors

- **WHEN** rollback runs with no `rollback/sebas` present
- **THEN** the operation fails with a no-rollback-version error

### Requirement: Event timeline

The watchdog SHALL keep an in-memory event timeline (bounded at 200 events,
oldest evicted) recording per-operation lifecycle events — Started,
Progress, Done, Error, Canceled, TimedOut — each with a monotonic sequence
number, timestamp, operation id, kind, and public message. `EventsSince`
returns all events with a sequence number greater than the cursor. The
timeline is not durable across restarts and is not an audit log.

#### Scenario: bounded retention

- **WHEN** more than 200 events are recorded
- **THEN** the oldest events are evicted and `EventsSince { seq: 1 }` no
  longer returns them

#### Scenario: cursor polling

- **WHEN** a client polls `EventsSince { seq: 42 }`
- **THEN** it receives exactly the events with sequence numbers > 42

### Requirement: Dangerous-action confirmation

Update, rollback, and restart-core submitted by a Feishu actor (with
`chat_id`) SHALL return a pending confirmation carrying an opaque
single-use token, the action, a human message, and a 300 s expiry; the
action executes only after a matching `Confirm` redeems the token (same
actor, same channel, same params). A Feishu actor without `chat_id` gets
`confirmation_required`. `Cli` and local actors execute directly without
confirmation. `Cancel` redeems the token so it can no longer be confirmed
and records a Canceled event. Concurrent redemption attempts yield exactly
one execution.

#### Scenario: feishu update requires confirm

- **WHEN** a Feishu actor submits `Update`
- **THEN** the response is a pending confirmation with a token, and the
  upgrade does not start until `Confirm` arrives

#### Scenario: param change invalidates grant

- **WHEN** a grant was issued for a dev update and `Confirm` is sent while
  the pending request describes a release update
- **THEN** the redemption is rejected

#### Scenario: single redemption under concurrency

- **WHEN** two `Confirm` calls for the same token race
- **THEN** exactly one succeeds and the other reports already-redeemed

### Requirement: Service lifecycle

The watchdog SHALL spawn the WebUI as a separate child process
(`sebas webui --config <path>` with the control secret) when
`[watchdog.webui] enabled = true`, and the WebUI child SHALL survive core
restarts (only the core child is respawned). With `--debug`, the watchdog
additionally spawns a debug gateway child (`sebas gateway --debug`).
Service status queries return a synthesized status report; per-service
start/stop is not available (rejected until wired).

#### Scenario: webui survives core restart

- **WHEN** the core child crashes and is restarted by the watchdog
- **THEN** the WebUI child process is untouched

#### Scenario: webui disabled by default

- **WHEN** the config has no `[watchdog.webui]` section
- **THEN** the watchdog spawns no WebUI child

### Requirement: Bare-core degraded mode

A core started without the watchdog (no `SEBAS_IPC=1`) SHALL run all
business functions but reject control-plane commands with an actionable
error (the control secret is absent), and SHALL print a startup warning to
that effect. The one-shot `sebas update` CLI remains available for manual
upgrades in bare mode.

#### Scenario: control command in bare mode

- **WHEN** the user runs `/upgrade` against a bare-core instance
- **THEN** the reply is a failure message explaining the watchdog is not
  supervising this instance

#### Scenario: manual update still works

- **WHEN** the user runs `sebas update --config ...` directly against the
  bare-core installation
- **THEN** the update runs as a one-shot without a watchdog
