# rail-declutter-unread

## Why

左侧项目栏信息密度失衡：常驻的 Inbox 分组、行内平铺的操作按钮与分支名占据视觉带宽，而真正需要被看见的信号（会话来了新消息）反而没有呈现。本次收敛栏内元素、补上未读徽标，让操作员一眼分清「哪里有事发生」。

## What Changes

- **BREAKING** 移除 Inbox 分组：无项目会话不再在 rail 展示（Feishu 来源会话改由 Feishu 侧与 sessions API 访问）。
- 移除项目语义变更：项目名下存在未归档会话时**阻止移除**，提示先归档/关闭（废除「迁移 Inbox 继续运行」的承诺）。
- 项目行操作收敛为 `...` + `+`（默认隐藏，hover/focus 显现）：移除动作移入 `...` 下拉（现阶段仅含移除项，留扩展位）；分支名不再显示（目录可达性探测保留，删除线告警不变）。
- 会话行操作收敛为单个 `...`（默认隐藏，hover/focus 显现）：菜单含归档与关闭（关闭为危险项，active 会话仍走确认弹窗）。
- 新增会话未读徽标：服务端按「可见回复段」口径计数，rail 会话行显示高亮数字，聚焦会话即清零。
- 会话名改用首条用户消息（`prompt_preview`，已投影未消费）：超长截断（40 字符 + 省略号），零轮占位会话回退会话标识，发出首条消息后名字随之更新。
- History 组按归档时间倒序（新的在前）。

## Capabilities

### New Capabilities

- `session-unread-badge`：服务端会话消息计数（口径定义、投影字段、事件广播）与 rail 未读徽标（游标存储、清零时机、与 transcript seen 分界线共用游标）。

### Modified Capabilities

- `project-session-actions`：Inbox 分组移除；移除项目遇存活会话改为阻止；History 倒序；项目/会话行操作收敛为 `...` 菜单；分支显示移除。
- `agent-workbench`：同步上述 rail 镜像需求；composer 创建止血为必须显式选择项目——Inbox 分组移除后 composer 不得再造 rail 无展示位的会话（过渡态；composer 终态「纯跟随、无创建模式」由 `workbench-interaction-polish` 收口，实施顺序本变更先行）。
- `webui`：dashboard 路由场景描述中 Inbox 分组的措辞更新。

## Impact

- 前端：`sebas-project-rail.ts`（分组、`...` 菜单、徽标）、`workbench-composer.ts`（inbox 绑定移除）、`transcript-view.ts`（seen 游标共用）。
- 后端：dispatch 引擎消息计数与投影（`SessionInfo` → `SessionRow` 新字段）；移除项目 API 前置校验；无新增 API（未读游标存浏览器 localStorage）。
- 测试：project-rail / composer / transcript 单测与 testsuite-webui 浏览器用例同步更新。

## Non-goals

- 不为无项目会话（Feishu 来源）另建 rail 展示位。
- 项目 `...` 菜单本期不新增功能（重命名、复制路径等），仅留结构扩展位。
- 不做跨设备未读同步（游标存 localStorage，换浏览器视为已读）。
- 「Waiting on you」组与 wait-badge（悬空审批数）保持现状。
- 工作台焦点头部的会话 key 展示与 IA-v1 sessions 页的 ID 展示保持现状（仅 rail 行与 rail 弹窗改用会话名）。
