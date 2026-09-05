# 提案：工作台对话区输入框按会话绑定 agent（只读），模型可选

## Why

后端语义是"agent 在建会话时定死，模型中程可换"，但工作台对话区底部的输入框（composer）在聚焦会话的 transcript 正下方摆着 agent 下拉——读起来像"给当前对话换 agent"，实际却是"用这个 agent 再开一个新会话"。语义错位误导操作者；预览原型（IA v2）本就没有这个下拉。此外会话数据（`SessionRow`/`SessionDetail`）不暴露 agent kind，"这个会话用的是哪个 agent"在 UI 上无处可看。

## What Changes

- 对话区 composer 改为**双模式**：
  - **跟随模式**（有聚焦会话）：发送 = 向聚焦会话发消息（`POST /api/sessions/{key}/message`）；底部左侧以小号只读文本显示 agent 名，旁边是模型下拉（数据源 = 该会话自己的 `available_models`）。
  - **创建模式**（无聚焦会话，或点击"新会话"chips 显式切入）：agent 下拉 + 模型下拉 + `→ 项目/inbox` 绑定显示；发送 = 创建会话并跳转。
- agent 名展示约束：小号（0.78rem mono、dim 色）、位于输入框底部工具栏，不可交互。
- 删除 composer 顶部的 agent 下拉，并顺带移除底部工具栏里重复绑定的第二个 agent 下拉（现有 leftover bug）。
- wire 层：`SessionInfo` 从 mapping 的 `pending_kind` 带出 `agent_kind`（`None` = 默认 kind），`SessionRow` / `SessionDetail` / summary 的 active_session 同步暴露；会话持久化层补齐该字段的落盘。
- 会话详情页头部 meta 顺带显示同一 agent 名（数据到位后是一行成本）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui/projects`：对话区 composer 的行为要求改为双模式（跟随/创建），跟随模式下 agent 只读、模型可选；`SessionRow` 增加 `agent_kind` 展示字段。

## Impact

- `sebas-webui/frontend`：`workbench-composer.ts`（重写模式逻辑与底部工具栏）、`dashboard.ts`（传递聚焦会话的 agent/model 数据）、`session-detail.ts`（头部 meta 加 agent 名）、`api/client.ts`（类型加 `agent_kind`）。
- `sebas-router`：`SessionInfo` 加字段（`events.rs`），`session_info_for` 填充（`mod.rs`）；`Mapping.pending_kind` 持久化（`state.rs` MappingDto / state-store sessions 表）。
- `sebas-webui/src`：`routes.rs` / `models.rs` 行构造带出字段。
- 无 breaking API 变更（新增可选 JSON 字段，旧客户端忽略）。

## Non-goals

- 不支持会话中途更换 agent（后端语义即不可能，不做假入口）。
- 不改侧栏 IA（不加侧栏新建按钮），创建入口保留在 composer。
- 不实现原型里的权限模式菜单（ask/auto/plan/full），那是独立变更。
- 不改会话详情页输入框行为（它已是纯消息发送）。
