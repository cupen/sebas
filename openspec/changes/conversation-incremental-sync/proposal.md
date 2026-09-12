## Why

工作台对话流的前端刷新是全量重拉：每收到一个 WS 事件（流式期间每个
chunk 都会触发）就 `refetch` → `GET /api/sessions/{key}` → 整段对话
entries 重新传输。长会话下这是高频大 payload 浪费。后端其实已具备增量
读取协议（核心通道 `Turns { from }`、position 无缝只追加），缺的只是
webui HTTP 层透传与前端游标管理。

## What Changes

- **HTTP 增量参数**：`GET /api/sessions/{key}` 增可选 query
  `entries_after=<position>`——响应结构不变，`entries` 只含
  `position > entries_after` 的条目；缺省/无参数 = 全量（现行为）。
  后端透传既有 `turns(key, from)`。
- **前端内存游标**：per-session 记录已渲染的最大 position；首次聚焦
  （或 F5 后）全量拉，后续 refetch 带 `entries_after=游标` 并把新条目
  append 进本地序列；游标仅在成功 merge 后推进；会话切换保留各自游标。
- **契约化**：position 无缝（gapless）与只追加（append-only）写成
  spec 场景——增量正确性的前提（core 侧已满足，无需新代码）。
- 持久化本身为现状（transcript 随会话状态落盘、重载恢复已有验收），
  本轮不动。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities
- `webui`: 「Session payload carries the conversation」增 `entries_after`
  参数语义与 position 无缝/只追加契约。
- `agent-workbench`: 新增「对话增量同步」需求（前端游标策略：
  首拉全量、后续增量 append、内存游标生命周期）。

## Impact

- `sebas-webui/src/api.rs`（session_detail 解析 query 并透传
  `turns(key, after)`）、前端 `dashboard.ts` / `client.ts`（游标与
  merge）、`session_endpoints_test.rs`。
- 不动 core/dispatch（`turns` 协议与 position 分配已满足）。

## Non-goals

- 不做 WS 直接推送条目（保持 pull 模式；push 是后续独立优化）。
- 不做 localStorage 跨刷新游标（F5 后视为首次全量）。
- 不改 position 基数（维持 0-based；「从 1 开始」按示意理解）。
- 不动 pending/review-card 的全量随行（它们不在增量范围内）。
