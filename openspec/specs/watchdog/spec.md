# watchdog Specification

## Purpose
Defines the watchdog daemon: supervision of the core child process, safe
execution of upgrades and rollbacks, the authenticated control RPC surface,
the dangerous-action confirmation flow, the in-memory event timeline, service
lifecycle, and bare-core degraded mode.

## Requirements

### Requirement: Core child supervision

The watchdog SHALL spawn the core as `current_exe() core --config <path>` — the same binary, core subcommand, and the watchdog's own config — with piped stdio, `kill_on_drop`, and env `SEBAS_IPC=1` plus a per-instance `SEBAS_CONTROL_SECRET`. The core signals readiness over the pipe; a child exit is classified and the watchdog loops to respawn after a fixed 1000 ms delay. Spawn failures (missing binary, no stdio) SHALL be retried with the 1 s backoff up to N consecutive failures (default 3, configurable via `[watchdog] max_spawn_failures`), then SHALL enter the `failed-startup` terminal state; each failure SHALL write a structured error log and surface through the existing reporting paths (systemd unit status, `sebas ctl status`, Feishu boot notifications when enabled). A core exit before ready due to an early-fatal line SHALL count toward the same `failed-startup` counter; a startup failure of the new binary after a ready-after-rollback SHALL NOT count toward it (handled by the New-binary auto-rollback rules). Stopping the core uses SIGTERM with a 5 s grace period, then SIGKILL.

#### Scenario: core exit respawns

- **WHEN** the core child exits without an upgrade having just completed
- **THEN** the watchdog restarts it after 1 s and the crash counter increments

#### Scenario: spawn failure retries up to limit then terminates watchdog

- **WHEN** spawning the core child fails N 次连续失败（默认 3）
- **THEN** 该 service 状态置 `failed-startup`、watchdog 以退出码 75 终止、`sebas ctl status` 报告失败原因摘要（最近一次 stderr line + 失败计数）

#### Scenario: spawn failure within limit still logged

- **WHEN** spawning fails but未达 N 次上限
- **THEN** watchdog SHALL 写结构化错误日志（含 stderr 摘要）、按 1 s 退避重试、触发者经 `sebas ctl status` SHALL 看到上次失败时间与原因

#### Scenario: spawn failure retries

- **WHEN** spawning the core child fails
- **THEN** the watchdog logs the error, waits, and retries — the watchdog
  process itself stays alive until N 次连续失败再终止

#### Scenario: early-fatal counts toward startup-failure limit

- **WHEN** core 在 pipe 上发送 early-fatal 行后退出、且未达 ready
- **THEN** 该退出 SHALL 计入 `failed-startup` 终态计数器，与 spawn failure 同等待遇

### Requirement: Crash backoff

The crash counter SHALL apply per managed service (core, webui, router),
each with an independent counter that resets when that service's previous
crash was more than 1 h ago. Restarts proceed while the service's crash
count is at or below the maximum (3); once exceeded, the watchdog sleeps
30 s, resets that service's counter, and keeps looping — the watchdog never
exits due to a crashing child.

#### Scenario: crash limit cycling

- **WHEN** the core crashes a 4th time within the window
- **THEN** the watchdog sleeps 30 s, resets the core's counter, and
  continues supervising

#### Scenario: independent per-service counters

- **WHEN** the webui child crashes twice and the core child crashes once
  within the window
- **THEN** the core's crash count is 1 (not 3), and both services are
  restarted per their own counters

### Requirement: New-binary auto-rollback
When an upgrade (non-dry-run, non-rollback) just completed and the freshly started core exits BEFORE reporting ready, the watchdog SHALL classify the exit as new-binary-not-ready and automatically roll back to the previous version — without counting the exit against the crash counter. If no rollback backup exists or the rollback itself fails, the watchdog SHALL enter the `failed-startup` terminal state, exit with EX_TEMPFAIL (75), and `sebas ctl status` SHALL report the failure summary ("rollback failed: <reason>") — the watchdog SHALL NOT silently continue. The rollback trigger itself SHALL write a structured log (including the before/after binary paths), and the operator SHALL see a "rollback triggered" event via `sebas ctl status`.

