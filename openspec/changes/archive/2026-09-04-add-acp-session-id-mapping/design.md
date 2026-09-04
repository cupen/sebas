# design — add-acp-session-id-mapping

## Context

`add-opencode-acp` 已把 ACP resume 机制落地（握手信号 + `session/load` + 诚实回退），真实验收暴露：opencode 的 `loadSession` 按**它自己的 ACP session id** 解析，sebas 目前只持久化自造的 uuid 路由 id，resume 时用 uuid 发 load 被拒（`Internal error: OpenCode service failure`），只能回退 fresh。本 change 补上「路由 id ↔ 真实 ACP session id」的映射，使 resume 真正恢复会话。spec 见 `acp-session-mapping`（新）+ `acp-driver`（ADDED）。

关键结构事实（已核实源码）：

- `SessionManager::spawn` 经 `DriverConfig` 传 `session_id` + `resume` 给驱动；`AcpDriver` 现在把 `routing_id` 直接当 load 目标（`LoadSessionRequest(routing_id)`）。
- 驱动握手信号 `handshake: Option<oneshot::Receiver<(String, bool)>>`（路由 id + resumed），由 `add-opencode-acp` 引入；`NewSessionResponse.session_id` / load 成功后的会话 id 目前被驱动丢弃，未上抛。
- session 持久化在 `sebas-router/src/state.rs` 的 `MappingDto`（key=web/feishu 会话 key → session_id + last_active），`add-state-store` 落地后迁移到 SQLite sessions 表。
- resume 入口：`session_boot::acp_resume_and_activate(mgr, router, key, old_sid, ...)` → `mgr.resume_session(kind, cmd, ..., old_sid)` → `SessionStart::Load(old_sid)`。

## Goals / Non-Goals

**Goals:**
- 驱动上报真实 ACP session id（fresh 与 load 两条路径）
- 映射持久化到 session 记录，重启后 resume 可用真实 id 加载
- 无映射/load 失败 → 维持诚实回退（`resumed=false`）

**Non-Goals:**
- 不扩展 ACP session 其他能力（fork/list/close）
- 不做历史记录 id 回填（旧记录无 `acp_session_id`，load 失败即 fresh，开发阶段接受）
- 不引入 opencode 专属逻辑（机制对任意原生 ACP agent 通用）

## Decisions

### D1 `SessionStart::Load` 携带可选 acp_session_id

把 load 目标与路由 id 解耦：

```rust
pub enum SessionStart {
    New,
    Load { routing_id: String, acp_session_id: Option<String> },
}
```

`acp_session_id` 为 `Some` 时驱动用它对 `session/load`（opencode 认的 id）；`None` 时保持今天行为（用 routing_id 尝试 load，兼容无独立 id 的 agent / 旧记录）。`DriverConfig` 增加 `load_session_id: Option<String>`，manager 从 `SessionStart::Load` 填入。**改 `SessionStart` 的代价**：公开枚举，dispatch / session_boot / 测试共 4-5 处调用点需同步，属本 change 必要涟漪。

### D2 握手信号扩展为 (routing_id, resumed, acp_session_id)

`add-opencode-acp` 的 `handshake` 从 `(String, bool)` 扩为 `(String, bool, Option<String>)`：

- fresh：`(routing_id, false, Some(new_session_id))`（`NewSessionResponse.session_id`）
- load 成功：`(routing_id, true, Some(loaded_id))`
- load 失败回退 fresh：`(new_uuid, false, Some(new_session_id))`
- Claude 驱动（`handshake=None`）：`acp_session_id` 恒 `None`（routing id 即会话 id，无需映射）

`SpawnOutcome` 同步增加 `acp_session_id: Option<String>`，`SessionMeta` 携带供 manager 内部一致。

### D3 映射持久化进 session 记录

`MappingDto` 增加 `#[serde(default)] acp_session_id: Option<String>`：

- 建会话（`acp_spawn_and_activate` 成功）→ `router.activate` 时写入 `acp_session_id`
- resume（`acp_resume_and_activate`）→ 按 `old_sid` 查映射取 `acp_session_id`，传给 `SessionStart::Load`；无则 `None`
- `add-state-store` 落地后：同字段进 SQLite sessions 表（映射与 session 记录同表，单写者不变）

**回滚/兼容**：`#[serde(default)]` 使旧 `state.json`（无字段）读为 `None`；旧二进制读新 `state.json` 遇未知字段 serde 默认忽略。无需数据迁移。

### D4 回退语义不变

无映射 / load 拒绝 → fresh fallback（新 uuid、`resumed=false`），与 `add-opencode-acp` 既有语义一致；原映射**保留在存储**（旧会话仍可被未来 load 寻址），不因一次失败而抹除。

## Risks / Trade-offs

- [SessionStart 枚举破坏面] → 4-5 处调用点同步，编译期暴露，测试兜底（`resume_session_test` 等）
- [映射陈旧：agent 端会话被删] → load 失败 → fresh 回退，映射保留但不再命中；可接受（与 Claude 的 ResumeRejected 语义一致）
- [opencode 重启后其磁盘 session store 是否保留] → 取决于 agent 自身持久化；sebas 只保证"用真实 id 尝试"，不保证 agent 侧必然可恢复

## Migration Plan

- 部署：加字段（serde default）+ 驱动上抛 + resume 读映射，向后兼容；新会话自动开始记录映射
- 回滚：旧二进制读新 state.json 忽略未知字段；`SessionStart` 枚举改动随二进制整体回滚

## Open Questions

- opencode 重启进程后其 ACP session 是否仍可被 load（取决于其磁盘 store）：实现后用真实 opencode 冒烟确认（沿用 `docs/acp-opencode-smoke.md` 的 resume 步骤）
- `add-state-store` 归档时 `acp_session_id` 进 SQLite 表的具体列：届时同步（本 change 先在 MappingDto 落地，保持单写者）