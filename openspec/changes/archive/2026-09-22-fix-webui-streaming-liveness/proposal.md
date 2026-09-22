# Fix WebUI Streaming Liveness

## Why

Agent 对话在 WebUI 上不流式、流式期间渲染卡顿。四路调查（产出侧/传输层/前端/浏览器实测）证实：WS 传输本身通畅，但 (1) 产出侧几乎没有产生增量——claude 驱动双层丢弃 partial 流、native 后端在 webui 面吞掉 TextDelta 只在回合边界整块落盘；(2) 前端把每个 WS 事件放大成 5+ 个 HTTP 全量 refetch，聚焦会话 transcript 内嵌于 `/api/summary`（实测曾达 2.8MB），把浏览器 renderer 压到 100%+ CPU；(3) 渲染层自动滚动被未读缝卡死、seam 翻转触发整块 DOM 重建、每帧全量重解析 markdown。三者叠加，用户观感即"不流式 + 非常卡"。

## What Changes

- claude 驱动开启 `include_partial_messages` 并映射 `Message::StreamEvent`（双层缺口同时补），产出 token 级 TextDelta。
- native 后端在 webui 面实时转发 TextDelta（接入 `subscribe_turn_events`），不再攒到回合边界。
- `/api/summary` 不再内嵌聚焦会话全量 transcript；前端 refetch 按事件类型分流并节流，消灭"每事件 5+ 请求 × MB 级响应"放大回路。
- 前端 transcript 渲染修复：自动滚动 sticky 判定、seam 模板统一避免整洞重建、markdown 增量渲染、纯 `turn.append` 触发滚动。
- 过程折叠的交互与长内容出口：折叠张开的 affordance 收敛为轻量行内 link（不再是大按钮块/卡框）；二级条目内容超阈值时截断显示并明示省略量，经「查看全部」在会话滚动容器之外的独立弹层查看完整内容（关闭即卸载 DOM）。
- 丢帧可收敛：WS Lagged 后强制 Resync 快照重取；会话重建/core 重启后前端 position 游标作废重取全量。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `live-turn-stream`: 新增"产出侧必须产生增量"（claude partials、native 面转发）、"慢消费者丢帧后必须可收敛（Resync 强制快照重取）"的需求。
- `webui`: dashboard/summary 语义变化——聚焦会话 transcript 不再内嵌于 summary 响应；refetch 分流节流；position 游标作废规则。
- `agent-workbench`: 过程折叠（process fold / 二级折叠）的 affordance 改为轻量行内 link；展开的长条目截断显示并新增「查看全部」隔离弹层。
- `agent-core`: 工具执行期进度事件不再是死代码（可选，随 native 增量转发一并落地）。

## Non-goals

- 不改远端节点（execution-node）链路的 150ms 合并窗口语义。
- 不改飞书卡片渲染路径。
- 不做 WS 协议（webui-ws-rpc 封套）变更。
- 不引入 gzip/压缩中间件（拆分 summary 后单响应回到 KB 级，暂无必要）。

## Impact

- 后端：`sebas-acp/src/claude/driver.rs`（partials + StreamEvent 映射）、`src/agent_backend.rs`（TextDelta 转发 + native 接入 turn_events）、`sebas-webui/src/api.rs`（summary 拆分、Resync 传播）。
- 前端：`sebas-webui/frontend/src/views/dashboard.ts`（refetch 分流节流、游标作废）、`transcript-view.ts`（滚动/seam/markdown 渲染）、`api/ws.ts`（新事件类型）。
- 风险：claude 驱动开启 partials 后 mapper 需处理新帧型，需回归 claude kind 全链路；summary 拆分是 API 响应形状变化，前端需同步升级。
