# workbench-natural-conversation-flow — Design

## Context

`transcript-view.ts` 现把 agent turn 分组为「文本段按到达序 + 全部 thinking/tool 收进单个 ProcessBlock（钉在回合首个过程条目位置）」，整体套 `.turn-block .bubble` 卡片；二级折叠、结构化标题、错误合并、已读缝均已就位。流式增量经 WS → 增量 refetch（position 游标）到达，分组每次全量重算。见 proposal.md — Why。

## Goals / Non-Goals

**Goals:**

- 分组层把「单 ProcessBlock」改为「交替 text-run/process-run 序列」，渲染层去卡片。
- 折叠行摘要在流式期间实时刷新；展开状态跨重渲染保持。

**Non-Goals:**

- 不动通道/WS 词汇、增量同步游标、已读缝、二级折叠标题规则、错误合并。

## Decisions

- **D1 分组：turn 内按到达序切交替 run**：连续同类（text 或 process）条目合并为 run，turn 即 `TextRun | ProcessRun` 交替序列；process-run 渲染为一个折叠。替换现有「过程条目全部 collect 进单块」的归并逻辑，文本合并语义不变。被否：后端改持久化结构（wire/历史数据迁移成本高，前端分组本就是既定架构——core 只存 chunk 序列）。
- **D2 折叠 key 与展开状态**：ProcessRun 的稳定 id 取 run 首条目的 `position`；`details` 折叠的 open 状态按 id 记录在视图本地 Map，重渲染（流式 refetch 触发全量重分组）后按 id 恢复。被否：用 Lit repeat key 之外的数组下标（重分组后下标漂移，展开状态会跳）。
- **D3 摘要实时刷新**：折叠 summary 行渲染「进行中工具 title（结构化 title，缺省通用标签）+ 条目计数」，数据源即现有增量 refetch 后的 run 内容——无需新事件；折叠保持收起由「open 状态 Map 未命中即收起」自然保证。被否：自动展开进行中的折叠（收展跳动，拷问轮已否）。
- **D4 去卡片样式**：agent 侧删 `.turn-block .bubble` 卡片壳（底色/边框/阴影/圆角），作者标签降为裸排小字行；`.is-user` 收敛为轻底色块（保留 tinted 背景，去边框阴影）；外层 stage 浮岛（「Workbench regions read as floating islands」）不动。被否：双方全裸排（己方发言辨识度靠排版撑太弱，拷问轮已否）。
- **D5 错误气泡与既有合并**：error 条目不参与 text/process run 切分，沿用相邻同文合并计数、独立气泡形态。

## Risks / Trade-offs

- [流式中 run 边界频繁重算，折叠展开状态可能闪动] → D2 的 id 锚定 + open Map 恢复；浏览器测试覆盖「展开后增量到达不收起」。
- [去卡片后 agent 与用户消息视觉区分减弱] → 用户消息保留 tinted 底 + 排版对齐差异；sandbox 截图验收。
- [已读缝按 turn 计数不受 run 切分影响，但回归风险仍在] → 未读/贴底单测与 testsuite 回归照跑。

## Open Questions

（无）