#### Scenario: unready binary rolled back

- **WHEN** an upgraded core exits before its ready handshake
- **THEN** the watchdog rolls the `current` symlink back to the stored previous version and respawns

#### Scenario: rollback failure terminates watchdog

- **WHEN** auto-rollback finds no backup, or rollback command itself fails
- **THEN** watchdog 进入 `failed-startup` 终态、以 EX_TEMPFAIL (75) 退出、`sebas ctl status` 报告失败原因摘要

#### Scenario: rollback failure tolerated

- **WHEN** auto-rollback finds no backup
- **THEN** the watchdog logs the failure and SHALL NOT silently continue — it SHALL enter `failed-startup` 终态并以 75 退出

#### Scenario: rollback event is logged

- **WHEN** watchdog 触发 auto-rollback
- **THEN** 写入结构化日志含前/后二进制路径与触发原因；`sebas ctl status` SHALL 看到「rollback 触发」事件

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
`RestartCore`, `ServiceStatus`, `ServiceStatusFor`, `ServiceSet`,
`ServiceRestart`, `Confirm`, and `Cancel`. `ServiceSet` and
`ServiceRestart` SHALL act on the auxiliary managed services (webui,
router, im) as specified in the Service lifecycle requirement; requests
naming the core service SHALL be rejected with an actionable error.
`Confirm` and `Cancel` SHALL be accepted only from a Feishu actor with a
`chat_id`; any other actor gets `unauthorized`.

#### Scenario: service set accepted

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "off" }`
- **THEN** the response is `Accepted` and the WebUI child stops

#### Scenario: service set rejected

- **WHEN** a client sends `ServiceSet { service: "core", desired: "off" }`
- **THEN** the response is `Rejected` with an actionable message pointing
  to `RestartCore` (core lifecycle is supervised, not user-toggled)

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

The watchdog SHALL manage auxiliary services — the WebUI child, the router
child, and the IM child — as supervised child processes spawned from the same
binary (`sebas webui --config <path>` / `sebas router --config <path>` /
`sebas im --config <path>`, each given the control secret). The WebUI child
SHALL be spawned when `[watchdog.webui] enabled = true`; the router child
SHALL be spawned only when router management is explicitly enabled in the
watchdog config (default off — existing deployments see no new process until
they opt in); the IM child SHALL be spawned when `[watchdog.im] enabled = true`,
whose default SHALL follow the feishu enablement decision (`[feishu] enabled`
or its implicit fallback) so that feishu deployments gain the IM service
without a new config key. With `--debug`, the watchdog additionally spawns the
debug router child (`sebas router --debug`) as today.

Auxiliary children SHALL survive core restarts (only the core child is
respawned by an upgrade), and an auxiliary child that exits SHALL itself be
restarted per the crash-backoff policy. `ServiceStatus` and
`ServiceStatusFor` SHALL report each managed service's actual observed
state (running / restarting / stopped / disabled) derived from process
liveness and desired state — never a synthesized or hardcoded value.

`ServiceSet { service, desired, persist }` and
`ServiceRestart { service }` SHALL execute for the auxiliary services
(`webui`, `router`, `im`): `desired` ∈ {on, off} stops or starts the child;
`persist: true` records the desired state so it survives a watchdog restart,
`persist: false` scopes it to the current watchdog run. `ServiceSet` or
`ServiceRestart` naming the core service SHALL be rejected with an actionable
error pointing at `RestartCore` (core restarts flow exclusively through the
confirmed dangerous-action path).

#### Scenario: webui survives core restart

- **WHEN** the core child crashes and is restarted by the watchdog
- **THEN** the WebUI child process is untouched

#### Scenario: webui disabled by default

- **WHEN** the config has no `[watchdog.webui]` section
- **THEN** the watchdog spawns no WebUI child

#### Scenario: crashed webui is restarted

- **WHEN** the WebUI child process exits unexpectedly
- **THEN** the watchdog restarts it after the crash-backoff delay and its
  reported status reflects the restart in progress

#### Scenario: status reflects reality

- **WHEN** the WebUI child is killed and `ServiceStatus` is queried before
  the restart completes
- **THEN** the webui entry reports a non-running state rather than
  "running"

#### Scenario: service set toggles auxiliary service

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "off", persist: false }`
- **THEN** the response is `Accepted` and the WebUI child stops; a
  subsequent `ServiceStatus` reports webui as stopped with desired state off

