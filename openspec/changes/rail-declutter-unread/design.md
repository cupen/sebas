# rail-declutter-unread — Design

## Context

rail（`sebas-project-rail.ts`）当前渲染：Projects 组（行内 `+`/`×`，分支名，会话数）、Waiting on you 组、Inbox 组（`project_id = null` 的会话，空则隐藏）、History 组（归档会话，插入序）。会话行有归档与关闭两个 hover 按钮。

已存在且被本设计依赖的机制：

- transcript 条目 `TurnEntry`（`sebas-dispatch/src/engine/events.rs`）：`kind ∈ {prompt, content}`，`element_type ∈ {markdown, thinking, tool, error}`，`position` 单调递增。
- transcript 视图的 seen-boundary seam（agent-workbench「Unseen-turn seam」）：按会话存 seen 时间戳于 localStorage，近底部滚动标记已读，`unseenCount` 已有状态。
- WS 事件：`session.created/updated/removed/pending_dropped` 等，`session.updated` 在十余个生命周期节点广播，但**不**保证每条 transcript 条目落盘即发。
- rail 已有 10s 节点轮询（`refresh()`），顺带重取会话列表。
- Web Awesome 3.12 提供 `wa-dropdown` / `wa-dropdown-item`。
- 归档语义（`api.rs archive_session`）：先 close（kill 子进程）再写 archive.json，消息网关拒绝已归档会话。

## Goals / Non-Goals

Goals：rail 元素收敛后的信息架构成立；未读信号精确、可测试；既有能力（关闭、可达性告警）不丢。

Non-Goals：跨设备未读同步；无项目会话的新展示位；项目 `...` 菜单新功能；服务端已读游标。

## Decisions

**D1 未读计数挂点：dispatch 引擎累计，投影暴露。** 在两条执行路径（native `agent_backend.rs`、ACP `session_backend.rs`）的 transcript flush 处累计「可见回复段」数，计入 `SessionInfo` 投影（新字段 `msg_count: u64`），随 `session.updated` 广播，rail 10s 轮询兜底。
备选：逐消息新 WS 事件（事件面膨胀，且 `session.updated` 已覆盖绝大多数时机）；纯前端近似（数字不诚实，已否决）。

**D2 消息口径：`kind=content` 且 `element_type ∈ {markdown, error}`。** 用户拍板「可见回复段」。注意与 seam 的差异：seam 按**轮**分界，徽标按**段**计数——两者共用同一游标（D3）但聚合粒度不同，属有意为之，测试需分别钉住。
备选：按轮计（后端需识别轮边界，`TurnEntry` 无轮 id，实现最重，已否决）。

**D3 游标：localStorage 共享模块，与 seam 同锚。** 新建前端模块（如 `unread-cursor.ts`）持久化每会话 `{ seen_ts, anchor_count }`；transcript-view 的 seen 存储迁移到该模块（行为不变），project-rail 徽标 = `msg_count − anchor_count`。无锚点 = 全部已读（避免首次打开/清缓存后历史会话全冒红点）。聚焦会话（switch 成功）时写锚。多标签页同源共享，`storage` 事件可顺带同步（不作为验收项）。
备选：服务端游标（跨设备一致，但 localhost 单操作员场景不需要，+API +seam 迁移面大，已否决）。

**D4 `...` 菜单：wa-dropdown，hover 显现沿用现有 CSS 契约。** 项目行 `row-actions` 改为 `...` + `+` 两个 `row-action`（顺序固定），会话行仅一个 `...`。现有 `.row-action { opacity: 0 }` + `.row:hover/:focus-within` 规则不变。菜单项：项目 = 移除；会话 = 归档、关闭（danger 样式，active 会话确认弹窗复用现有 `closeTarget` 流程）。无障碍：`aria-haspopup`、菜单项可键盘到达；焦点在行内（focus-within）时按钮不隐藏。
备选：自实现 popup（无理由，wa-dropdown 现成）。

