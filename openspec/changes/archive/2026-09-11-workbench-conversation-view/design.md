## Context

动机见 `proposal.md`。塑造本设计的现状与约束：

- **前端已经为对话视图写好了一半**：`transcript-view.ts` 按 `element_type === 'prompt'` 渲染「你」的气泡（`is-user` / `you` 作者 / accent-soft 底），但该分支永远收不到数据。
- **wire 在交付前丢掉了用户条目**：`sebas-webui/src/api.rs:158-166` 的 `filter(|e| e.kind != "prompt")`，并把 `element_type` 折叠成 `markdown | thinking | error`——注意它读的是 `TurnEntry.element_type`，与 `kind` 是两个字段。
- **条目粒度 = 流式 chunk**：每个 `TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolEnd` 各是一条 `TurnEntry`，`position = log.len()` 单调追加（`engine/mod.rs:473-478`、`:724-750`）。工具条目被写成普通 `markdown`，正文前缀 `📖 **` / `✓ **`。
- **实时机制**：单条共享 WebSocket → 视图 `refetch()` 全量重取（`dashboard.ts:246`）；core channel 的 turn 检索已带 position 游标（`Turn content retrieval`），但 webui 的 HTTP 面没用它。
- **两个面重复**：`dashboard.ts`（`/`）与 `session-detail.ts`（`/sessions/:key`）各有一份 transcript + composer；rail 点击 `navigate('/sessions/<key>')`，rail 的 current 标记看 `location.pathname`（`project-rail.ts:415`）。
- **模型目录已可读但没被读**：`/admin/providers` 每项带 `models`，webui BFF 的 `/router/api/providers` 原样透传（`routes.rs:168`）；`/admin/defaults` 给出默认 provider/model，但**没有 BFF 路由**；`/api/router` 的 `ProviderInfo` 反而把 `models` 丢了（`models.rs:191-199`）。
- **依赖**：上一个 change `workbench-turn-queue` 保证「提交只在开轮时进入 transcript」，本 change 的对话顺序建立在这个不变量上。

## Goals / Non-Goals

**Goals：**

- 一次 agent 回合是一个**显示单位**（一个气泡），而不是 N 个 chunk 气泡。
- 对话两侧都在同一个有序序列里，顺序真实（不需要靠时间戳猜边界）。
- 只有一个对话面；书签与既有入口不失效。
- 模型选择器读「Settings 里配的目录」，且**不解释**该目录的结构。

**Non-Goals：**

- 队列语义、待生效堆叠区（`workbench-turn-queue`）。
- `models` 数据结构（顺序=强弱档、`[1m]` 后缀）的规范化——独立 change。
- 工具产出的独立面板（diff/文件）——工具调用留在气泡内的组里。
- 增量 turn 传输（HTTP 面接 position 游标）——先沿用全量 refetch，见 Risks。

## Decisions

### D1：payload 只留一条有序条目序列

- **选择**：`GET /api/sessions/{key}` 与 summary 的聚焦会话返回 `entries: [{position, kind, element_type, content, created_at_unix}]`；`user_prompt`（单条）与 `body`（只有 agent 输出）退役。
- **理由**：两侧交替要求同一序列；两个字段意味着客户端必须自己按时间戳重建顺序，而时间戳只到秒（`created_at_unix`），同一秒内的条目无法定序。
- **备选**：保留 `body` 并另加 `prompts` 数组——否决（两处真相 + 客户端合并逻辑 + 同秒歧义）；沿用 `user_prompt` 只显示最后一条——否决（正是现在的病）。

### D2：`kind` 与 `element_type` 都上 wire，工具条目打 `tool` 标签

- **选择**：条目带 `kind ∈ {prompt, content}` 与 `element_type ∈ {markdown, thinking, tool, error}`；`ToolStart`/`ToolEnd` 两条 push 点写 `element_type = "tool"`（内容仍是可读的 markdown，渲染层自己决定怎么折叠）。
- **理由**：前端要把工具调用收进「用了 N 个工具」组，靠字符串前缀（`📖 **`）判断等于把展示细节当契约，改一次文案就静默失效。
- **备选**：前端嗅探前缀——否决；给工具条目单独的 `kind`——否决（`kind` 是「谁说的」，工具输出是 agent 产生的，属 `content`）。

### D3：回合分组放在前端，不动 core 的 transcript 粒度

- **选择**：客户端把条目按 `kind == prompt` 切成回合：每个 prompt 开启一个 operator 回合，其后到下一个 prompt 之前的条目属于随后的 agent 回合。core 的 transcript 仍是 chunk 级追加日志。
- **理由**：transcript 同时供飞书卡片、`turn-content` 检索与 e2e 断言使用，改粒度等于同时改这三者；而显示单位是纯展示概念，属前端。
- **备选**：core 侧把一次回合合并成一条——否决（blast radius 大，且会破坏「position 单调 = 可增量取」的语义）；服务端直接返回回合组——否决（多一层结构，且与 position 语义重复）。

### D4：一个回合内的渲染分块规则

- **选择**：一个 agent 回合按 position 顺序切成**块序列**：
  1. 连续的非 `tool`、非 `thinking` 条目**拼接**成一段文本（工具调用前后的文本因此天然分成两段，而不是硬粘在一起）；
  2. `thinking` 条目收集成气泡内的折叠块（沿用现有 details 折叠）；
  3. `tool` 条目收集成气泡内的「用了 N 个工具」可展开组，按其位置落在对应文本段之间。