#### Scenario: service set persisted across watchdog restart

- **WHEN** `ServiceSet { service: "router", desired: "on", persist: true }`
  is accepted and the watchdog is later restarted
- **THEN** the watchdog spawns the router child again without a new
  `ServiceSet`

#### Scenario: service commands on core are rejected

- **WHEN** a client sends `ServiceRestart { service: "core" }`
- **THEN** the response is `Rejected` with an actionable message pointing
  to `RestartCore`

### Requirement: Managed service table
The watchdog SHALL supervise all child processes through one declarative table of managed services — core, webui, router, and im — where each entry declares its spawn specification (argv, env), its desired state (from config or `ServiceSet`), and its restart policy. **补充**：受管服务名在 RPC/REST 接口与 control RPC 中 SHALL 统一使用 `core` / `webui` / `router` / `im` 四个字符串（产品对外 IM 即「飞书」，但内部字符串 SHALL 为 `im` 以与 `ServiceName::Im` 枚举对齐）；webui 前端 SHALL 通过 `service_from_str("im")` 识别，禁止使用 `feishu` 作为内部名。监督循环 SHALL 维持对全部 entry 的均匀处理（spawn / 退出分类 / 重启决策），New-binary auto-rollback 仍是 core 专属。config 中 disabled 的服务 SHALL 无子进程被拉起、状态报告为 `disabled`。core 子进程的 pipe 协议 SHALL 仅承载 readiness 握手（含 early fatal-error 行），控制操作不再经 pipe，control RPC socket 是唯一命令面。

#### Scenario: router managed when enabled

- **WHEN** the watchdog config enables router management
- **THEN** the watchdog spawns `sebas router --config <path>` as a supervised child and `ServiceStatus` includes a real router entry

#### Scenario: im managed when enabled

- **WHEN** the watchdog config enables IM management (explicitly or via the feishu-enablement default)
- **THEN** the watchdog spawns `sebas im --config <path>` with the control secret and `ServiceStatus` includes a real im entry

#### Scenario: disabled service reports disabled

- **WHEN** `ServiceStatus` is queried while webui management is disabled in config
- **THEN** the webui entry reports state `disabled` and no child exists

#### Scenario: upgrade commands only via RPC

- **WHEN** the core child wants an upgrade or rollback executed
- **THEN** it sends the request over the control RPC socket; the pipe carries only the readiness handshake

#### Scenario: im 字符串命名约定

- **WHEN** webui 通过 `/api/admin/services` 列出受管进程
- **THEN** IM/飞书服务的 name 字段 SHALL 为 `im`（不是 `feishu`）；前端识别 SHALL 经 `service_from_str("im")`，使用 `feishu` 调用将得到 `UnknownService` 错误

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

### Requirement: Readiness implies channel armed

core 子进程的 readiness 信号 SHALL 在核心会话通道武装完成之后发出：watchdog 观察到 ready 时，通道 socket 必已存在于解析路径。由此消灭"进程 Running 但通道永不武装"这一监督盲区。

#### Scenario: ready 之后 socket 必然存在

- **WHEN** watchdog 启动 core 并收到其 readiness 信号
- **THEN** 此时按同一 config 解析的通道 socket 路径上存在可连接的 socket

#### Scenario: 通道武装失败不产生 ready

- **WHEN** core 的通道 socket bind 失败（路径被存活进程占用）
- **THEN** core 以 bind 失败退出码退出且从未发出 ready，supervisor 依据退出码标记 Degraded（对齐 webui bind 失败语义），不进入无限重启循环
