## Why

当前 watchdog / 受管服务 / webui-spawn 三条路径在「启动失败」上的处理分散且不一致：watchdog 对 `spawn failed` 走 `warn!` + 1 s 退避重试（`src/watchdog/supervisor.rs:353`），但**没有终态上限**——只要 spawn 能执行就一直试，对 systemd / 触发 CLI 看上去是「卡住」；`New-binary auto-rollback`（spec line 71）写明 "rollback failure tolerated: logs the failure and continues its supervision loop"，继续跑而不是终止；`session_backend.rs:412` 注释明确「web_spawn never fails structurally: the placeholder is inserted and the spawn failure surfaces as a Removed event later」——**故意把启动失败延后到事件流**，触发者看不到当期错误；`dispatch.rs:112/129` 对 `web_spawn` 的 spawn 失败只 `warn!` 不传播。零散事实：进程启动失败至少有「silent retry」「silent continue」「delayed to event stream」「only warn」四种处理，触发者（systemd / 操作员 / 飞书）拿不到当期错误。这次把所有形态的「启动失败」规约收敛成 fail-fast + 结构化上报，并把行为钉进 spec。

## What Changes

- **新增**：进程启动失败的统一规约——任何进程（含 watchdog 自身、watchdog 受管的 core/webui/router/im、webui 派生的 acp 子进程、router 派生的上游连接）在「达到声明启动成功」前发生的失败**SHALL**：(1) 写入结构化错误日志含触发者可读摘要；(2) 终止该层进程或将该层明确降级为 `failed-startup` 终态（**不**进入监督循环的隐式吞错重试）；(3) 经既有上报通道让触发者收到「启动失败 + 原因」。
- **新增**：watchdog `Core child supervision` 的 spawn 失败策略从「无条件重试」改为「有限重试 + 终态失败」——N 次失败（默认 3，可配）后该 service 进入 `failed-startup` 终态、watchdog 整体退出非零、systemd unit 看到失败、`sebas ctl status` 报告失败原因。
- **新增**：watchdog `New-binary auto-rollback` 的 "rollback failure tolerated" 条款**改为**「rollback 失败时 watchdog 同样进入 `failed-startup` 终态并退出非零」——rollback 不再被静默吞掉。
- **新增**：bare core 形态（`sebas core --webui` / 沙箱联调）下，core 在 ready 之前 fatal（配置错误、socket bind 失败、state DB 不可写）**SHALL** 立即以非零退出码终止，并把 stderr 摘要写入 `--log-file`（若指定）/`SEBAS_STARTUP_ERROR_FILE`（沙箱验收用）；webui 在 `web_spawn` 派生 acp 子进程失败时**SHALL** 把错误立刻 inline 到 transcript（不延后到 Removed 事件）。
- **新增**：cli-service 退出码语义——`sebas core` / `sebas router` / `sebas webui` / `sebas run` 的退出码 SHALL 区分「启动失败」与「运行时崩溃」：启动失败 → 退出码 75（EX_TEMPFAIL，systemd 不会立即拉起）；运行时崩溃 → 退出码 1 或既有语义码。
- **修改**：`session_backend.rs:412` 注释的「spawn failure surfaces as a Removed event later」策略**改为**「spawn failure SHALL immediately surface as a transcript-level error with the spawn cause；Removed 事件仍可作为后续状态变更的次要信号」。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `watchdog`: 「启动失败上报与终态」规约（spawn 失败有限重试+终态、rollback 失败终止、watchdog 自身退出码）；「Bare-core degraded mode」补 startup-failure 错误摘要文件契约。
- `cli-service`: 各子命令的退出码语义区分 startup-failure vs runtime-crash。
- `core-session-channel`: core 启动失败时 core session channel 的错误传播规约。
- `webui`: 「web_spawn」失败的 inline transcript 上报规约。

## Impact

- 受影响代码：`src/watchdog/supervisor.rs`（spawn 失败重试上限 + 终态）、`src/watchdog/updater.rs`（rollback 失败终止逻辑）、`src/run.rs`（watchdog 自身退出码）、`src/webui_cmd.rs`（bare core 启动失败错误摘要）、`sebas-webui/src/session_backend.rs`（web_spawn inline 错误）、`sebas-webui/src/dispatch.rs`（dispatch spawn 失败传播）、各 `sebas xxx` 子命令入口退出码。
- 受影响 spec：`watchdog`、`cli-service`、`core-session-channel`、`webui` 四份 spec 的相关 requirement 改写。
- 受影响 systemd unit：`Restart=on-failure`（既有）会因 EX_TEMPFAIL (75) 走指数退避而不是无限重启——这是设计意图。
- 不影响：被测 webui 后端路由形状（仅行为规约）、现有 happy-path 旅程（启动失败是异常路径）。

## Non-goals

- 不重写 watchdog 监督循环的 happy-path（已 ready 服务的崩溃退避、回滚仍按既有规约）。
- 不改 `crash backoff` 既有条款（运行期崩溃仍走 4 次/1 h 窗口）。
- 不引入新的上报通道——继续使用 stderr / `sebas ctl status` / systemd unit 状态 + 飞书 boot 通知（如有）。
- 不支持「运行中降级到 backup 二进制」（仅启动阶段触发回滚）。
- 不改 spec 既有 `failure_is_startup_cause` / `failure_is_startup_cause` 字段名（如有），仅补全语义。
- 不动 cli-service 既有子命令树与隐藏别名兼容窗口。