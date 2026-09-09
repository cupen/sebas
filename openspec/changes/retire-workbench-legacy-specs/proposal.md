## Why

三个 legacy capability——`agent-workbench`、`project-session-actions` 与嵌套的 `webui/projects`——的行为语义已全部被新 `workbench` capability（consolidate-workbench-session-actions 归档时创建）或新 SPA `webui` 工作台接管。consolidate 归档已把三份主 spec 从 `openspec/specs/` 物理移除；本 change 补上显式的退役记录（REMOVED delta），使目录树的删减在 change 历史中可追溯、每一条 requirement 的去向有据可查。

## What Changes

- 记录 `agent-workbench`（18 条 requirement）全部退役：语义迁入 `workbench`，逐条 Migration 见 REMOVED delta。
- 记录 `project-session-actions`（5 条 requirement）全部退役：其细化文本即 `workbench` 的对应 requirement 基座。
- 记录 `webui/projects` 嵌套 spec（7 条 requirement）退役：属 HTMX 时代旧面（`GET /agent`、`/api/agent/projects`），已被 `webui/spec.md` 的 SPA 工作台（`GET /`、`/api/projects`）取代。
- 无新行为、无 spec 文件变更——三个旧 spec 文件已随 consolidate 归档删除，本 change 仅为记录。

## Capabilities

### New Capabilities

### Modified Capabilities

### Removed Capabilities
- `agent-workbench`: 并入 `workbench`，目录退役
- `project-session-actions`: 并入 `workbench`，目录退役
- `webui/projects`: 被 `webui` SPA 工作台取代，嵌套目录退役

## Impact

- 变更仅落在 `changes/` 归档记录，`openspec/specs/` 无新增改动（已删除的三个 spec 不复活）。
- **Non-goals**：不复活/重写任何已删 requirement；不做 `workbench` 内容修订；`workbench` 的命名归位（批次 E）不在本 change。
