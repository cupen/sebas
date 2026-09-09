## Why

工作台/项目/会话操作面分散在三个 capability——`agent-workbench`、`project-session-actions` 与嵌套的 `webui/projects`——其中「0-turn 占位会话」「归档 / History / Inbox」「目录选择器」语义重复（五个 requirement 几乎逐字重叠），且 `webui/projects` 描述的是一套已被新 SPA 工作台取代的旧接口（`GET /agent`、`POST /api/agent/projects` 属 HTMX 时代，新面已归 `webui/spec.md` 的 `GET /`、`/api/projects`）。三处并存导致读者无法判断哪个是行为真相源。

## What Changes

- **project-session-actions 升格为工作台行为真相源并改名 `workbench`**：其规范文本更新最全（`browse-dirs` 懒加载、Windows 路径往返修复、API 创建空会话场景仅在此），将其 spec 作为新 capability 基座。
- **`agent-workbench` 并入 workbench 后归档**：其中独有的 requirement（会话归属、未读转折缝、composer 承诺边界、原生内核执行/门控审批、模型选择覆盖原生内核等）迁入新 `workbench` spec；重复内容以 project-session-actions 为准。
- **`webui/projects` 归档**：其描述的 `/agent` 页面、`/api/agent/projects` 接口已被 `webui/spec.md` 的 SPA 工作台取代；`agent_kind` 渲染与会话作用域 composer 等仍有效的语义在 webui/spec.md 中已有对应需求（会话详情 header / follow-up composer），无需保留独立 capability。
- **移除树中唯一嵌套目录**：`webui/projects` 归档后 `openspec/specs/` 全平铺。

## Capabilities

### New Capabilities
- `workbench`: 项目导向 agent 工作台行为面（项目注册、会话归属、0-turn 占位会话、归档/History/Inbox、目录选择器、并发与执行面）。

### Modified Capabilities
<!-- 无既有 requirement 级行为变更——本 change 只做目录收敛与文本去重，不新增/改写任何行为语义。 -->

## Impact

- `openspec/specs/` 目录树：新增 `workbench`，归档 `agent-workbench` 与 `project-session-actions`，删除嵌套 `webui/projects`（无新行为）。
- 归档目录：`changes/archive/` 增加三个 2026-09-09 归档 change（agent-workbench、project-session-actions、webui-projects），其 proposal/design 保留供历史追溯。
- 引用同步：`testsuite-acceptance/spec.md` 提及 `agent-workbench`、`project-session-actions`，改指 `workbench`；无源码影响。
- **Non-goals**：不改任何 Rust/TS 源码；不改 webui/spec.md 现有 requirement 语义；不在此 change 处理命名族整体对齐（workbench 之外的前缀对齐另走批次 E）。
