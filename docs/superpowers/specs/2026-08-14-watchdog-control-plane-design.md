# Watchdog Control Plane — WebUI / Feishu 统一控制设计

> 日期：2026-08-14
> 状态：草案 v2（已吸收第一轮验收反馈）
> 作者：Claude（与 cupen 协作）

## 1. 背景

sebas 正在从单一 `sebas run` 进程演进为带热更新能力的长期服务。新的架构原则是：

> **控制能力属于 watchdog；WebUI、Feishu、CLI 只是进入 watchdog control plane 的 adapter。**

WebUI 是主要人机控制端；Feishu 也必须具备控制能力；未来 CLI/socket 作为本地救援与自动化入口。所有入口必须调用同一套 watchdog control service，不能各自实现升级、重启、回滚、服务开关逻辑。

## 2. 目标

1. **watchdog 拥有控制面**：升级、回滚、重启、服务开关、状态查询均由 watchdog control service 授权和执行。
2. **多入口一致**：WebUI、Feishu、未来 CLI/socket 生成同一种 `ControlRequest`，获得同一种 operation/event 结果。
3. **明确进程边界**：in-process adapter 可直接调用 control service；跨进程 adapter 必须通过 watchdog 私有 control RPC。
4. **业务语义隔离**：watchdog 可识别控制命令，但不理解 core 业务语义（session、permission、ACP、card rendering 仍属 core）。
5. **热更新友好**：WebUI/control service 在 core child 重启期间仍可访问；Feishu 在 transport 迁入 watchdog 前是降级控制入口。
6. **可审计可恢复**：控制操作有 actor、operation id、状态机、事件 timeline、失败恢复策略。

## 3. 非目标

- P0 不迁移 router/session/ACP/permission 业务逻辑到 watchdog。
- P0 不要求 Feishu transport 全量迁入 watchdog。
- P0 不开放公网 WebUI；默认仅 loopback。
- P0 不要求完整 CLI 产品化，但需要私有 control RPC 作为跨进程 adapter 基础。

## 4. 总体架构

```text
sebas watchdog
  ├── control service                      # 唯一控制事实来源
  │     ├── authorization / confirmation
  │     ├── operation state machine
  │     ├── updater runner
  │     ├── core supervisor
  │     ├── service manager
  │     └── event timeline / audit
  │
  ├── adapters
  │     ├── WebUI adapter                  # watchdog-hosted, primary UX
  │     ├── Feishu adapter/proxy           # early: core-hosted proxy; later: watchdog-hosted broker
  │     └── CLI/socket adapter             # later public client; private RPC introduced early
  │
  ├── managed services
  │     ├── core child: sebas run
  │     ├── updater subprocess: sebas update ...
  │     ├── WebUI HTTP server
  │     ├── LLM gateway server
  │     └── Feishu transport broker（later）
```

## 4.1 File / module layout

长期建议把当前 `src/watchdog.rs` 拆成目录模块，避免单文件膨胀：

```text
src/watchdog/
├── mod.rs              # public entry: run_watchdog, Watchdog bootstrap
├── supervisor.rs       # core child lifecycle, ready/restart/crash policy
├── control.rs          # ControlService, ControlRequest, operation admission
├── auth.rs             # Actor, authorization, signed assertions
├── confirmation.rs     # confirmation grant lifecycle
├── events.rs           # ControlEvent timeline + audit writer
├── updater.rs          # UpdaterRunner wrapper around `sebas update ...`
├── services.rs         # ServiceManager for WebUI/Gateway/Feishu lifecycle
├── control_rpc.rs      # private Unix-socket JSONL RPC
├── adapters/
│   ├── mod.rs
│   ├── webui.rs        # watchdog-hosted WebUI adapter glue
│   ├── feishu.rs       # Feishu control adapter/proxy glue
│   └── cli.rs          # later public CLI adapter glue
└── transport.rs        # later Feishu opaque transport broker
```

Flat modules retained outside watchdog:

```text
src/update.rs           # one-shot update command implementation
src/upgrade.rs          # version install/checksum/rollback primitives
src/ipc.rs              # watchdog-core child lifecycle/transport protocol
webui/*                 # HTTP UI crate/templates/routes
router/*                # core command/session router
```

