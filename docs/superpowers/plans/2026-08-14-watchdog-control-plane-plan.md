# Watchdog Control Plane Multi-Phase Plan

> **For agentic workers:** 本计划按阶段落地 `watchdog` 统一控制面。执行时优先保持每个 phase 可独立编译、可测试、可回滚。不要把 core 业务语义搬进 watchdog。
>
> 关联设计：`docs/superpowers/specs/2026-08-14-watchdog-control-plane-design.md`
> 状态：草案 v2（已吸收第一轮验收反馈）

**Goal:** 将 sebas 控制能力收敛到 watchdog control service；WebUI 与 Feishu 都作为入口调用同一套控制 API；watchdog 管理 core/updater/webui/gateway/feishu 的生命周期，但不理解 session/permission/ACP/card rendering 等 core 业务语义。

## Phase dependency summary

| Phase | Prerequisites | Deliverables | Parallelizable |
|---|---|---|---|
| P0 — updater split hardening | none | updater subprocess, readiness/version policy | no |
| P1 — control foundation | P0 | ControlService, private RPC, auth, confirmation, events | no |
| P2 — WebUI console | P1 | watchdog-hosted admin UI + security baseline | with P3 |
| P3 — Feishu adapter | P1 | core-hosted Feishu proxy over control RPC | with P2 |
| P4 — ServiceManager | P1, coordinates P2 | desired/runtime lifecycle for WebUI/Gateway/Feishu | with late P2/P3 |
| P5 — Feishu broker | P4 | watchdog-hosted Feishu transport | no |
| P6 — CLI/socket client | P1 | public `sebas control ...` client | with P2/P3/P4 |

```text
P0 updater split hardening
  -> P1 control service + private RPC + auth/confirmation/event contracts
    -> P2 watchdog-hosted WebUI control console
    -> P3 Feishu proxy adapter over control RPC
    -> P4 ServiceManager for WebUI/Gateway/Feishu lifecycle
    -> P6 public CLI/socket client
      P4 -> P5 Feishu transport broker
```

关键调整：私有 control RPC 不等到 CLI 阶段才做；它必须在 P1 就存在，供 core-hosted Feishu proxy 等跨进程 adapter 使用。

## Module directory layout

Implementation should migrate the single `src/watchdog.rs` file toward:

```text
src/watchdog/
├── mod.rs
├── supervisor.rs
├── control.rs
├── auth.rs
├── confirmation.rs
├── events.rs
├── updater.rs
├── services.rs
├── control_rpc.rs
├── adapters/
│   ├── mod.rs
│   ├── webui.rs
│   ├── feishu.rs
│   └── cli.rs
└── transport.rs
```

Keep these flat modules unless/until separately refactored:

```text
src/update.rs
src/upgrade.rs
src/ipc.rs
router/*
webui/*
```

Dependency rule: `webui` crate must not directly depend on the root `sebas` crate. Use trait/client injection for P2; consider a tiny shared DTO crate later only if needed.

## Test naming strategy

- Unit tests: inline next to pure control/auth/confirmation/state-machine code.
- Integration tests: `tests/watchdog_{phase}_{name}_test.rs`.
- Fake binaries: `tests/bin/fake-updater.rs`, `tests/bin/fake-child.rs` or generated temp scripts.
- WebUI tests: `webui/tests/admin_test.rs` where crate boundaries require.
- Adapter parity tests: assert WebUI and Feishu produce the same normalized `ControlRequest` for equivalent actions.

## Phase 0 — Stabilize updater split and readiness semantics

**目标：** 巩固当前热更新拆分：watchdog 编排 `sebas update` 子进程，update 子命令负责 release/dev/rollback 实际安装工作。明确 P0 暂不 self-reexec watchdog，只重启 core child。

### Task 0.1 — Harden updater subprocess orchestration

**Files:**

- `src/watchdog.rs`
- `src/update.rs`
- `src/upgrade.rs`
- `src/main.rs`
- `src/cli.rs`

**要求：**

- watchdog 不直接调用 `upgrade::download_release` / `compile_dev` / `install_version` / `rollback`。
- watchdog 只启动 `sebas update ...` 子进程并根据退出码决定是否重启 core。
- `sebas update --dev --project-dir <dir>` 支持显式 dev 项目目录；后续 P1/P3 改为 configured target name。
- dry-run 成功不重启 core。
- updater 失败不重启 core。
- updater stdout/stderr 有 bounded capture 或明确继承策略；不能无限占内存。
- updater timeout：先 terminate，再 force kill。

**测试：**

- `cargo check`
- `cargo test update_subcommand_accepts_dev_project_dir_and_dry_run`
- `cargo test update_rollback_conflicts_with_dev`
- `cargo test upgrade::tests`

