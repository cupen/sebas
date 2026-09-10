## Context

这是一个跨前后端、带破坏性 wire 变更的提案。已通过 grilling 锁定全部决策；本设计只记录「怎么实现」的关键选择与取舍，「为什么」见 proposal.md。

关键现状（已用沙箱实证，非凭记忆）：

- **根因（发不出消息）**：`sebas-dispatch/src/state.rs:318` 的 `route_text` 在 `MappingState::Spawning` 分支用 `pending_kind.is_some() || pending_model.is_some()` 判定「这是等待首条消息的占位会话」；`sebas-webui/src/session_backend.rs:263` 的 `parse_acp_kind` 只认 `acp:<slug>`，导致 `backend=null`/`"acp"` 创建的占位两个字段全 `None` → 消息被 `Enqueued` 进无人 drain 的队列（实证：`agent="claudecode"` 路径正常，两条默认路径卡死）。
- **wire 现状**：`POST /api/sessions` 的 `backend` 接受 `null`/`"acp"`/`"acp:<slug>"`/`"native"`；`AgentKindInfo` 有 `name`/`slug`/`reachable`/`cause`/`version`，其中 `name` 目前等于 slug（无独立 display）。
- **project 现状**：`projects.json`（`{path,name,added_at,branch,branch_at}`）+ SQLite `projects` 表（`path TEXT PRIMARY KEY`），path 同时是主键与 wire 标识符；`session_map` 存 `project_dir` 全路径。
- **focus 现状**：`session_detail` 的 GET 已会 `set_focus`（`api.rs:148`），`create_session` 也会（:500）——focus 同步的后端语义已在，缺的是前端 rail 创建占位后不 navigate 到深链而是留在工作台时 composer 模式的跟随。

## Goals / Non-Goals

**Goals:**

- 修复占位会话「消息被 Enqueued 进死队列」的根因（accepted ≠ delivered 是 bug）。
- wire 全面改用 agent id / project id，driver 名、raw path 不再上 wire。
- `/api/agents` 成为 agent 可用性唯一真源（含 native 一行）。
- 会话 agent 不可变成为 spec invariant，UI 有锁定提示。
- 项目级 `default_agent` 落 `projects.json`。
- rail 补项目/会话删除入口。

**Non-Goals:**

- 创建时 ACP `model` 下发（下一个 change）。
- `/api/summary` 收敛（`execution_bodies` 删除等，并行 change）。
- Feishu 侧命名统一。
- 配置模型清理（`[acp.agents.*]` 的 `driver` 字段保留为静态 launch 策略，不上 wire）。

## Decisions

### D1：占位身份用显式标记，与 kind/model 解耦

- **选择**：`MappingState::Spawning` 增加 `awaiting_first_prompt: bool`（或等价的占位标记字段），`web_create_placeholder` 置 true、真实 spawn 置 false；`route_text` 的 Spawning 分支只看这个标记，不再推断。
- **序列化兼容**：`#[serde(default)]`，旧 `state.json`/`session_map` 记录默认为 false；旧占位记录重启后按「非占位」处理（极端边缘，可接受，因旧占位本来就永远卡死）。
- **备选**：让 `parse_acp_kind` 把裸 `"acp"`/`null` 解析成 default kind（治标，下次新增语法又踩坑）；或创建时归一化到具体 slug（改变快照展示语义）。均否决。

### D2：wire 字段重命名为 `agent` / `project_id`，一次性切换

- **选择**：`POST /api/sessions` 的请求体从 `{prompt, project_dir, backend, model}` 改为 `{prompt, project_id, agent, model}`；`agent` 必填，值域 = `{[acp.agents.*] 键名} ∪ {"native"}`；旧 `backend` 字段与旧值一律 400。`POST /api/projects/{id}/remove` 等端点的路径参数从 encoded path 改为 `project_id`。
- **spec 标注 BREAKING**：本地产品无外部消费者，所有内部调用点（前端 api client、CLI、e2e helpers、fake-claude 沙箱）在同一 change 内切换。
- **project_id 生成**：`proj-<12hex>`，从 canonicalized path 的 SHA-256 取前 12 hex。重启/重建注册机后同路径同 id，无需持久化分配器。
- **备选**：双轨过渡（backend/agent 并存）——本地产品无必要，维护成本高，否决。

