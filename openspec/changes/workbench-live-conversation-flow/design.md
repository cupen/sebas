# Design — workbench-live-conversation-flow

## Context

增量数据现状：ACP 子进程的事件（`AcpEvent::TextDelta/ThinkingDelta/ToolStart/ToolProgress/ToolEnd`，见 `sebas-acp/src/session.rs`）已经实时流经 core，但两条下游链路都把内容丢掉了：core→webui 的 `SessionEvent`（`sebas-dispatch/src/engine/events.rs`）只有 Created/Updated/Removed/PendingDropped/Resync；webui→浏览器的 `WebUiEvent`（`sebas-webui/src/events.rs`）只有 session.created/updated/removed/pending_dropped/config.updated/permission.requested。前端 `transcript-view.ts` 只按快照渲染。spawn 侧：0-turn 占位会话等首条消息才拉起（`agent_backend.rs` 在 spawn 时才解析 `available_models`），resume 基建已存在（acp-session-mapping：routing id ↔ ACP id 持久映射 + `session/load`）。UI 侧：会话头在 `dashboard.ts` 渲染 All sessions/Archive/Close/mode；提交按钮五态机已在 `workbench-composer.ts`；rail 宽度边界 180–480px 默认 220px（`split-persist.ts`）；分隔条 `--divider-width: 12px`。

## Goals / Non-Goals

**Goals:** 回合内容实时上屏（文本/工具/思考）；聚焦即拉起带 resume；底沿与头部职责重组；版面对齐与 rail 宽度；流式与已读锚一致。

**Non-goals:** 见 proposal（不做 slash 面板、不改锚存储、不动 rail 信息架构、不退役 close 端点、不做按会话订阅）。

## Decisions

**D1 · 传输形态：core 通道新增回合事件帧，webui 原样转播为新 `WebUiEvent` 变体。**
core 侧订阅本会话执行体的 ACP 事件，经核心通道以新帧类型下发；webui 的 `SessionBackend` 增加回合事件订阅面（进程内后端直接转发，core_channel 客户端消费新帧），转播为 `turn.delta`（合并后的文本/思考增量）与 `turn.tool`（start/progress/end，kind 区分）两类 dotted-type 事件。不新开按会话订阅通道——维持「所有客户端收全部事件、前端按聚焦过滤」的既有 WS 姿态（proposal Non-goal），老前端对未知类型本就容忍（`ws.ts` 白名单外忽略），双向滚动升级都不碎。
备选：复用 `session.updated` 捎带 payload——弃，语义污染且无法表达增量；SSE——弃，WS 已是既定通道。

**D2 · 合并发生在 core 侧：约 250ms 窗口 + 大小上限立即冲刷。**
镜像 node-link 的「coalesced in transport, exact in log」原则：合并只影响传输帧，落库的 `TurnEntry` 内容不变。帧内带 `(session, turn position, entry position, monotonic seq)` 定位四元组，前端据 seq 丢弃重复、据快照 refetch 收敛。选 core 侧而非 webui 侧合并：少一跳转发放大，且远端节点会话未来可直接复用同一帧契约。
备选：逐 delta 直发——弃，高频小帧浪费；webui 侧合并——弃，多一跳且时序更乱。

**D3 · 聚焦即拉起：webui 在聚焦时触发既有 spawn 路径（无 prompt），resume 沿 acp-session-mapping。**
触发点放在 webui（聚焦 = switch 端点 / rail 点击 / 深链）而非 core 自动化：只有 webui 知道「操作者正在看」，避免 core 为不可见会话白起进程。失败非致命：占位保留、错误就地如实呈现。模型芯片三态由此而来——启动中（spawn 窗口）→ agent 上报的模型 → spawn 完成且为空的诚实「无可用模型」。
备选：core 侧会话创建即拉起——弃，批量建会话会白起；首条消息才拉起维持现状——弃，正是本次要修的「无可用模型」根因。

**D4 · 底沿构成：左 mode 下拉、右模型芯片 + 提交按钮；提交按钮图标即状态。**
五态机逻辑不变（sending spinner > stop 方块 > queued 时钟 > send 箭头 > disabled），只删文字标签、留 aria-label/title。agent 锁标签（🔒 agent 名）迁到会话头作纯展示。mode 下拉自会话头迁入底沿左端，远端会话的 effective/desired 呈现语义不变。

**D5 · 会话头去交互化 + 归档唯一出口。**
头部删除 All sessions / Archive / Close / mode 切换。归档入口放 rail 会话行悬停溢出菜单（项目行已有 `...` 菜单模式，同构复用）；确认弹窗合并 close 语义的「将丢弃 N 条待执行」警告。`POST /api/sessions/{key}/close` 端点保留（服务端行为不变），仅 UI 停用——退役另立变更。

**D6 · 版面常量：分隔条 12→6px；对话区与输入框共用同一套水平内边距 token；rail 默认 280px（≥1440px 视口）、上限 520px。**
`split-persist.ts` 的 localStorage 键不变——已存的旧宽度值继续生效（只有默认值与新 clamp 上限变化），不迁移用户数据。12 个中文字的保证用「rail 默认宽 − 行内边距 − 徽标/按钮预留 ≥ 12 × 标准标题字号」在样式层实现，测试断言 280px 下无截断。

**D7 · 流式与已读锚：复用 transcript-view 既有的贴底防抖推进路径。**
流式追加到「聚焦 + sticky 贴底」的视图时调用既有 `scheduleMarkSeen`；未贴底/未聚焦不写锚，自然计未读。msg_count 只数 markdown/error 段（session-unread-badge 既有定义），工具/思考流天然不闪角标——无需新逻辑，只需测试钉住行为。

## Risks / Trade-offs

- [流事件与回合结束快照竞态，内容重复或丢失] → seq 去重 + 快照收敛为基准；流事件只是增量补充，重取永远赢。
- [高频 delta 广播压垮 WS] → core 侧 250ms 窗口 + 大小上限冲刷；每会话独立窗口。
- [聚焦即拉起放大子进程数（快速切换多个占位会话）] → 仅聚焦会话触发；同一会话幂等（已有活子进程不重复 spawn）；core 并发上限既有语义兜底。
- [resume 失败被误读为会话丢失] → `resumed=false` 时如实提示「已开启新对话」，占位与历史不删。
- [rail 旧存宽度 <280px 的用户看不到新默认] → 刻意保留用户偏好；提示文案无需，拖拽即可加宽。
- [长回合增量渲染的性能] → 逐块追加 DOM（与快照渲染同构），不做整段重排；必要时按现有 turn 分块复用。

## Migration Plan

1. 后端先行或前端先行皆可：新 WS 事件类型双向兼容（老端忽略、新端等不到就维持快照行为）。
2. 聚焦即拉起上线后，重启后的旧会话在首次聚焦时自动 resume；无需数据迁移。
3. 会话头动作移除是纯 UI 收敛，rail 菜单入口同 PR 可用，无过渡期双入口。
4. 回滚：回退二进制即可；无 schema、无持久化格式变化。

## Open Questions

（无——八个决策点已在 grilling 会话中与操作者逐一对齐。）