**D5 移除项目阻止：后端强制 + 弹窗预检。** `POST /api/projects/{id}/remove` 在存在非归档会话时返回 typed rejection（带计数文案所需的会话数）；rail 弹窗在打开时预检本地列表并就地说明。废除「迁移 Inbox」文案。
遗留无项目会话（含 Feishu 来源）：不迁移、不清理——仅不再被 rail 渲染，API 仍可达（提案 Non-goals 已声明）。

**D6 composer 创建强制选项目。** `workbench-composer.ts` 的绑定语义从 `null = inbox` 改为「无选择 = 不可提交」（禁用态说明文案）；不再渲染「→ inbox」。默认预选当前项目维持现状。过渡态：composer 创建模式的终态由 `workbench-interaction-polish` 收口（纯跟随、无创建模式）；本变更先行落地，止血点在于 Inbox 分组移除后 composer 不得再产出 rail 无展示位的无项目会话。

**D7 History 倒序：前端排序。** `/api/archive` 保持插入序返回，rail 渲染前按 `archived_at` 降序排。服务端不动（语义未变，改它反而动 wire 契约）。

**D8 分支：隐藏显示、保留探测。** 去掉项目行 `<span class="branch">`；`loadBranch`/`accessible` 链路保留（删除线告警依赖）。工作台项目头部的分支显示（agent-workbench「Project view states real working-copy context」）不在本次范围。

**D9 恢复归档会话遇项目缺失：前端预检。** restore 前比对 `archived_sessions[].project_path` 与已注册项目；路径未注册则就地报错「请先重新添加项目」，不发请求。后端 restore 不改（不变量「活跃会话必有归属项目」由前端守门；后端强行兜底属新需求）。

**D10 会话名：消费既有 `prompt_preview`，前端截断。** 后端已把首条用户消息投影为 `SessionRow.prompt_preview`（`routes.rs:55`），前端从未消费——rail 行改 `label = prompt_preview ?? session_id_short ?? chat_id`。截断在前端做：上限 40 码点 + `…`，CSS ellipsis 兜底，`title` 挂全文；不做服务端截断（archive label 已复用全文，改投影语义反而破坏它）。零轮占位会话首条消息发出后 `session.updated` 触发刷新，名字自然从 ID 变为预览。rail 内的关闭/归档确认弹窗复用同一 label helper。工作台 `.fkey` 与 IA-v1 sessions 页保持 ID 展示（Non-goals）。

## Risks / Trade-offs

- [seam 与徽标聚合粒度不同（轮 vs 段）被误认为 bug] → design 与两处 spec 场景都写明口径；单测分别断言。
- [`session.updated` 不逐条目触发，徽标可能滞后] → 10s 轮询兜底是验收内行为；若验收发现不可接受，再评估在 transcript flush 处补发事件（任务外延，Open Questions 记录）。
- [hover 隐藏的 `...` 与 dropdown 面板交互丢 hover 态] → 面板打开期间行保持 `:focus-within` 显现；`wa-dropdown` 面板挂在 body 级时用 `@wa-open/​wa-close` 维持行的显隐类。
- [触屏设备无 hover] → 现状限制，本次不扩大（focus-within 已部分覆盖）。
- [localStorage 清空后已读丢失] → 有意为之（无锚点 = 已读），spec 已钉。

## Migration Plan

无数据迁移、无配置变更、无 wire 破坏性变更（`msg_count` 为新增字段，旧客户端容忍未知字段）。回滚 = 还原代码。`session-unread-badge` 为新增 capability，归档时并入 specs 树。

## Open Questions

- 徽标数字是否需要 `99+` 封顶（纯实现细节，任务内自决）。
- 若验收中 10s 轮询的徽标延迟体验不佳，是否在 transcript flush 处补发轻量事件——留待沙箱联调判定，不影响 spec。