### Task 0.2 — Add watchdog updater smoke test

**Files:**

- `tests/watchdog_update_test.rs` 或 `src/watchdog.rs` inline tests

**要求：**

- 抽 `UpdaterRunner` struct/trait，允许测试注入 fake updater。
- 验证：
  - dry-run updater exit 0 -> 不请求 restart。
  - release updater exit 0 -> 请求 restart。
  - updater exit non-zero -> 返回 error，不 restart。
  - updater timeout -> kill updater，不 restart。

### Task 0.3 — Define watchdog version policy

**Files:**

- `docs/superpowers/specs/2026-08-14-watchdog-control-plane-design.md`
- `src/watchdog.rs` comments/tests where applicable

**要求：**

- P0 policy: old watchdog may run new core; IPC must remain at least one release backward compatible。
- update that changes watchdog/control plane security should surface “watchdog service restart required”。
- new core readiness failure after update must not silently loop forever; define rollback/manual recovery behavior。

**测试：**

- fake current/new core readiness success/failure matrix。

## Phase 1 — ControlService, private RPC, auth, confirmation, events

**目标：** 建立 watchdog 内部统一控制 API 和跨进程 adapter 的私有 RPC。所有后续 WebUI/Feishu/CLI 入口都基于它。

### Task 1.1 — Define control domain types

**Files:**

- `src/watchdog/control.rs`（新）
- `src/watchdog/mod.rs` 或后续拆分 `src/watchdog.rs`

**接口草案：**

```rust
pub enum ControlRequest {
    Status,
    RestartCore,
    StopCore,
    StartCore,
    Update { kind: UpdateKind, dry_run: bool, target: Option<UpdateTarget> },
    Rollback { dry_run: bool },
    ServiceSet { service: ManagedService, desired: DesiredState, persist: bool },
    ServiceRestart { service: ManagedService },
    ServiceStatus,
}

pub enum UpdateKind { Release, Dev }
pub enum UpdateTarget { ConfiguredDevTarget { name: String } }
pub enum ManagedService { WebUi, Gateway, Feishu }
pub enum DesiredState { Enabled, Disabled }
```

**要求：**

- Core 不作为 generic service toggle；用 Start/Stop/RestartCore。
- Dev update 不接受 arbitrary raw path；使用 configured target name。
- control types 不引用 router/session/card/ACP 类型。

**测试：**

- compile + type-level tests。
- debug/log output 不泄漏 secrets。

### Task 1.2 — Add private authenticated control RPC

**Files:**

- `src/watchdog/control_rpc.rs`（新）
- `src/ipc.rs` 或独立 protocol module
- `src/watchdog.rs`

**要求：**

- Unix socket：`$XDG_RUNTIME_DIR/sebas/control.sock`，fallback 到 data_dir。
- socket permission 0600。
- watchdog startup secret / authenticated channel。
- RPC client 不能提交 `Actor::System`。
- JSONL envelope 带 version、request_id、actor assertion、confirmation、request。
- response 区分 accepted/rejected。
- event polling: `operation.get` / `events.since(seq)`。

**测试：**

- socket permission。
- missing/invalid secret rejected。
- forged `System` actor rejected。
- old protocol version rejected with typed error。

### Task 1.3 — Trusted actor and authorization policy

**Files:**

- `src/watchdog/control.rs`
- `src/watchdog/auth.rs`（新）

**要求：**

- Actor construction boundary 明确。
- core-hosted Feishu proxy 必须提交 signed short-lived assertion：principal、canonical action、args hash、iat/exp、nonce、watchdog instance id。
- watchdog 验证 Feishu owner/allowlist，不信任 core 传入的 open_id 字符串。
- `Actor::System` 只能 watchdog 内部构造。

**测试：**

- forged owner id rejected。
- replayed assertion rejected。
- expired assertion rejected。
- parameter substitution rejected。
- external System actor rejected。

### Task 1.4 — Confirmation grants

**Files:**

- `src/watchdog/confirmation.rs`（新）
- `src/watchdog/control.rs`

**要求：**

- dangerous action 需要 confirmation grant。
- grant single-use、short-lived、绑定 principal/action/channel/watchdog instance/nonce。
- double-click/callback retry idempotent。
- adapter-local confirm boolean 不可作为授权依据。

**测试：**

- replay rejected。
- cross-user/card forwarding rejected。
- expired grant rejected。
- changed params rejected。
- concurrent confirm only one execution。

### Task 1.5 — Operation state machine and event/audit contracts

**Files:**

- `src/watchdog/control.rs`
- `src/watchdog/events.rs`（新）

**要求：**

