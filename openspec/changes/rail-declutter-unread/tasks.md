## 1. 后端：消息计数与投影

- [x] 1.1 dispatch 引擎新增每会话消息计数：native（`agent_backend.rs`）与 ACP（`session_backend.rs`）transcript flush 处累计 `kind=content` 且 `element_type ∈ {markdown, error}` 的条目数；`SessionInfo` 暴露 `msg_count`；`cargo test -p sebas-dispatch` 新增计数口径单测（回复 +1、thinking/tool/prompt 不计）并通过
- [x] 1.2 webui 投影与事件：`SessionRow` 增加 `msg_count` 字段（`routes.rs`/`api.rs`），`session.updated` 携带；`cargo test`（webui 侧投影测试）通过，`GET /api/sessions` 响应含该字段
- [x] 1.3 移除项目前置校验：`POST /api/projects/{id}/remove` 在存在非归档会话时返回 typed rejection（错误信息含会话数）；`cargo test` 覆盖「有会话拒绝 / 无会话放行」两例

## 2. 前端：未读游标与徽标

- [x] 2.1 新建共享未读游标模块（localStorage 存每会话 `{seen_ts, anchor_count}`，无锚点视为已读，聚焦写锚 API）；vitest 单测覆盖读写、首访、清零
- [x] 2.2 `transcript-view.ts` 的 seen 存储迁移到共享模块（seam 行为不变，`unseenCount` 语义不变）；现有 transcript-view 测试全部通过
- [x] 2.3 `project-rail.ts` 会话行渲染未读徽标（`msg_count − anchor_count`，0 或负数不显示，`99+` 封顶），聚焦会话（switch 成功）即清零；vitest 断言徽标出现/消失/清零

## 3. 前端：rail 行操作收敛

- [x] 3.1 项目行改为 `...` + `+`（`wa-dropdown` 实现 `...`，仅含「移除」项，hover/focus 显现沿用 `.row-action` 现有规则，顺序 `...` 在前）；vitest 断言按钮存在性、顺序与移除弹窗联动
- [x] 3.2 会话行收敛为单个 `...` 菜单（含「归档」与「关闭」，关闭 danger 样式且复用 active 确认弹窗流程，不再有行内直删按钮）；vitest 覆盖菜单触发归档/关闭两条路径
- [x] 3.3 移除项目弹窗预检非归档会话并就地展示「先归档/关闭（N 个会话）」文案，后端 typed rejection 内联呈现；vitest 覆盖阻止路径
- [x] 3.4 会话行命名改用 `prompt_preview`（回退 `session_id_short ?? chat_id`，前端截断 40 码点 + `…`，`title` 挂全文），关闭/归档确认弹窗复用同一 label；vitest 覆盖命名、截断与占位会话回退

## 4. 前端：分组与显示整理

- [x] 4.1 移除 Inbox 组（`inboxSessions`/`renderInbox` 及相关测试桩），无项目会话不再渲染；vitest 断言 Inbox 组不存在
- [x] 4.2 项目行去掉分支名 span（保留 `loadBranch` 与 `accessible` 删除线告警），History 组按 `archived_at` 降序渲染；vitest 断言分支不显示、History 顺序
- [x] 4.3 composer 创建模式强制显式选项目：移除 inbox 绑定（`null = inbox` 语义与「→ inbox」徽标），未选项目时禁用提交并说明；vitest 覆盖禁用与提交绑定

## 5. 联调验收

- [x] 5.1 `cargo build` 后以 `invoke testsuite-webui-sandbox` 起沙箱，人工核验验收旅程：hover 显隐与 `...` 菜单、阻止移除文案、归档/关闭入菜单、History 倒序、新回复徽标亮起与聚焦清零（fake-claude 桩造新回复）
- [x] 5.2 更新 `tests/testsuite-webui` 浏览器用例与 `tests/acceptance/COVERAGE.md` 覆盖面记录，`invoke testsuite-webui-server` 相关用例通过；`openspec validate rail-declutter-unread` 保持 valid
