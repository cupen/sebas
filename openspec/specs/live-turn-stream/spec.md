# live-turn-stream Specification

## Purpose
定义回合内容从 core 经 webui 到浏览器的实时流式契约：哪些增量、如何合并、按什么顺序与容错规则到达浏览器，以及流式到达与已读锚（session-unread-badge）如何保持一致。

## Requirements

### Requirement: Turn content streams to the browser

core SHALL 把本机会话执行体产生的回合事件（文本增量、思考增量、工具开始/进度/结束）经核心通道转发给 webui；webui SHALL 把它们作为新 WebSocket 事件广播给所有已连接客户端。文本与思考增量 SHALL 在传输侧按时间窗（约 250ms，含大小上限的立即冲刷）合并后发送——传输合并 SHALL NOT 改变落库的回合条目内容（日志仍是唯一事实）。事件 SHALL 携带会话标识与回合内定位信息（回合位置、条目位置、事件序号），浏览器据此把内容追加进正确的对话块。前端 SHALL 容忍乱序、重复与迟到的流事件：以快照重取为收敛基准，流事件只是增量补充。回合进行中，正文增量 SHALL 在合并窗口内可见，而不是等回合结束。

执行体 SHALL 在生成过程中产生增量事件：支持 partial 流的 agent 驱动（如 claude）SHALL 启用 token 级部分消息并映射为文本增量，不得只在完整消息/块边界发事件；native 执行体在任何呈现面（webui、IM 桥）SHALL 逐增量转发文本事件，不得攒积到回合结束或工具边界才整块上屏。webui 面与 IM 面对同一执行体的流式粒度 SHALL 一致。

慢消费者丢帧 SHALL 可收敛：任一推送层（webui WS、核心通道）检测到订阅落后（broadcast Lagged）时，SHALL 向该客户端发出重新同步信号并继续连接；客户端收到重新同步信号后 SHALL 重取受影响会话的快照，不得静默丢弃增量后等待下一次无关刷新。

#### Scenario: text streams during a turn

- **WHEN** an agent turn is producing output in a focused session
- **THEN** the browser appends the streamed text into the turn's block within the coalescing window, without waiting for the turn to finish

#### Scenario: claude streams at token granularity

- **WHEN** a claude session produces a long text answer
- **THEN** partial message events arrive during generation (multiple frames within a second), not one block at message completion

#### Scenario: native session streams on webui like on IM

- **WHEN** a native-kind session produces text deltas
- **THEN** the webui receives the same per-delta stream the IM bridge receives, instead of one block at turn/tool boundaries

#### Scenario: tool cards appear live

- **WHEN** the agent starts and finishes a tool call during a turn
- **THEN** the tool card appears in the conversation at start and updates to its finished state at end, while the turn is still running

#### Scenario: coalescing does not corrupt the log

- **WHEN** several text deltas are merged into one streamed frame
- **THEN** the persisted turn entries still contain the same content as if no coalescing happened

#### Scenario: late or duplicate events are tolerated

- **WHEN** the browser receives a stream event that is out of order, already applied, or older than a refetched snapshot
- **THEN** the conversation view converges to the snapshot state and does not duplicate content

#### Scenario: lagged subscriber resyncs instead of silently dropping

- **WHEN** a slow client's broadcast subscription falls behind and frames are dropped
- **THEN** the client receives a resync signal and refetches the affected session snapshot, and no incremental content is permanently lost

#### Scenario: unknown stream events are ignored

- **WHEN** a client receives a stream event type it does not know
- **THEN** it ignores it without breaking the connection, as with any other unknown WebSocket event

### Requirement: Streaming respects the read anchor

当操作者聚焦某会话且滚动到最新回合时，流式到达的可见内容 SHALL 随渲染推进该浏览器的读锚——推进的是与 rail 徽标同一条「段计数」锚（会话的当前消息计数），角标不闪烁、已读缝不出现。当操作者未聚焦或未贴底时，流式到达的可见内容按 session-unread-badge 的既有规则计为未读。思考与工具类流事件 SHALL NOT 计入未读（与会话未读计数的既有一致）。流式推进与聚焦写锚、手动读到底部推进 SHALL 写同一个存储键的同一个字段，任何表面读到的已读水位一致。

#### Scenario: streaming while focused and at the bottom

- **WHEN** the operator is focused on a session and scrolled to the newest turn while output streams in
- **THEN** the rail shows no unread badge for that session and no seam is drawn inside the streaming turn

#### Scenario: streaming while scrolled up

- **WHEN** output streams into a session the operator is not viewing, or has scrolled away from the bottom
- **THEN** the session's unread count grows per the existing badge rules

#### Scenario: streaming advance equals focus advance

- **WHEN** the operator reads a streaming turn to the bottom and then switches away and back
- **THEN** the stored segment anchor equals the session's current message count, so no badge or seam reappears for the content just read