- **理由**：agent 常常「说一句 → 调工具 → 再说一句」，把文本跨工具硬拼会读成一段连续论述，与真实时序不符。
- **备选**：整个回合所有文本拼成一段、工具组固定放末尾——否决（时序错）。

### D5：seam（未读边界）按回合计数、锚定回合首

- **选择**：未读边界以「回合」为单位计数，锚定边界下方第一个回合的 `position`，永不落在回合内部；本地已读时间戳（`created_at_unix`）仍是判定依据。
- **理由**：现有 seam 按条目计数（`transcript-view.ts:511`），一个回合几十条 chunk 会显示成「~87 new」，而新视图里只有 1 条新气泡——数字与视觉必须一致。
- **备选**：沿用条目计数——否决（数字与气泡数不符）；改服务端记已读——否决（spec 明确 per-browser、不落服务端）。

### D6：dashboard 成为唯一对话面，session-detail 退休

- **选择**：rail 点击 → `POST /api/sessions/{key}/switch` + `navigate('/')`，dashboard 就地渲染该会话（复用 ② 的待生效堆叠区与现有 composer）；`/sessions/{key}` 深链仍解析，直接渲染 dashboard 聚焦该会话（复用后端「读 detail 即设焦点」的既有语义）。`session-detail.ts` 退休；其独有的 Close、归档、review-cards 搬进 dashboard 的会话头。
- **理由**：两份实现必然漂移（本 change 要改的地方它俩都有）；而 dashboard 已经具备 follow-up/creation 两态的 composer。
- **备选**：保留两页只共享组件——否决（路由、状态、测例仍两份）；反过来让详情页唯一——否决（rail 是主入口，跳走会丢掉项目上下文与创建入口）。

### D7：模型目录经 BFF 读 Settings，SPA 侧留 adapter 缝

- **选择**：新增 BFF `GET /router/api/defaults`（透传 router `/admin/defaults`，用于预选）；目录数据源用已存在的 `/router/api/providers`（含 `models`）。SPA 侧一个 adapter `toModelCatalog(providers, defaults) → { provider, model }[]`，选择器只消费 adapter 的输出。
- **理由**：`models` 的结构即将由独立 change 改造；把「读取」收敛到一个函数，结构变化只改这里，不做任何解释（顺序、`[1m]` 后缀原样展示）。
- **备选**：在 SPA 里直接读原始 payload 并解释 `[1m]`——否决（把待改造的结构散进 UI）；在 webui 后端做规范化——否决（等于现在就定下即将被改的结构）。

### D8：会话内模型面与会话外目录分离

- **选择**：会话已存在时，选择器只提供该会话的 `available_models`；为空则不给下拉、不报错（沿用 `acp-model-selection`）。创建模式才用目录。目录不可得时显示「目录不可用」并禁用选择，绝不伪造选项。
- **理由**：切换要发给该会话的执行体，目录里的模型它未必认；两条来源混在一个下拉里会让人选到必然失败的值。
- **备选**：合并两个来源——否决（必然出现选中即报错的项）。

## Risks / Trade-offs

- [**payload 随会话变长**：条目是 chunk 级，长会话可能上千条，每次 WS → refetch 全量重取] → 前端按 `position` 去重、只追加新条目渲染（重取仍是全量）；HTTP 面接 position 游标留作独立 change（本 change 不改传输，避免与视图重写同时动）。
- [**文本分段规则被误读**：工具调用把一段论述切成两块，用户可能以为是两段话] → 工具组在视觉上明确嵌在两段之间，且组标题带计数，让「中间发生过事」可读。
- [**session-detail 退休的回归面**：书签、Close、归档、review-cards 的入口都会换位置] → 深链与四类入口各写一条浏览器 e2e；`session-detail` 的相关单测随之下移或删除。
- [**模型目录不可达时的静默**：`/router/api/providers` 需要 router 在跑] → 失败必须落到「目录不可用」的显式态（与现有 `provider status unavailable` 同款诚实降级），不许退化成空列表。
- [**与 ② 的时序耦合**：若 ② 未落地，忙中提交会插在在跑的回合中间，本 change 的回合分组就会把它算错] → 依赖顺序写死在 tasks 前置条件里：② 合并后再动本 change。

## Migration Plan

1. **wire**：条目序列（含 `kind`/`element_type`）透出，`user_prompt`/`body` 退役；工具条目标签。
2. **对话视图**：回合分组、文本拼接、thinking 折叠、工具组、seam 改按回合。
3. **面合一**：rail 就地聚焦、dashboard 会话头（Close/归档/review-cards）、`session-detail` 退休、深链兼容。
4. **模型目录**：BFF defaults + adapter + 两级选择器 + 诚实降级。
5. **门禁与 e2e**。

**回滚**：改动集中在 SPA 与 webui payload，唯一对外契约变化是会话 payload 的字段形状，消费者只有本仓库的 SPA（同 change 内切换）。回滚即回到旧 bundle/二进制；core 侧只多了 `element_type = "tool"` 标签，对旧客户端是普通 markdown，向前兼容。
