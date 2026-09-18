## MODIFIED Requirements

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
