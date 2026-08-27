## Why

WebUI 目前只能通过 Dashboard 管理已有 session（列表/详情/关闭），没有从
WebUI 直接发起新工作的入口。用户需要在浏览器里就能：
1. 选择一个 git 仓库作为项目
2. 在该目录下启动 Claude Code 会话
3. 在同一个 WebUI 界面里与 agent 对话

已有的 `agent.html` 模板和 `Mapping.project_dir` 字段就是为此准备的骨架，
但 Rust 后端从未连通过。填上这个缺口，让 WebUI 从「查看器」变为
「项目工作台」（类似 DeepSeek 的 Project/Codex 模式）。

## What Changes

- **新路由 `/agent`**：项目导向的 agent 聊天页面（替代 session 列表的
  纯文本模式，支持项目目录选择 + 侧栏会话列表 + 消息 composer）。
- **新路由 `/api/agent/projects`**：输入 git 仓库路径，创建新的 agent
  会话（`web_spawn(prompt, project_dir)`），其中 `project_dir` 是仓库
  路径，`prompt` 自动设为「在 {project_dir} 下工作，请先了解项目结构」。
- **新路由 `/api/agent/{key}/message`**：向已有 agent 会话发消息。
- **新路由 `/agent/{key}` 和 `/agent/{key}/timeline`**：agent 会话详情
  与 timeline 增量更新。
- **SessionRow 模型**新增 `project_dir` 和 `prompt_preview` 字段。
- **WebUI 侧栏导航**新增「Agent」tab，与现有 Dashboard/Sessions 并列。
- **BREAKING**：`/agent` 路由路径取代了 `agent.html` 的 dead template，
  但无外部 API 兼容性影响（之前从未注册过）。

## Capabilities

### New Capabilities

- `webui/projects`: Project-oriented agent workspace. 用户从 WebUI 选择
  git 仓库目录 → 在该目录下启动 Claude Code 会话 → 在 WebUI 中完成
  从代码理解到修改的完整工作流。

### Modified Capabilities

（无 — 新路由独立于现有 webui 路由体系，不修改已有行为）

## Impact

- `webui/src/routes.rs`：新增 `/agent` 页面路由、`/api/agent/*` API 路由。
- `webui/src/server.rs`：注册新路由到 axum Router。
- `webui/src/models.rs`：`SessionRow` 新增 `project_dir` / `prompt_preview`。
- `webui/templates/agent.html`：将现有 dead template 接入真实数据。
- `webui/templates/agent_timeline.html`：timeline 片段（可能已存在）。
- `webui/templates/base.html` 或 `sidebar_active.html`：新增 Agent 导航 tab。
- `router/src/state.rs`：`Mapping` 的 `project_dir` 已存在，无需改动。
- 可选的静态 CSS 调整（`.codex-*` class 可能已在 `style.css` 中）。

## Non-goals

- 不做文件系统浏览（用户手动输入路径，暂不提供目录选择器）。
- 不做 git 仓库自动发现或扫描（只接受显式路径）。
- 不改变现有 Dashboard/Sessions 路由的行为。
- 不涉及 agent 子进程的 IPC 或文件系统监控——纯 WebUI 侧改动。