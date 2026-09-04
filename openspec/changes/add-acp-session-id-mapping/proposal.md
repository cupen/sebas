# add-acp-session-id-mapping

## Why

`add-opencode-acp` 的真实验收证明：ACP resume 的机制已跑通，但 opencode 的 `session/load` 需要**它自身的 ACP session id**，而 sebas 目前只持久化自造的 uuid 路由 id——真实验收中 `LoadSessionRequest(uuid)` 被 opencode 拒绝（`Internal error: OpenCode service failure`），只能诚实回退 fresh。要让 resume 真正恢复会话，必须把「路由 session_id ↔ 真实 ACP session id」的映射持久化，并在 resume 时用真实 id 加载。

## What Changes

- **ACP 驱动上报真实 ACP session id**：`AcpDriver` 在握手完成时把 `NewSessionResponse.session_id` / `LoadSessionResponse` 对应的 ACP 会话 id 随握手信号一并上抛（`handshake` 从 `(String, bool)` 扩展为 `(String, bool, Option<String>)`：路由 id、resumed、acp_session_id）。
- **持久化映射**：在现有 session 记录（`state.json` 的 `MappingDto`，`add-state-store` 落地后为 SQLite sessions 表）中新增 `acp_session_id` 字段；fresh 建会话时写入，resume 时读取。
- **resume 用真实 id**：`acp_resume_and_activate` / driver `session/load` 改用映射里的 `acp_session_id`（而非自造 uuid）；load 失败仍走诚实回退（`resumed=false`）。

## Capabilities

### New Capabilities

- `acp-session-mapping`: ACP 会话 id 与 sebas 路由 id 的映射生命周期——fresh 时建立、持久化、resume 时读取、无映射时的回退语义。

### Modified Capabilities

- `acp-driver`: `session/load` 的目标 id 从「路由 id 恒等于 ACP 会话 id」改为「可用真实 ACP session id 加载」；驱动上报 `acp_session_id`。

## Non-goals

- 不做 ACP 会话 fork/list/close/delete 等其他 session 能力扩展
- 不做历史会话的 id 回填迁移（旧记录无 `acp_session_id`，load 失败即 fresh fallback，开发阶段可接受）
- 不改飞书侧、不改 webui 展示
- 不实现 opencode 专属逻辑（映射机制对任意原生 ACP agent 通用）

## Impact

- `sebas-acp/src/acp_driver/`：握手信号扩展 + load 目标 id 逻辑
- `sebas-acp/src/claude/manager.rs`：`SpawnOutcome` 增加 `acp_session_id`；会话表 `SessionMeta` 携带
- `src/session_boot.rs` / `src/dispatch.rs`：resume 时读取映射、spawn 时写入映射
- `sebas-router/src/state.rs`（MappingDto）：新增字段（`add-state-store` 落地后同步到 SQLite 表）
- 依赖：无新增