Dependency decision for WebUI: the `webui` crate must not depend on the root `sebas` crate directly, or it creates a reverse dependency. Use one of these approaches:

1. P2 preferred: keep WebUI server startup/glue in root/watchdog, and pass a small trait object/client into `webui` (`dyn WebUiControlClient`) rather than exposing `ControlService` concrete type to the `webui` crate.
2. Later option: extract shared DTOs to a tiny `control-types` crate if both `webui` and root need concrete request/response types.

Do not make `webui -> sebas` a direct dependency.

## 5. Control transport / adapter hosting

`ControlService` 是 watchdog 进程内对象。adapter 有两种托管模式：

### 5.1 In-process adapter

例如 watchdog-hosted WebUI：

```text
HTTP handler -> ControlService method
```

该路径仍必须构造 `Actor`、通过授权、使用 confirmation token，不能绕过 control service。

### 5.2 Proxy adapter

例如 Feishu transport 尚在 core child 时：

```text
core Feishu parser -> watchdog private control RPC -> ControlService
```

因此私有 control RPC 必须在 Phase 1 引入，而不是等到 CLI 阶段。CLI 可以 Phase 6 才产品化，但 RPC 作为跨进程 adapter 边界必须提前存在。

### 5.3 私有 control RPC 草案

传输：优先 Unix domain socket；Linux 路径：

```text
$XDG_RUNTIME_DIR/sebas/control.sock
# fallback: <data_dir>/control.sock
```

权限：

- socket 文件默认 `0600`。
- watchdog 启动时生成 per-instance secret。
- 非 in-process adapter 调用 RPC 时必须提供启动 secret 或等价 authenticated channel。
- `Actor::System` 只允许 watchdog 内部构造，RPC client 不能提交 System actor。

JSONL envelope：

```json
{
  "version": 1,
  "request_id": "req_...",
  "actor_assertion": { "type": "..." },
  "confirmation": { "token": "..." },
  "request": { "cmd": "update", "kind": "release", "dry_run": false }
}
```

响应：

```json
{"type":"accepted","operation_id":"op_...","status":"running"}
{"type":"rejected","code":"unauthorized","message":"..."}
```

事件获取：

- P1 可用 polling：`operation.get` / `events.since(seq)`。
- 后续可加 stream/subscription。
- adapter 必须能在 accepted 后通过 `operation_id` 补取早期事件，避免订阅竞态。

## 6. 可信 actor 边界

`Actor` 不能由不可信 client 任意声称。watchdog 只接受经过可信边界构造或验证的 actor。

```rust
pub enum Actor {
    WebUi { principal: WebUiPrincipal },
    Feishu { principal: FeishuPrincipal },
    Cli { uid: u32 },
    System,
}

pub struct FeishuPrincipal {
    pub open_id: String,
    pub chat_id: Option<String>,
    pub verified_by: ActorVerifier,
}
```

### 6.1 WebUI actor

- loopback unauthenticated mode 只能产生 low-risk/local actor。
- 启用登录后，actor 必须绑定 authenticated session、role/capabilities、session expiry。
- 非 loopback 必须启用 secure mode。

### 6.2 Feishu actor

两种阶段：

1. **core-hosted Feishu proxy（早期）**：core 不能直接声称 owner。它必须提交短期 signed assertion：
   - Feishu principal（open_id/chat_id）
   - canonical action + normalized args
   - issued_at / expires_at
   - nonce
   - watchdog instance id
   - signature / startup secret MAC
2. **watchdog-hosted Feishu broker（后期）**：watchdog 自己验证 Feishu inbound，并在本进程构造 Feishu actor。

### 6.3 禁止规则

- RPC client 不能传 `Actor::System`。
- Feishu owner 身份必须由 watchdog 验证配置/签名，不能信任 core 提供的 open_id 字符串。
- replayed assertion、过期 assertion、参数被替换的 assertion 必须拒绝。

## 7. Authorization 与 confirmation

控制请求内部必须包含授权上下文与确认授予，不能依赖 adapter-local boolean。