- State: `PendingConfirmation -> Accepted -> Running -> Succeeded|Failed|Canceled|TimedOut`。
- update/rollback/restart/stop-core 互斥。
- typed machine-readable error codes。
- events 有 seq、timestamp、operation_id、public_message、redacted diagnostic。
- bounded in-memory timeline + bounded durable audit for privileged actions。
- idempotency key 支持 browser retry / Feishu callback retry。

**测试：**

- operation conflict table。
- event ordering and capacity eviction。
- audit redaction。
- duplicate idempotency returns same operation/outcome。

## Phase 2 — WebUI as primary watchdog control console

**目标：** WebUI 成为 watchdog-hosted 主要控制端，展示状态、事件与危险操作按钮。

### Task 2.1 — Attach WebUI lifecycle to watchdog

**Files:**

- `src/watchdog/services.rs`（新）
- `src/run.rs`
- `webui/src/server.rs`
- `src/cli.rs`
- `src/config.rs`

**要求：**

- watchdog 根据配置启动 WebUI task。
- WebUI task 与 core child 生命周期分离。
- core restart 不停止 WebUI。
- 保留 `sebas run --webui` 兼容，但加 single-owner/port guard 防双启动。

**测试：**

- watchdog 启动 WebUI task。
- core restart WebUI 仍可访问。
- double-start guarded。

### Task 2.2 — Add admin dashboard pages

**Files:**

- `webui/src/routes.rs`
- `webui/templates/*.html`
- `webui/static/style.css`

**页面：**

- `/admin/status`
- `/admin/update`
- `/admin/services`
- `/admin/events`

**要求：**

- WebUI adapter 调用 in-process ControlService 或 local control RPC，不绕过 authorization。
- 显示 watchdog/core/updater/webui/gateway/feishu desired/runtime status。
- Update/Dry-run/Dev Update/Rollback/Restart Core 走 operation API。
- Timeline 使用 `events.since(seq)` 或 service-provided snapshot。

**测试：**

- fake ControlService adapter contract tests。
- route smoke tests。
- mutation route produces expected normalized ControlRequest。

### Task 2.3 — WebUI local security baseline

**Files:**

- `src/config.rs`
- `webui/src/server.rs`
- `webui/src/routes.rs`

**要求：**

- 默认 bind `127.0.0.1`。
- host 非 loopback 且 secure mode disabled -> startup error。
- mutation endpoints POST-only。
- cookie-auth mutation 使用 CSRF token + origin/referer policy。
- custom header 可作为额外保护，但不能替代 cookie CSRF。
- password 从 env 读取；不明文落盘/日志；session 有 expiry；登录限速。

**测试：**

- non-loopback no auth rejected。
- spoofed forwarded headers ignored unless trusted proxy configured。
- GET mutation -> 405。
- missing CSRF -> 403。
- secret redaction。

## Phase 3 — Feishu Control Adapter over watchdog RPC

**目标：** Feishu 也具备 watchdog 控制能力，但早期作为 core-hosted proxy，经私有 control RPC 调用 watchdog。明确此阶段 core 不可用时 Feishu 控制也不可用。

### Task 3.1 — Split Feishu commands into control vs core

**Files:**

- `router/src/commands.rs`
- `router/src/router/inbound.rs`
- `src/dispatch.rs`
- `src/watchdog/control_rpc.rs`

**Control commands:**

```text
/upgrade [--dry-run|--dev|--dev --dry-run]
/rollback
/restart
/services
/gateway on|off|restart|status
/webui status
/system
```

**要求：**

- control commands 生成 normalized `ControlRequest` 并经 control RPC 发给 watchdog。
- core commands 保持原 router/session 逻辑。
- `/status` 暂保留 core；新增 `/system` 给 watchdog。
- Feishu proxy 提交 signed actor assertion。

**测试：**

- command parse tests。
- normalized request adapter contract tests。
- core unavailable limitation documented in status/help。

### Task 3.2 — Dangerous action confirmation cards

**Files:**

- `feishu/src/cards.rs`
- `router/src/card_events.rs`
- `src/dispatch.rs`
- `src/watchdog/confirmation.rs`

**要求：**

- `/upgrade`、`/rollback`、`/restart` 先请求 watchdog 创建 pending confirmation。
- Feishu card 携带 opaque confirmation token，不携带可篡改 action truth。
- 确认 callback 经 RPC finalize confirmation。
- 取消记录 Canceled event。

**测试：**

- confirm card render tests。
- non-owner denied。
- replay/cross-user/changed-param rejected。
- double-click idempotent。

### Task 3.3 — Feishu progress rendering

**Files:**

- `src/watchdog/events.rs`
- `src/dispatch.rs`
- `router/src/router/mod.rs`

**要求：**

- Feishu adapter 使用 operation_id 拉取 events。
- MVP 可发文本进度；后续更新同一张进度卡。
- Feishu send failure 不影响 control operation。

