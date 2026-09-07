## MODIFIED Requirements

### Requirement: Honest degradation when the core is unreachable
When the core session channel is unhealthy or the core is unreachable, the WebUI SHALL display a stable degradation banner — never pretend to be healthy, never silently retry behind a fake spinner. **补充**：当 core 因 startup failure（端口未 bind、control secret 缺失、state DB 不可写、config 解析失败）退出 75 时，WebUI SHALL 经 `GET /health` 或 `/api/summary` 探测到 core 进程不可用，并在 degradation banner 中显式呈现 "core startup failed: <可读原因>"（原因来自 core 进程 stderr 末行或 `SEBAS_STARTUP_ERROR_FILE`）；不出现"假活"或"假装还在连"状态。

#### Scenario: core startup failure surfaces in banner

- **WHEN** core 在 ready 之前 fatal 并以 75 退出
- **THEN** webui degradation banner SHALL 显示 "core startup failed: <原因>"；`/api/summary.reachability.ok` 为 false 且 cause 字段包含 startup failure 的可读摘要

#### Scenario: core healthy after retry does not retain banner

- **WHEN** watchdog 重启 core 后 core 进入 ready（属运行期崩溃退避场景，不是 startup failure）
- **THEN** degradation banner 消失、reachability 恢复 true——本规约不影响运行期恢复路径

#### Scenario: core down is stated, not hidden

- **WHEN** the core is not running and a client renders a session view
- **THEN** the view states that the core is not connected and why, rather than
  rendering an empty or stale list as though it were current

#### Scenario: no unsendable controls

- **WHEN** the channel is unreachable
- **THEN** controls that would require a channel request are unavailable and
  labeled with the reason, and no such request reports success

#### Scenario: reconnect resumes from a snapshot

- **WHEN** the core restarts while a client is connected
- **THEN** the client reconnects, takes a fresh snapshot, and its view converges
  on the core's state without a manual reload