### D3：`/api/agents` 扩展为唯一真源，形状 `{id, display, reachable, cause?, version?}`

- **选择**：`GET /api/agent-kinds` 改名 `/api/agents`；响应项从 `AgentKindInfo{name,slug,...}` 改为 `{id, display, reachable, cause?, version?}`；`native` 作为 `id="native"` 的一行进同表，其 `reachable/cause` 来自 native 内核的凭据探测（与今天 `execution_bodies` 的 native 项同源）。`driver` 字段不出现在响应。
- **display 来源**：配置可选 `display` 字段；缺省时 `driver="claude"` → "Claude Code"、`driver="acp"` → 键名本身、native → "Native Kernel"。
- **CLI 同步**：`sebas agent-kinds list` 复用同一探测函数，输出列跟随新形状。
- **备选**：保留 agent-kinds + execution_bodies 双源——情报重复会漂移，否决。

### D4：会话 agent 不可变 = spec invariant + UI 锁定提示

- **选择**：spec 新增「Session agent binding is immutable」requirement；后端不新增 `set_session_agent` 端点（不存在即不可变），任何变相改 agent 的请求（如 create 同 key）由既有 typed rejection 覆盖。UI 在 composer 跟随模式与会话详情 head 的 agent 标签旁加锁图标 + title「chosen at creation」。
- **依据**：`acp-session-mapping` 的 resume 依赖 agent 自己的真实 session id，跨 agent 无法恢复——不可变是技术事实的 spec 化，不是新增限制。

### D5：项目级 `default_agent` 落 `projects.json`，创建会话时更新

- **选择**：`projects.json` 与 SQLite `projects` 表各加 `default_agent` 列；`POST /api/sessions` 成功创建带 `project_id` 的会话后，把该项目的 `default_agent` 更新为本次的 `agent`；composer 进入某项目的创建模式时从 `GET /api/projects` 的响应读 `default_agent` 预选。
- **备选**：前端 localStorage 记忆（跨浏览器不一致）；全局默认（项目间互相污染）。均否决。

### D6：focus 同步靠后端既有语义 + 前端不再跳深链

- **选择**：rail「+」创建占位会话后**留在工作台路由**（不再 `navigate(/sessions/{key})`），依赖 `create_session` 的 `set_focus` 让 summary 的 `active_session_key` 驱动 composer 进入跟随模式；`session_detail` 的 GET 既有的 `set_focus` 行为覆盖深链直达路径。无需新增 switch 调用。
- **依据**：后端 focus 语义已完整（`api.rs:148,500,589`），缺的是前端创建后不该跳走——跳走再跳回才是 focus 错位的来源。

## Risks / Trade-offs

- [BREAKING 变更集中在一个 change，评审面大] → 所有调用点同批切换 + e2e 全量回归；spec 用 BREAKING 标注。
- [`awaiting_first_prompt` 标记的旧记录兼容] → `#[serde(default)] = false`，旧占位记录（本就卡死）重启后仍卡死，用户重新创建即可——可接受，因为这些会话本来就不可用。
- [project_id 是哈希，URL 不可读] → 有意取舍：可读性让位给稳定性；UI 展示名用 `name`/`display`，id 只出现在 URL 与 wire。
- [`/api/agents` 承载 native 探测，与 agent-kinds 的 binary 探测语义不同源] → native 行的 cause 来自凭据探测而非 binary 探测，在 display 与文档中如实区分。
- [删除 `/api/agent-defaults` 影响 settings 弹窗的 provider/model 默认展示] → 该展示挪到 settings 的 provider 管理区（既有 `/router/api/providers` + defaults 端点），本 change 只删 agent-defaults 端点本身。

## Migration Plan

1. **数据库**：`projects` 表加 `id` 与 `default_agent` 列（migration 版本 +1）；`session_map` 的 `project_dir` 列保留（内部仍用 path spawn），但 webui API 层不再透出。
2. **`projects.json`**：格式加 `id`、`default_agent` 字段；旧文件缺字段时启动回填（id 从 path 重算，default_agent 为 null）。
3. **wire**：无灰度——前后端同一 commit 切换；旧 `backend` 请求一律 400。
4. **回滚**：revert 提交；数据库新增列向后兼容（旧代码不读新列）。
