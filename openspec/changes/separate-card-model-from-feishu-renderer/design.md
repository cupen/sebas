# Design — separate-card-model-from-feishu-renderer

## Context

channels 的「Neutral outbound presentation」只给了中立呈现模型的高层定义（per-turn 实例、streaming、freeze、interactive elements），内容契约细节（budget、rotation、terminal 态、thinking 策略）散在 feishu-cards 且用飞书词汇书写。glossary 卡片词条已把「卡片」定义为中立呈现模型。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：中立内容契约迁入 channels（通道无关措辞）；feishu-cards 收窄为渲染器专属并显式收编 emoji/布局等 renderer 词汇；不丢任何 SHALL 语义。
- **非目标**：不改源码；不改 im-service 的卡片归属语义；不在此 change 做 `cards`/`feishu-*` 目录改名（批次 E）。

## Decisions

### 决策 1：channels 用一个综合 requirement 收编全部中立契约

不逐条平移 feishu-cards 的 8 条（会制造 channels 与 feishu-cards 的字面重复），而是合成单一「Neutral presentation content contract」requirement（per-turn 实例/frozen/coalesce/terminal/thinking/budget/rotation/lifecycle），8 个场景覆盖原语义。

### 决策 2：飞书 renderer 词汇显式下沉为 layout details

emoji（💭/🤔/✅/📎/❌）、折叠文案（已折叠 N 字）、footer 格式在收窄后仍存在（它们是真实渲染行为），故新增「Feishu card layout details」requirement 显式声明这些是 renderer 词汇——避免语义丢失又保持中立/专属边界。

### 决策 3：MODIFIED 而非 RENAMED

5 条保留的 renderer requirement（Interactive/Help/Error/Theme/Renders-neutral）语义与主体不变，仅微调措辞（去掉中立部分、明确引用 channels 契约），故用 MODIFIED 全量重写；新增 layout details 用 ADDED。
