# Design — consolidate-workbench-session-actions

## Context

工作台行为面此前分裂在三处：`agent-workbench`（379 行，含项目/会话/并发/原生内核）、`project-session-actions`（121 行，五个操作 requirement）、嵌套 `webui/projects`（175 行，HTMX 时代旧接口 `GET /agent`、`POST /api/agent/projects`）。其中五个操作 requirement 在两份主 spec 间几乎逐字重复。归档目录显示 `webui/projects`（add-webui-agent-projects）早于新 SPA 工作台（webui 重构），其接口面已被 `webui/spec.md` 的 `GET /`、`/api/projects` 取代。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：把三处重叠收敛到单一 `workbench` capability；保持全部既有行为 requirement 语义（不做行为变更，只做文本归位与去重）；消除 `openspec/specs/` 树中唯一嵌套目录。
- **非目标**：不改 webui/spec.md 的既有 requirement；不新增/删除任何行为语义；不在此 change 处理命名族对齐（`workbench` 之外的改名走后续批次）；不改源码。

## Decisions

### 决策 1：以 project-session-actions 文本为操作面的语义基座

深读对比显示 project-session-actions 是较新/较全版本：目录树用 `GET /api/fs/browse-dirs`（lazy-load、root 限定、Windows 路径往返 400 修复）、Archive expiry 明确「no operator-facing notification」、新增「empty session created via API」场景。agent-workbench 中对应 requirement 是更早的简版（`GET /api/fs/browse`、缺 API 场景）。因此目录选择器 / 0-turn 会话 / 归档 / History-Inbox / 保留期五条 requirement 以 project-session-actions 文本为准；agent-workbench 独有的非操作 requirement（组织单元、注册表归属、会话归属、并发、未读缝、composer 保证、origin 可见、原生内核执行/门控审批、可用性、模型选择）全部迁入。

- **备选**：以 agent-workbench 为基座、反向并入 actions 的差异——会丢失 actions 的细化场景，且目录名最终仍是 workbench，徒增一次往返，弃。

### 决策 2：agent-workbench 与 project-session-actions 均走「归档 + 历史追溯」

两目录不直接删除：各自打包为归档 change（内容即主 spec 现状 + 说明性 proposal/design），保留在 `changes/archive/YYYY-MM-DD-*` 供追溯。新 `workbench` capability 由本 change 的 specs delta 在归档时创建。

- **备选**：git mv 目录——丢失 openspec 归档惯例（archive 目录是 change 记录的既定归宿），且 git 历史本身已保留文件内容，无需额外软链。

### 决策 3：webui/projects 直接归档，不展平保留

其五条 requirement（agent 页、创建项目会话、会话消息、详情/timeline、会话模型含 project_dir/agent_kind）中，页面与端点语义已被 `webui/spec.md` 新 SPA 覆盖；agent_kind 字段语义在 webui/spec.md 会话详情/follow-up composer 已有对应需求；Composer 会话作用域语义已并入 agent-driver 引用。无独有行为需要保留。

- **备选**：展平为 `webui-projects` 保留待下轮归并——但其语义确已被 webui 覆盖，保留会继续制造「哪份规范有效」的歧义，弃。
