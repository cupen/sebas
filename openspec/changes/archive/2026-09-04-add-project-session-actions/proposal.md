## Why

项目注册与会话创建的操作路径不对。点击 PROJECTS 右侧的 `+` 弹出的是内联文本输入框而非目录选择器，项目行上的 `+` 仅「聚焦项目」而非建会话（因为 `POST /api/sessions` 要求 `prompt` 必填），且完全没有归档能力——会话不可移除、History 组目前装的是飞书收件箱而非归档。这些操作缺口让 workbench 的「项目即工作单位」落地不完整。

## What Changes

- **添加项目弹出目录选择器**：`+` 按钮改弹出 `wa-dialog`，内含目录浏览器（服务端 `GET /api/fs/browse` 递归列目录）和手动路径输入，选中即注册，项目名取自目录名，发起 git 分支探测并在项目行中显示。
- **项目行独立「新建会话」按钮**：每个项目行右侧加 `+` 按钮，点击立即创建空会话（0-turn placeholder），不需要用户先输入第一条消息。会话被激活并展开，项目被选中。
- **会话行「归档」按钮**：每个会话名右侧加归档按钮，点击将会话移入 History 组（只读不可操作）。后端新增 `archived_at` 字段标记移除时间。
- **History 组改为归档桶**：原 History 组（飞书 inbox）独立为「Inbox」组。归档会话按 `archive_retention_days`（默认 30 天，config 可配）过期自动删除。
- **后端 `POST /api/sessions` 支持空 prompt**：`prompt` 字段改为可选；省略时创建 placeholder 会话（不拉 ACP 子进程），首条消息时再真正 spawn。

## Capabilities

### New Capabilities
- `project-session-actions`: 项目的目录选择器注册、会话的零提示创建、会话的归档与过期清理——workbench 操作面的补齐。

### Modified Capabilities
- `webui`: 路由面新增 `GET /api/fs/browse` 目录浏览 API；`POST /api/sessions` 的 `prompt` 字段改为可选；`session_row` 新增 `archived_at` 字段；`config` 新增 `[webui] archive_retention_days` 配置项。侧栏 PROJECTS 的添加/新建/归档交互语义变更。
- `agent-workbench`: 新增操作面——项目注册的目录选择器路径、会话的零提示创建、归档与过期清理；History 组重定义。

## Impact

- `sebas-webui/src/`：`api.rs` 扩展 `CreateSessionRequest` 使 `prompt` 可选；`projects.rs` 新增 `fs_browse` 目录浏览；新增 `archive.rs` 模块处理归档持久化、过期清理；`routes.rs` 挂载 `GET /api/fs/browse`；`config.rs` 扩展 `WebUiConfig` 增加 `archive_retention_days`。
- `sebas-webui/frontend/src/`：`project-rail.ts` 重写添加项目为 dialog + 目录浏览器；每项目行增「新建会话」按钮；每会话行增「归档」按钮；`shared-state.ts` 增归档/恢复状态；`sessions.ts` 增归档标签与只读态。
- 新文件 `~/.sebas/archive.json`（webui 独占，记录归档会话及其元数据）。
- 无新增 Rust 依赖，router 与 feishu 侧不改。

## Assumptions & Decisions

1. **目录选择器用服务端 `GET /api/fs/browse` + 手动路径回退**：浏览器 `showDirectoryPicker()` 无法返回绝对路径，而 spawn 会话需要真实路径。服务端端列目录是最可靠的方案。
2. **History 组改为归档桶，飞书收件箱独立为 Inbox 组**：预览设计的 History 语义就是归档，不应与飞书 unbound 会话混排。
3. **归档保留期默认 30 天，config 可配置 (`[webui] archive_retention_days`)**：清理在 webui 启动时及每次列表请求时触发。