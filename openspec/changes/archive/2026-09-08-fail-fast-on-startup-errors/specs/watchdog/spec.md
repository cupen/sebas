## MODIFIED Requirements

### Requirement: Core child supervision
The watchdog SHALL spawn the core as `current_exe() core --config <path>` — the same binary, core subcommand, and the watchdog's own config — with piped stdio, `kill_on_drop`, and env `SEBAS_IPC=1` plus a per-instance `SEBAS_CONTROL_SECRET`. The core signals readiness over the pipe; a child exit is classified and the watchdog loops to respawn after a fixed 1000 ms delay. **修改**：spawn failures (missing binary, no stdio) 在 N 次（默认 3，可由 `[watchdog] max_spawn_failures` 配置）连续失败后 SHALL 进入 `failed-startup` 终态——该 service 状态置 `failed-startup`、watchdog 进程 SHALL 以 EX_TEMPFAIL (75) 退出、`sebas ctl status` SHALL 报告失败原因摘要（最近一次 stderr line + 失败计数）。N 次以内 SHALL 仍按 1 s 退避重试，但每次失败 SHALL 写入结构化错误日志（含 stderr 摘要）且 SHALL 经既有上报通道（systemd unit 状态、`sebas ctl status`、飞书 boot 通知若启用）让触发者即时可见。Stopping the core uses SIGTERM with a 5 s grace period, then SIGKILL。**新增**：core 在 ready 之前因 early-fatal 行退出同样计入 `failed-startup` 终态计数器；新二进制 ready-after-rollback 后再启动失败不计入本计数器（按 New-binary auto-rollback 既有规约处理）。

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

### Requirement: New-binary auto-rollback
When an upgrade (non-dry-run, non-rollback) just completed and the freshly started core exits BEFORE reporting ready, the watchdog SHALL classify the exit as new-binary-not-ready and automatically roll back to the previous version — without counting the exit against the crash counter. **修改**：If no rollback backup exists or the rollback itself fails, the watchdog **SHALL** 进入 `failed-startup` 终态、watchdog 进程以 EX_TEMPFAIL (75) 退出、`sebas ctl status` SHALL 报告失败原因摘要（"rollback failed: <原因>"）——不再 silently continue。**新增**：rollback 触发本身 SHALL 写入结构化日志（包含前/后二进制路径），触发者经 `sebas ctl status` SHALL 看到「rollback 触发」事件。

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