```rust
pub struct AuthorizedControlRequest {
    pub request_id: String,
    pub actor: Actor,
    pub request: ControlRequest,
    pub confirmation: Option<ConfirmationGrant>,
    pub idempotency_key: Option<String>,
}

pub struct ConfirmationGrant {
    pub token: String,
    pub action_hash: String,
    pub principal_hash: String,
    pub channel_hash: String,
    pub expires_at: SystemTime,
    pub nonce: String,
}
```

确认 grant 必须：

- 单次使用。
- 短期有效。
- 绑定 authenticated principal。
- 绑定 canonical action 和 normalized params。
- 绑定 adapter/channel/chat。
- 绑定 watchdog instance。
- 支持 double-click/idempotent finalization。

危险操作：

- update release
- update dev
- rollback
- restart core
- service stop/disable/restart
- WebUI bind host 改为非 loopback

## 8. Control API

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

pub enum UpdateTarget {
    ConfiguredDevTarget { name: String },
}

pub enum ManagedService { WebUi, Gateway, Feishu }
pub enum DesiredState { Enabled, Disabled }
```

Core 不作为普通 `ServiceSet` toggle；用 `StartCore/StopCore/RestartCore` 明确表达，避免和 supervisor 自动重启语义冲突。

## 9. Operation state machine

长操作不应同步返回完成结果，而是返回 operation id，并通过 event/status 查询进度。

```text
PendingConfirmation
  -> Accepted
  -> Running
  -> Succeeded | Failed | Canceled | TimedOut
```

冲突策略：

| Running op | Incoming op | Policy |
|---|---|---|
| update | update/rollback/restart-core/stop-core | reject busy |
| rollback | update/rollback/restart-core/stop-core | reject busy |
| restart-core | update/rollback/restart-core/stop-core | reject busy |
| service restart gateway | update | allow unless updater needs gateway |
| service disable webui | any dangerous op | require fallback path check |
| config persist | service/config persist | serialize |

错误码需机器可读：`busy`、`unauthorized`、`confirmation_required`、`confirmation_expired`、`invalid_target`、`timeout`、`updater_failed`、`service_unavailable`。

updater runner 要求：

- process group 启动。
- 有 timeout。
- 超时先 graceful terminate，再 force kill。
- stdout/stderr bounded capture。
- 失败不重启 core。

## 10. Control result / event / audit

```rust
pub enum ControlResponse {
    Accepted { operation_id: String, status: OperationStatus },
    Rejected { code: ErrorCode, message: String },
}

