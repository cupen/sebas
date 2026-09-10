## Why

工作台输入框能敲字但消息「发出去后 agent 永远不响应」——根因是占位会话（0-turn）用 `pending_kind.is_some()` 推断「这是等待首条消息的占位」，而 `parse_acp_kind` 只认 `acp:<slug>`，导致 `backend=null`（rail「+」）或 `backend="acp"`（composer 默认）创建出的占位会话**丢掉占位身份**，首条消息被 `Enqueued` 进一个无人 drain 的死队列。同时 wire 上 driver 名（`acp`/`native`）泄漏给前端，「前端看不懂 `acp` 是哪个 agent」是真实心智摩擦。

## What Changes

**BREAKING**：
- **`POST /api/sessions` 的 `backend` 字段退役，新字段 `agent` 必填**，值 = `{[acp.agents.*] 配置名} ∪ {"native"}`（`"acp"` / `"acp:<slug>"` / `null` 一律 400）。
- **`/api/agent-kinds` 改名为 `/api/agents`**，响应字段改为 `{id, display, reachable, cause?, version?}`，`driver` 不上 wire；`native` 作为一行进同表（id=`"native"`）。
- **`/api/agent-defaults` 删除**（provider/model 默认已挪到 settings 里的 provider 管理；agent 默认改为项目级）。
- **Project 获得稳定 id `proj-<12hex>`**（从规范化 path 确定性哈希）；`projects.json` 与 SQLite `projects` 表均加 `id` 列；wire 上引用项目一律用 `project_id`，`project_dir` 不再出现。
- **`projects.json` 新增 `default_agent` 列**（用户在某项目下最近一次创建会话选用的 agent id）；同一项目下次激活时 composer 预选它。

**行为修复与不变量：**
- `route_text` 用显式 `awaiting_first_prompt` 标记判定占位（不再靠 kind/model 推断），占位会话首条消息必触发 spawn——**这是「发不出消息」的真正修复**。
- 会话创建后 agent **不可变**（spec invariant）；UI 在 composer/会话详情给锁定提示；任何「改 agent」的请求被后端类型化拒绝。
- rail 增加项目删除（带确认）与会话删除（inactive 直删/active 需确认）入口，复用既有 `remove`/`close` 端点。

## Capabilities

### Modified Capabilities

- `agent-workbench`：新增「Rail 项目/会话删除入口」「占位会话首条消息必达」「会话 agent 不可变 + 锁定提示」「项目级默认 agent」requirements；修订「Composer promises only what the process can do」补 accepted-must-deliver 语义。
- `webui`：HTTP route surface 修订（`backend`→`agent`、`/api/agents`、`/api/agent-defaults` 删除、project 端点改 `project_id`）；「Session dashboard and focus semantics」补 focus 跟随深链访问。
- `project-session-actions`：新增 rail 删除（项目+会话）与项目级默认 agent 的 requirement。
- `agent-driver`：新增「driver 是配置层概念，不上 wire」requirement；「Reachability」修订为 `/api/agents` 统一承担。

## Impact

- **Rust 后端**：`src/agent_backend.rs`（hint 校验、路由）、`src/core_channel/server.rs`（请求字段改名）、`sebas-dispatch/src/state.rs`（占位标记）、`sebas-dispatch/src/engine/mod.rs`（web_spawn/web_create_placeholder）、`sebas-webui/src/api.rs`（handler 字段、新 `/api/agents`）、`sebas-webui/src/agent_kinds.rs`（→ agents，字段扩展）、`src/projects.rs` / `src/sebas_state/`（id 列 + 迁移）、`src/config.rs`（default_agent 读写）。
- **前端**：`api/client.ts`（类型与端点）、`views/workbench-composer.ts`（agent 下拉数据源、必填校验、跟随模式锁定提示）、`views/project-rail.ts`（删除入口、default_agent 预选）、`views/session-detail.ts`（agent 锁定提示）、`views/settings-modal.ts`（移除 agent-defaults 引用）。
- **e2e**：`tests/testsuite-webui/tests/` 的 session-roundtrip / session-mgmt / projects / models 各补一条旅程。
- **CLI**：`sebas agent-kinds list` 输出列随 `/api/agents` 新形状同步。
- **无外部消费者**（本地产品），BREAKING 影响面为内部调用点，全部在同一 change 内切换。

## Non-goals

- **创建时 ACP `model` 生效**（spawn 后、首 prompt 前下发 set_config_option）——独立任务，下一个 change 做；本 change 只保证 wire 里 `model` 字段仍存在且不报错。
- **`/api/summary` 收敛**（`execution_bodies` 删除、recent_sessions 精简）——并行可做，但不在本 change。
- **Feishu 侧 agent/driver 命名统一**——IM 桥是另一摊，不阻塞本 change。
- **会话硬删除**（抹除磁盘痕迹）——rail「删除」即既有 close 语义。
- **native 内核的模型选择**——已覆盖，不动。
