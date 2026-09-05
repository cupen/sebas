# 设计：对话区 composer 会话化 + agent 只读标签

## Context

后端事实（见 proposal.md）：`POST /api/sessions` 的 `backend` hint 在 spawn 时写进
router mapping 的 `pending_kind`（`sebas-router/src/state.rs`），伴随会话生命周期
保留（spawn 只是读取，不清除）；模型中程可换（`session/set_config_option`）。
前端 `workbench-composer.ts` 却把 agent 下拉放在对话区底部、聚焦会话 transcript
正下方，且 `submit()` 恒为 `createSession()`；同一个 `backend` 状态还绑定了两个
下拉（顶部 + 底部，leftover）。`SessionInfo`（`sebas-router/src/router/events.rs`）
不携带 kind，`SessionRow`/`SessionDetail` 因此无从展示。

## Goals / Non-Goals

Goals：composer 语义与后端一致（跟随=发消息，创建=选 agent）；agent 名以小号
只读文本常驻输入框底部；会话数据带出 agent kind。

Non-Goals：见 proposal.md（不换 agent、不动侧栏 IA、不做权限模式菜单）。

## Decisions

**D1 — 双模式收敛在 composer 内部，模式由"是否有聚焦会话 + 显式切换"决定。**
跟随模式的发送走现有 `POST /api/sessions/{key}/message`（聚焦只是 display
pointer，不碰路由——webui spec 的 focus 语义不变）。创建模式沿用
`createSession`。备选"composer 永远只创建、跟送去详情页"被否：与原型
"Ask for follow-up changes" 的意图和用户心智模型冲突。
"新会话"入口做成 composer 底部一个小 chips（文本按钮），聚焦时可见；避免
"想给同项目再开一个会话"没有入口，也不新增侧栏 IA。

**D2 — agent kind 走 `SessionInfo.agent_kind = mapping.pending_kind`，None 语义
保留（= 默认 kind）。** 不在后端把 None 解析成具体 slug：默认 kind 是配置态，
解析留给前端（composer 已加载 `/api/agent-kinds`，None → 显示"acp（默认）"）。
备选"后端解析成显示名"被否：router 不该依赖 agent-kinds 配置的展示层细节。

**D3 — 前端显示名解析复用 `/api/agent-kinds`。** slug → `AgentKindInfo.name`
映射；未知/不可达 slug 原样显示（slug 本身就是可读的）。`providerLabel`
（provider 名）保留在创建模式；跟随模式下 provider 对已绑会话意义不大，
由 agent 名 + 模型名取代。

**D4 — 持久化补 `pending_kind`。** `MappingDto`（state.json 旧格式）加
`#[serde(default)] pending_kind`；若 state-store sessions 表已接管 mapping
落盘，则同步加列。没有这一步，重启后的 dormant 会话标签回退到默认 kind，
与事实不符。schema 变更全部 `#[serde(default)]`/nullable，旧文件可读。

**D5 — composer 布局：删顶部下拉，两模式共用底部工具栏槽位。**
跟随模式：`<agent 名（只读小字）> [模型 ▼]`；创建模式：
`[agent ▼] [模型 ▼] → 项目/inbox`。0.78rem mono、dim 色与现有 `.label`
一致，满足"不要太大"。模型数据源：跟随模式用聚焦会话自己的
`available_models`（dashboard 已持有 focusedDetail，作 property 传入）；
创建模式保留"借最近会话列表"的现有数据源。

## Risks / Trade-offs

- [聚焦会话 detail 尚在途时 agent 名/模型列表为空] → 底部显示占位（`· · ·`），
  detail 到达后随现有 refetch 刷新；不阻塞输入。
- [`pending_kind` 在老会话记录中缺失] → None → "acp（默认）"，不报错（spec
  已覆盖该场景）。
- [创建模式下模型列表仍是"借来的"] → 已知妥协，维持现状；聚焦模式下数据源
  是真实的。未来若做创建对话框再收编。
- [双模式引入状态机复杂度] → 模式只有两个来源（聚焦状态 + 显式切换），在
  `workbench-composer` 内部以单个 `@state` 表示；不外溢到 app-shell。

## Migration Plan

纯增量：wire 新字段对旧前端不可见；前端先合也只在无 `agent_kind` 数据时显示
回退标签。无数据迁移（MappingDto serde default 即兼容）。回滚 = revert，
无残留状态。

## Open Questions

（无）
