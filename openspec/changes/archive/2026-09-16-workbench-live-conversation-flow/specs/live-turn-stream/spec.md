## Purpose

定义回合内容从 core 经 webui 到浏览器的实时流式契约：哪些增量、如何合并、按什么顺序与容错规则到达浏览器，以及流式到达与已读锚（session-unread-badge）如何保持一致。

## ADDED Requirements

### Requirement: Turn content streams to the browser

core SHALL 把本机会话执行体产生的回合事件（文本增量、思考增量、工具开始/进度/结束）经核心通道转发给 webui；webui SHALL 把它们作为新 WebSocket 事件广播给所有已连接客户端。文本与思考增量 SHALL 在传输侧按时间窗（约 250ms，含大小上限的立即冲刷）合并后发送——传输合并 SHALL NOT 改变落库的回合条目内容（日志仍是唯一事实）。事件 SHALL 携带会话标识与回合内定位信息（回合位置、条目位置、事件序号），浏览器据此把内容追加进正确的对话块。前端 SHALL 容忍乱序、重复与迟到的流事件：以快照重取为收敛基准，流事件只是增量补充。回合进行中，正文增量 SHALL 在合并窗口内可见，而不是等回合结束。

#### Scenario: text streams during a turn

- **WHEN** an agent turn is producing output in a focused session
- **THEN** the browser appends the streamed text into the turn's block within the coalescing window, without waiting for the turn to finish

#### Scenario: tool cards appear live

- **WHEN** the agent starts and finishes a tool call during a turn
- **THEN** the tool card appears in the conversation at start and updates to its finished state at end, while the turn is still running

#### Scenario: coalescing does not corrupt the log

- **WHEN** several text deltas are merged into one streamed frame
- **THEN** the persisted turn entries still contain the same content as if no coalescing happened

#### Scenario: late or duplicate events are tolerated

- **WHEN** the browser receives a stream event that is out of order, already applied, or older than a refetched snapshot
- **THEN** the conversation view converges to the snapshot state and does not duplicate content

#### Scenario: unknown stream events are ignored

- **WHEN** a client receives a stream event type it does not know
- **THEN** it ignores it without breaking the connection, as with any other unknown WebSocket event

### Requirement: Streaming respects the read anchor

当操作者聚焦某会话且滚动到最新回合时，流式到达的可见内容 SHALL 随渲染推进该浏览器的读锚——角标不闪烁、已读缝不出现。当操作者未聚焦或未贴底时，流式到达的可见内容按 session-unread-badge 的既有规则计为未读。思考与工具类流事件 SHALL NOT 计入未读（与会话未读计数的既有一致）。

#### Scenario: streaming while focused and at the bottom

- **WHEN** the operator is focused on a session and scrolled to the newest turn while output streams in
- **THEN** the rail shows no unread badge for that session and no seam is drawn inside the streaming turn

#### Scenario: streaming while scrolled up

- **WHEN** output streams into a session the operator is not viewing, or has scrolled away from the bottom
- **THEN** the session's unread count grows per the existing badge rules
