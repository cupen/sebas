## Why

中间那栏说不上是「对话」。前端的对侧气泡**已经写好了**——`transcript-view.ts` 按 `element_type === 'prompt'` 渲染「你」头像与 accent-soft 气泡——但 `sebas-webui/src/api.rs:162` 组装 `body` 时把所有 `kind == "prompt"` 的条目过滤掉了，那条分支是**死代码**。于是 transcript 里只有 agent 的一串独白，你自己说过的话一条都看不见。

放行之后也不能直接渲染：一次 agent 答复以**流式 chunk** 粒度落条目（每个 `TextDelta` 一条，`engine/mod.rs:724`），而且工具调用被打成普通 `markdown`，与正文**长得一模一样**（靠 `📖 **` / `✓ **` 前缀区分）。所以「多轮对话」缺的不只是数据，还有**回合这个显示单位**。

同时有两个结构性重复：`/` 与 `/sessions/:key` 各写了一份 transcript + composer，而点侧栏会话是 `navigate('/sessions/<key>')`——**点会话就离开工作台**；侧栏的「当前」高亮看的是 `location.pathname`，不是焦点指针（`project-rail.ts:415`）。而创建会话的模型下拉是「偷」最近一个暴露过 `available_models` 的会话（`workbench-composer.ts:295-325`），spec 要求的 backend catalog 背后没有 API——`/api/agent-defaults` 已随 `workbench-agent-wire-fix 3.3` 退役。

## What Changes

**BREAKING（wire）**：

- `GET /api/sessions/{key}` 与 `GET /api/summary` 聚焦会话的 payload：`user_prompt`（单条）与 `body`（只有 agent 输出）**退役**，改为**一条带 `kind` / `element_type` 的条目序列**（用户提交与 agent 输出同一序列、按 position 排序）。
- `/sessions/{key}` **不再是独立详情页**：深链仍解析，但渲染的是同一个工作台 + 聚焦态；`session-detail` 视图退休。

**行为**：

- **一个 agent 回合 = 一个气泡**：连续流式文本按顺序拼接回一个气泡；thinking 折进气泡内；工具调用收进气泡内一个「用了 N 个工具」可展开组——为此 `ToolStart`/`ToolEnd` 两条 push 点新增 `element_type = "tool"` 标签（不再靠 emoji 前缀当契约）。
- **一条提交在它开轮时以 operator 气泡出现**（时机承接 `workbench-turn-queue`），对话两侧交替、顺序真实。
- **两个面合一**：点侧栏会话**就地聚焦**（留在 `/`），rail 的当前标记改看焦点指针；Close、归档、review-cards 从详情页搬进工作台。
- **模型选择器接真目录**：读 Settings 里配置的 provider/models，**两级**（先 provider 后 model）；会话已存在时让位给该会话的 `available_models`（切换要发给 agent，只有它认的才算数）；目录不可得时如实显示不可用，不伪造选项。SPA 侧留一个 adapter 缝，`models` 结构改造只换这一处。

## Capabilities

### Modified Capabilities

- `agent-workbench`：新增「工作台把聚焦会话渲染为对话（一个回合一个气泡 + 工具组）」「工作台是唯一对话面（就地聚焦）」；修订「Model selector offers the backend catalog before any session」（目录真源 = Settings）与「Unseen-turn seam」（seam 落在回合之间、按回合计数）。
- `webui`：修订「Session dashboard and focus semantics」（就地聚焦、rail 标记看焦点指针）；新增「会话 payload 承载对话条目」。
- `core-session-channel`：修订「Turn content retrieval」（条目带 `kind`/`element_type`，prompt 条目不丢，工具可辨）。
- `acp-model-selection`：修订「Session model list is exposed」（agent 不暴露模型面时会话内不给切换，但创建时仍可从 Settings 目录选下一个会话的模型）。

## Impact

- **前端（主要）**：`views/transcript-view.ts`（→ 对话视图：回合分组、拼接、工具组）、`views/dashboard.ts` 与 `views/session-detail.ts`（合一，后者退休）、`views/project-rail.ts`（点击就地聚焦、当前标记看焦点）、`views/workbench-composer.ts` + `views/settings-modal.ts`（模型目录两级选择）、`api/client.ts`（payload 类型与端点）、`components/`（工具组、提交气泡）。
- **Rust**：`sebas-webui/src/api.rs`（payload 组装：不再过滤 prompt、透出 kind/element_type）、`sebas-webui/src/models.rs`、`sebas-dispatch/src/engine/mod.rs`（工具条目标签）、`sebas-webui/src/routes.rs`（`/router/api/defaults` 预选路由）。
- **e2e**：`tests/testsuite-webui-browser`（对话视图两侧交替、工具组展开、就地聚焦、模型两级选择）；`tests/testsuite-process-e2e`（条目 role/element_type 透出）。
- **文档**：`openspec/glossary.md`（turn / 回合 作为显示单位；若与既有词条冲突先改 glossary）。

## Non-goals

- **队列语义与待生效堆叠区**——上一个 change `workbench-turn-queue` 负责；本 change 假定提交只在开轮时进入对话。
- **`models` 数据结构优化**（顺序=强弱档、`[1m]` 后缀那套）——另开 change；本 change 只读、不解释该结构，靠 adapter 缝隔离。
- **工具产出的独立工作物面板**（diff / 文件 / 命令结果抽栏）——本次工具调用收在气泡内；独立面板是后续独立设计。
- **token 级打字机流式**——沿用「条目追加 + refetch」的既有实时机制，不做逐字动画。
- **移动端/窄屏布局重构**——沿用既有响应式规则。