pub struct ControlEvent {
    pub seq: u64,
    pub timestamp: SystemTime,
    pub operation_id: String,
    pub kind: ControlEventKind,
    pub public_message: String,
    pub diagnostic: Option<RedactedDiagnostic>,
}
```

要求：

- events 有单调 seq。
- terminal event 明确 outcome code。
- public_message 可直接给 WebUI/Feishu 展示。
- diagnostic 必须 redaction，不泄漏 token、secret、完整敏感路径。
- privileged action 至少写入 bounded durable audit：time、operation id、actor principal hash、action、authorization result、terminal result。

## 11. WebUI 控制台与安全

WebUI 是主要控制端，建议 watchdog-hosted。

页面：

- `/admin/status`
- `/admin/update`
- `/admin/services`
- `/admin/events`

P0 安全策略：

- 默认 bind `127.0.0.1`。
- 非 loopback 直接 bind 必须启用 secure mode；否则拒绝启动。
- secure mode 至少要求：authenticated session、allowed-origin policy、CSRF protection。
- 若经反代暴露，只信任显式配置的 trusted proxy，不信任任意 forwarded headers。
- mutation endpoint 必须 POST-only。
- cookie-auth mutation 必须 CSRF token；custom header 不能替代 cookie CSRF 全部语义。
- 密码不可明文落盘/日志；优先 env；存储时使用 salted memory-hard hash。
- session 有过期时间；登录有限速/锁定策略。

Loopback unauthenticated mode 是有意识的便利模式，只适合本机/SSH tunnel 场景。

## 12. Feishu 控制 adapter

watchdog control commands：

```text
/upgrade [--dry-run|--dev|--dev --dry-run]
/rollback
/restart
/services
/gateway on|off|restart|status
/webui status
/system
```

core commands 继续归 core：

```text
/new /sessions /switch /resume /cancel /compact /cost /model /cd /provider /settings /btw
```

阶段限制：

| Phase | Feishu physical host | Can issue control | Works if core dead | Progress events | Keeps connection through core restart |
|---|---|---:|---:|---:|---:|
| P1-P3 | core proxy | yes | no | yes, best-effort | no |
| P5 | watchdog broker | yes | yes | yes | yes |

因此 P1-P3 的 Feishu control 是“逻辑上进入 watchdog control plane”，但可用性仍受 core 存活影响。若产品要求 core dead 时仍可 Feishu 控制，P5 不能是 optional。

## 13. Dev update target policy

`UpdateKind::Dev` 不接受 arbitrary raw path。adapter 只能提交 configured target name。

配置示例：

```toml
[watchdog.dev_targets.local]
path = "/home/cupen/workbench/repos-tool/sebas"
allow_remote_feishu = false
```

要求：

- watchdog 侧 canonicalize path。
- 拒绝 symlink escape、路径穿越、非 allowlist 目录。
- 不通过 shell 拼接命令。
- 记录 resolved target/revision 到 audit，但对远端展示做脱敏。
- 远程 Feishu 默认不能指定 dev target，除非配置显式允许。

## 14. Watchdog 自身升级策略

由于 release 通常同时包含 core 与 watchdog 代码，必须明确策略。推荐：

### P0 策略：core restart only + IPC compatibility contract

- `sebas update` 安装新版本并切 `current`。
- watchdog 重启 core child 时使用 `current/sebas run`。
- watchdog 自身暂不 reexec。
- 要求 watchdog-child IPC 保持至少一版向后兼容。
- 若新 core ready 失败，watchdog 必须进入可恢复状态：
  - 记录失败原因；
  - 不把失败当作无限自动重试的成功；
  - 允许用户执行 rollback 或手动恢复。
- 当这次 update 触及 watchdog/control plane 语义时，控制响应必须显式提示 `watchdog service restart required`。

### 后续策略：watchdog self-reexec

- update 完成后，watchdog 可选择 exec `current/sebas watchdog`。
- self-reexec 需要：状态持久化、listener fd handoff 或短暂停机策略、失败回滚。
- 安全修复涉及 watchdog/control plane 时，应提示“需要重启 watchdog/systemd service”。

验收必须覆盖：old-watchdog/new-core compatibility、new core readiness failure、rollback after readiness failure。

## 15. Service semantics

区分 desired state 与 observed state：

```rust
pub enum DesiredState { Enabled, Disabled }
pub enum RuntimeState { Starting, Running, Stopping, Stopped, Failed { reason: String } }
```

- `enabled` 是期望配置，不等于当前运行。
- `persist=true` 必须有原子配置写入与失败回滚；未实现持久化前不能暴露 persist=true，或必须明确返回 unsupported。
- WebUI 从 WebUI 禁用自身时，必须先返回 accepted/done，再停止 task，并提示恢复路径（Feishu 或 local control RPC）。
- Feishu lifecycle control 只有当 Feishu transport 由 watchdog 持有后才可完全生效；早期只能显示 unavailable/forwarding limitation。

## 16. Migration matrix

| Capability | Legacy owner | New owner | Cutover phase | Compatibility |
|---|---|---|---|---|
| update/rollback | router/dispatch -> watchdog IPC | ControlService | P1 | legacy command maps to control RPC |
| WebUI lifecycle | `sebas run --webui` | watchdog ServiceManager | P2/P4 | old flag retained, single-owner port lock |
| Gateway lifecycle | `sebas run --gateway` / gateway cmd | watchdog ServiceManager | P4 | old flag retained, double-start guarded |
| Feishu control parse | core router | core proxy -> watchdog RPC | P3 | core commands unaffected |
| Feishu transport | core run/ws_loop | watchdog broker | P5 | optional, versioned envelope |
| CLI control | none/internal | control RPC client | P6 | RPC exists earlier, CLI public later |

Single-owner invariant：同一服务同一时间只能有一个 owner 绑定端口/连接；迁移期必须有 lock/guard 防双启动。

## 17. 分期策略

```text
Phase 0: updater split hardening
  -> Phase 1: ControlService + private RPC + auth/confirmation/events
      ├── Phase 2: watchdog-hosted WebUI console
      ├── Phase 3: Feishu proxy adapter over control RPC
      ├── Phase 4: ServiceManager lifecycle model
      └── Phase 6: public CLI/socket client
