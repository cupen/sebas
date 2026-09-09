## Why

`feishu-cards` 目前同时承载两类语义：**中立卡片内容契约**（per-turn 卡片实例、流式累积、终态渲染、thinking 折叠、截断/预算/轮换、生命周期）与**飞书渲染专属**（schema 2.0 JSON 元素、help/error/status 卡、主题、`root_id` threading）。glossary 已把「卡片」定义为通道中立呈现模型（由适配器渲染成渠道形态），但中立契约仍藏在 feishu 前缀的 capability 里——`channels` 的「Neutral outbound presentation」只有高层定义，im-service 视 feishu-cards 为卡片状态机规格。中立模型应由 `channels` 持有，`feishu-cards` 收窄为纯飞书渲染器。

## What Changes

- **`channels` ADDED「Neutral presentation content contract」**：定义中立累积呈现模型的内容契约——per-turn 单实例、frozen-at-turn-end、流式合并（coalesce）、终态呈现、thinking 折叠/丢弃策略、内容预算与轮换、生命周期清理。全部用通道无关措辞（不出现 feishu schema 2.0 / emoji 文案 / `root_id`）。
- **`feishu-cards` 收窄为渲染专属**：REMOVED 迁入 channels 的中立契约 requirement（Per-turn card model、Card structure、Terminal state rendering、Thinking display policy、Streaming update cadence、Long-content truncation、Body budget and rotation、Card lifecycle cleanup），MODIFIED 保留飞书渲染 requirement 并重写为纯 renderer 视角（Interactive card JSON、Help card、Error/status cards、Card theme、Feishu adapter renders the neutral presentation model）；Purpose 改为飞书卡片渲染器。
- 目录名 `feishu-cards` 沿用至批次 E（按新 split 改名 `cards` + `feishu-render` 或并入 feishu-bridge）。

## Capabilities

### New Capabilities

### Modified Capabilities
- `channels`: 新增中立呈现内容契约 requirement
- `feishu-cards`: 中立契约迁出，收窄为飞书渲染专属（文本修订）

## Impact

- im-service L38「卡片状态机…遵循 feishu-cards」改指 `channels`（中立契约）；feishu-bridge L173「per feishu-cards rendering rules」保持（渲染规则仍在）。
- glossary 卡片词条引用 `(channels;feishu-cards)` 微调措辞。
- **Non-goals**：不改任何源码行为；不搬移 requirement 的 SHALL 语义（只改归属与措辞视角）；`cards`/`feishu-*` 目录改名留批次 E。