**测试：**

- Progress/Done/Error render。
- event delivery failure does not fail operation。

## Phase 4 — ServiceManager for WebUI/Gateway/Feishu

**目标：** watchdog 统一管理服务 desired/runtime 状态，支持 WebUI/Gateway/Feishu lifecycle。

### Task 4.1 — Introduce ServiceManager

**Files:**

- `src/watchdog/services.rs`
- `src/config.rs`
- `src/watchdog/control.rs`

**要求：**

- desired state 与 runtime state 分离。
- start/stop/restart/status。
- persist=true 只有在原子配置写入实现后才开放；否则返回 unsupported。
- disabling WebUI from WebUI 必须先返回 accepted/done，并提示恢复路径。

**测试：**

- service transitions。
- disable WebUI from WebUI。
- failed restart reports RuntimeState::Failed。
- config persist failure rollback。

### Task 4.2 — Gateway lifecycle migration

**Files:**

- `src/gateway_cmd.rs`
- `src/run.rs`
- `src/watchdog/services.rs`
- `gateway/*`

**要求：**

- gateway 作为 watchdog managed task 启动。
- core restart 不影响 gateway。
- 旧 `sebas run --gateway` 兼容但 single-owner guarded。

**测试：**

- gateway start/stop smoke。
- provider config load failure reports service error。
- double-start prevented。

## Phase 5 — Feishu transport broker（optional / availability-driven）

**目标：** 将 Feishu transport 移入 watchdog，使 core 重启/崩溃期间 Feishu 仍可控制 watchdog。

**产品决策：** 如果要求“core dead 时 Feishu 仍可执行 watchdog control”，则 P5 必须做；否则它可以后置。

### Task 5.1 — Define transport envelopes

**Files:**

- `src/ipc.rs`
- `src/watchdog/transport.rs`（新）
- `src/run.rs`

**要求：**

- `FeishuInbound` opaque event 转发给 core。
- `FeishuOutboundRequest` 从 core 到 watchdog。
- `FeishuOutboundResult` 返回 message_id/reaction_id。
- watchdog 只识别 control commands；不引入 permission/session/card semantic types。

**测试：**

- request/response correlation。
- child unavailable inbound policy。
- no core semantic type leakage。

### Task 5.2 — Migrate outbound first

**Files:**

- `src/dispatch.rs`
- `feishu/src/client.rs`
- `src/watchdog/transport.rs`

**要求：**

- child 经 IPC 请求 watchdog 发 Feishu。
- watchdog 返回 send_card message_id。
- router 仍能 record root msg id。

**测试：**

- send_card result mapping。
- update_card by msg id。
- reaction id return。

### Task 5.3 — Migrate inbound last

**Files:**

- `src/ws_loop.rs`
- `src/run.rs`
- `src/watchdog/transport.rs`

**要求：**

- watchdog 持有 Feishu WS/HTTP inbound。
- core ready 后转发 inbound。
- core restarting 时对普通消息提示 restarting 或短队列。
- control commands 可在 core down 时由 watchdog 直接处理。

**测试：**

- core restart during inbound。
- Feishu control while core dead。
- button callback during restart policy。

## Phase 6 — Public CLI/socket client

**目标：** 将 P1 私有 control RPC 产品化为本地 CLI，用作自动化与救援入口。

### Task 6.1 — CLI commands

**Files:**

- `src/cli.rs`
- `src/main.rs`
- `src/watchdog/control_rpc.rs`

**Commands:**

```bash
sebas control status
sebas control update [--dry-run]
sebas control update --dev --target local
sebas control rollback
sebas control restart-core
sebas control service gateway on|off|restart|status
```

**测试：**

- CLI parse tests。
- child dead 时 CLI status works。
- unauthorized socket client rejected。

## Cross-phase rules

- 每个 phase 保持 `cargo check` 通过。
- 不把 core business semantic types 引入 watchdog control/service layer。
- 危险操作必须记录 actor、confirmation、operation id。
- Feishu/WebUI/CLI 不得绕过 ControlService。
- update/rollback/restart/stop-core 必须互斥。
- adapter parity tests：WebUI 和 Feishu 对同一动作产生相同 normalized request。
- 外部网络/真实 Feishu/GitHub 不出现在单测中；使用 fake runner/client。
- 迁移期每个 capability 必须有 single-owner guard，防双启动/双解析。

## Suggested validation commands

```bash
cargo check
cargo test -p router --test commands_test
cargo test upgrade::tests
cargo test update_subcommand_accepts_dev_project_dir_and_dry_run
cargo test update_rollback_conflicts_with_dev
```

每个 phase 必须补 focused tests，并将设计验收标准转成可执行测试或明确的 manual smoke。
