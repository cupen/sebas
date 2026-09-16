# workbench-natural-conversation-flow

## Why

对话流现状是 feishu 卡片式：agent turn 整体包在一个大气泡卡片里，且该回合全部 thinking/工具条目收进位于「回合首个过程条目」位置的单个折叠——工具实际发生在哪两段正文之间无从得知，时间序被打断。卡片是 feishu 特有的承载方式，web 端不必照搬；流式管道（workbench-live-conversation-flow）已交付，呈现层是最后一块。

## What Changes

- **agent turn 去卡片化**：撤掉包裹整个回合的大气泡容器，作者标签 + 正文段 + 过程折叠按自然对话流裸排；用户消息保留轻底色块（去卡片边框/阴影感）；外层对话舞台浮岛分层不动。
- **过程折叠时间序化**：回合内按到达顺序切分——每段连续 thinking/工具连发 = 一个折叠，嵌在正文段之间的真实发生位置；折叠内保留逐条目二级折叠（结构化标题规则沿用）。
- **流式姿态**：过程折叠始终默认收起，折叠行摘要实时刷新（进行中的工具名、条目计数）；用户展开某折叠后，该折叠内新条目实时就地追加。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`: MODIFIED「Workbench renders the focused session as a conversation」——单气泡+单折叠改为自然对话流+时间序多折叠+流式摘要；MODIFIED「Operator submission receipt」——用户消息承载形态措辞随轻底色块调整。

## Impact

- `sebas-webui/frontend/src/views/transcript-view.ts`：回合内分组逻辑（单一 process 块 → 交替 text-run/process-run 序列）、模板与样式（去卡片、轻底色、折叠行摘要）。
- 不改流式管道（core 通道 / WS 事件词汇照旧）、不改增量同步与游标、不改已读缝计数（仍按 turn）。

## Non-goals

- 不改流式管道与通道词汇（live-turn-stream 既有契约）。
- 不改已读缝/未读锚语义与贴底滚动行为。
- 不改二级折叠的结构化标题规则（Process fold titles 需求沿用）。
- 错误条目仍按既有相邻合并规则渲染为独立计数气泡。
- agent 作者标签（agentDisplay 解析、首字形头像）语义不变。
