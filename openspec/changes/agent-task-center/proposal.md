# Proposal: Agent Task Center

## Why

sebas 目前是「单会话」工作台——操作员开一轮会话干活，干完就结束。但真实项目是多步骤、多角色的：一个功能要拆设计/实现/修 bug 几轮，人和多个 agent 要各自领活、互相评审。缺一个「任务」这一层的协作单元来组织这些会话，让操作员和 agent 能看到「项目里还有哪些活、谁在干、进展如何」。

## What Changes

- **新增「任务」概念**：挂在 project 之下、session 之上；一张任务卡 = beads issue，存于项目目录下的 `.beads/` 仓库级 DB。
- **WebUI 新增任务面板**：stage 区在「会话」旁加「任务」标签，三栏看板（待认领 | 进行中 | 评审中）+ 折叠完成区 + 拖拽改状态。
- **任务详情页**：GitHub issue 式双栏（左描述+评论流，右元信息+操作按钮+历史 session 列表）。
- **session ↔ 任务关联**：session_map 表加 `task_id` 列；任务详情页可一键「开一轮会话推进此任务」，复用现有创建对话框。
- **session 终态联动**：session 进入 `Closed`/`Exited` 且最后一轮 turn 正常完成时，自动给关联任务加 `in-review` 标签。
- **实时刷新**：sebas 后端轮询 `bd events tail --since <seq>`（实测 p95 = 330ms），有变化经 WebSocket 推给前端局部更新看板。
- **权限闸门**：`in_review → done` 只能由人或创建时指定的 reviewer agent 推进；assignee 字段 WebUI 层校验 ∈ 已注册用户 ∪ 已配置 agent kind。

## Capabilities

### New Capabilities

- `agent-task-center`：任务作为项目协作单元的核心——任务 CRUD、认领/释放/指派、状态机、看板展示、任务详情页、评论流、beads 引擎接入与初始化、实时刷新、权限闸门。

### Modified Capabilities

- `state-store`：`session_map` 表新增 `task_id` 列（可空 TEXT，存 beads issue id 如 `bd-42`），创建 session 时可选携带；core 不校验 task 存在性，由 WebUI 层验证。
- `agent-workbench`：stage 区新增「任务」标签页与「会话」并列；任务详情页新增「开一轮会话推进此任务」按钮（复用现有创建对话框）。

## Impact

- **新代码**：`sebas-webui/src/beads_client.rs`（异步封装 `bd` CLI）、`sebas-webui/src/task_routes.rs`（HTTP API）、`sebas-webui/src/task_events.rs`（轮询 + WS 推送）、前端任务面板组件（看板/详情页/任务卡）。
- **schema 变更**：`projects.db` 的 `session_map` 表加 `task_id TEXT` 列（`sebas_db::schema` 日期戳版本迁移）。
- **外部依赖**：`bd` CLI ≥ 1.3.4（已在本机 /usr/sbin/bd 验证可用）。
- **存储**：新增 `<project>/.beads/dolt/` 目录（beads 自管理），sebas 不直接读写 Dolt。
- **API**：新增 `/api/projects/{id}/tasks/*` 一组端点（list/create/claim/release/assign/review/close/comment/show）。
- **WebSocket**：新增 `task_updated` 事件类型，载荷为 beads issue 完整快照。
- **性能**：后端每 300ms 轮询一次 `bd events tail`（单次 tail p50 = 65ms，实测支撑）；单 project 任务量级百级内性能无虞。

## Non-goals

- ❌ 任务优先级 / 自定义标签体系（beads 支持但首版 UI 不暴露）
- ❌ 任务依赖图、子任务、 Epic（beads 支持但首版 UI 不暴露）
- ❌ agent 自主轮询队列捡活（首版仅「人派给 agent」，二期再加）
- ❌ 跨机任务同步（`bd dolt push` 由用户自己跑，sebas 不集成；journal per-replica 边界已在 spike 实测确认）
- ❌ 任务评论自动同步 session 对话（评论 = 元讨论，session = 执行讨论，语义约定分离）
- ❌ 在 rail 创建对话框里加「关联任务」选项（二期；首版只有任务详情页一个入口）
- ❌ 任务通知推送到飞书等外部渠道（依赖现有 session-unread-badge 通道）