Phase 4 -> Phase 5: Feishu transport broker
```

Parallelization:

- Phase 2 and Phase 3 may proceed in parallel after Phase 1, because both are adapters over the same control API.
- Phase 4 can start after Phase 1 but should coordinate with Phase 2 for WebUI lifecycle ownership.
- Phase 5 depends on Phase 4 if Feishu transport becomes a managed service.
- Phase 6 depends only on Phase 1 because the private RPC already exists.

Recommended phase order:

1. **Phase 0**：稳定 updater 子进程与 rollback/readiness 语义。
2. **Phase 1**：control domain、authorization、confirmation、operation state、private authenticated control RPC、event/audit contracts。
3. **Phase 2**：watchdog-hosted WebUI control console + security baseline。
4. **Phase 3**：Feishu proxy adapter 经 control RPC 调用 watchdog；明确 core-unavailable 限制。
5. **Phase 4**：ServiceManager 管理 WebUI/Gateway/Feishu desired/runtime state。
6. **Phase 5**：可选 Feishu transport broker；若要求 core dead 时仍 Feishu 控制，则变为必选。
7. **Phase 6**：公开 CLI/socket client。

## 18. Cross-phase test strategy

| Layer | Purpose | Location | Test doubles |
|---|---|---|---|
| Unit | pure control/auth/confirmation/state machine | inline `#[cfg(test)]` in `src/watchdog/*.rs` | none or fake clock |
| Integration | watchdog runner/updater/service lifecycle | `tests/watchdog_*_test.rs` | fake updater, fake child, temp dirs |
| Adapter contract | WebUI/Feishu normalize to same request | `tests/watchdog_adapter_*_test.rs`, `webui/tests/*` | fake ControlService/control RPC |
| Security | auth, CSRF, replay, forged actor, path validation | focused tests per module | fake Feishu assertions, fake HTTP |
| E2E smoke | core restart while WebUI/control survives | `tests/watchdog_e2e_*_test.rs` | fake child binary |

Quality gates:

```bash
cargo check
cargo test -p router --test commands_test
cargo test upgrade::tests
cargo test watchdog_
cargo clippy -- -D warnings   # once existing warnings are cleaned or scoped
```

Rules:

- No real Feishu/GitHub/network in unit tests.
- Use fake updater binary/script for timeout/exit-code/readiness tests.
- Use fake Feishu client/assertion verifier for adapter tests.
- Add a watchdog-layer forbidden-type check for core semantic types such as `SessionKey`, `CardState`, `Permission`, ACP session structs, and router mapping internals.
- Config migrations need backward-compatibility tests for legacy flags/config sections.

## 19. Open decisions resolved for implementation

1. **WebUI dependency direction**：do not make `webui` depend on root `sebas`; use trait/client injection or extract DTOs to a small shared crate later.
2. **Phase 2 vs Phase 4 WebUI lifecycle**：Phase 2 may use a lightweight watchdog-owned task handle; Phase 4 refactors it into full `ServiceManager` desired/runtime state.
3. **Feishu Phase 3 routing path**：before Phase 5, control commands are parsed in the existing core-side Feishu path and proxied to watchdog via private control RPC. This is simpler but unavailable when core is dead; Phase 5 moves interception into watchdog broker if higher availability is required.
4. **Phase 5 IPC shape**：prefer a separate logical transport channel/protocol namespace from lifecycle control. It may share the same physical pipe/socket initially, but message envelopes must distinguish `control`, `lifecycle`, and `transport` to avoid semantic coupling.

## 20. 验收标准

- WebUI 与 Feishu 对同一动作生成同一 normalized `ControlRequest`。
- 所有控制入口通过 control service/RPC，不绕过 authorization。
- forged owner、forged System actor、replayed confirmation/assertion 被拒绝。
- update/rollback/restart 互斥状态机有测试覆盖。
- WebUI core dead 时仍可访问 status。
- 非 loopback WebUI 无 secure mode 时拒绝启动。
- watchdog 不引用 permission/session/card rendering 等 core 业务类型。
- Phase 文档明确每阶段 Feishu 可用性限制。
