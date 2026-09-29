## ADDED Requirements

### Requirement: Unseen-turn seam presence follows arrival context

未读分界线（unseen-turn seam）SHALL 只随到达上下文出现：对会话非聚焦（或页面隐藏）期间到达的可见回复，操作员下次进入该会话时 SHALL 恰好绘出一次分界线；对聚焦且贴底观看期间流式到达的内容 SHALL NOT 绘出；分界线随已读即清——边界内容一旦被看过，后续进入同一会话不得再对已看内容重现分界线。

#### Scenario: refocus after unfocused completion shows the seam

- **WHEN** 会话在非聚焦状态下完成一段可见回复，操作员随后聚焦该会话
- **THEN** 侧栏行先出现未读徽标，转写在未读内容之上绘出分界线

#### Scenario: focused completion draws no seam

- **WHEN** 操作员正聚焦会话并停留在直播边缘时回复完成
- **THEN** 该段内容不绘出分界线，也不误标未读

#### Scenario: seam clears after being read

- **WHEN** 操作员已看过分界线以下内容后离开再回到该会话
- **THEN** 已看内容之上不再重现分界线

### Requirement: Focus anchors at the server's current count

聚焦会话推进读锚 SHALL 以服务端当前段计数为准：即使聚焦动作与回合定稿竞速，锚推进路径不得停在回合前的旧值（establishment 对已有锚只增不减的规则不变，但「已有锚 + 新段落」的推进不得依赖竞速窗口内的陈旧本地状态）。

#### Scenario: immediate refocus after a completed turn parks the full count

- **WHEN** 会话完成第二回合（服务端 msg_count=2）且操作员随即聚焦该会话
- **THEN** 存储读锚推进到 2（不是回合前的 1），徽标不再复燃
