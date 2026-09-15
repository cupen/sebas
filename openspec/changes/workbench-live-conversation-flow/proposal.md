# workbench-live-conversation-flow

## Why

对话流是工作台的核心体验，当前有五处硬伤：

1. 回合内容只在结束后整块出现——core 通道与浏览器 WS 词汇表都不携带回合增量，尽管 ACP 事件流里文本/工具/思考增量现成（`AcpEvent::TextDelta` 等）。
2. 新会话在子进程拉起前，模型芯片永远显示「无可用模型」（`available_models` 要等 spawn 后由 agent 上报）。
3. 会话头占据一排交互按钮（All sessions / Archive / Close / mode），与「会话只集中一种状态」的诉求相悖。
4. rail 与工作台之间约 39px 死空隙，对话区与输入框左右边缘各内缩约 20px 不齐。
5. rail 默认 220px，减去内边距后标题区约 150px，放不下 12 个中文字（≈156px）。

## What Changes

- **实时流式**：core→webui 通道新增回合事件流（文本增量按约 250ms 窗口合并，工具开始/进度/结束与思考增量照发），webui 以新 WS 事件广播，前端把增量就地追加进对话块。快照重取仍是收敛机制，流事件只是增量补充。
- **聚焦即拉起**：聚焦无活子进程的会话（新建占位或重启后重开）后台拉起 ACP 子进程，有历史则按既有映射 resume；模型芯片在拉起窗口显示「启动中…」，拉起后显示 agent 上报的模型——「无可用模型」只在 spawn 完成且确实无模型时出现。
- **输入框底沿重组**：左 = mode 切换（自会话头迁入）；右 = 模型芯片 + 提交按钮；状态词删除，提交按钮图标即状态（箭头/停止方块/排队时钟/转圈）。agent 锁标签移回会话头作纯展示。
- **会话头去交互化**：头部零按钮零链接；归档成为唯一生命周期出口，入口在 rail 会话行悬停菜单，确认弹窗合并 close 的「将丢弃 N 条待执行」提示（归档服务端本就包含关闭语义）。close API 端点保留但 UI 不再调用。
- **流式与已读锚联动**：聚焦且贴底时流式到达即推进读锚（角标不闪、已读缝不出现）；未贴底照常计未读。
- **版面修正**：分隔条收窄、对话区与输入框共用同一套水平内边距（边缘齐平、竖向空隙收紧）；rail 默认 280px（宽视口）、拖拽上限 520px，标题保证 12 个中文字可见。

## Capabilities

### New Capabilities

- `live-turn-stream`: 回合内容从 core 经 webui 到浏览器的流式契约——合并增量、事件词汇、顺序与容错、与已读锚的一致性。

### Modified Capabilities

- `agent-workbench`: 0-turn 占位会话「聚焦即拉起（带 resume）」取代「首条消息才 spawn」；模型芯片增加拉起窗口态；会话头去交互化、归档入口移至 rail 行悬停菜单；新增工作台镶边密度与对齐需求。
- `project-session-actions`: 同步拉起语义与归档入口变化（该能力与 agent-workbench 存在重复需求，同步修改避免规格分叉）。
- `session-unread-badge`: 流式内容在聚焦贴底时增量推进读锚，不闪角标。
- `webui`: 「会话 mode 在 dashboard 可见可切」的切换入口自会话头迁至输入框底沿。

## Impact

- 协议：core 通道（`src/core_channel/`）新增回合事件帧；`sebas-webui` 的 `WebUiEvent` 新增变体；前端 `ws.ts` / `transcript-view.ts` 增量渲染。
- 行为：spawn 触发时机（聚焦 vs 首条消息）、`dashboard` 会话头与 `project-rail` 悬停菜单、`workbench-composer` 底沿构成、`split-persist` rail 宽度默认/上限。
- 兼容：新 WS 事件类型对老前端无害（ws.ts 本就容忍未知类型）；close 端点保留。

## Non-goals

- 不做 slash 命令面板（模型/mode 切换维持下拉控件）。
- 不改已读锚的存储模型（仍为 localStorage 每浏览器，服务端不记录）。
- 不重做 rail 信息架构（History 组、项目分组维持现状）。
- 不退役 close API 端点（仅 UI 停用，退役另立变更）。
- 不做按会话订阅的 WS 通道（维持「所有客户端收全部事件、前端按聚焦过滤」的既有姿态）。
