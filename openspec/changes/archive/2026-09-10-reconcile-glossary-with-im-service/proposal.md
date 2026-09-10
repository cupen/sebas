## Why

`openspec/glossary.md` 自称术语的单一事实来源，但四处定义与现实（以及它自身）矛盾，会误导读者与后续 change：**(1)** 第 11 行 core 定义说「按配置承载通道适配器(飞书 WS 等)」，同文件第 94 行又说「core 是纯会话核心，不注册任何 IM 适配器」——直接打架，前者是 ADR 年代 architecture.md 的直引；**(2)** 第 20 行 dispatch 定义含「出站呈现编排」，但该职责已移交 `sebas-im` 前端（`channels` spec 明规「core SHALL NOT hold any IM-facing presentation state」）；**(3)** 「escalate」一词二义（native 审批决策「带理由的一次性放行」vs `acp-driver` 挂起检测的击杀阶梯），未像「router 三义」那样消解；**(4)** 「ACP」一词二义且历史文档读起来像自相矛盾——`docs/design-history.md` ADR-1（08-06）说「弃用 ACP」，`multi-third-party-acp-agents`（09-03）又「引入 ACP」，实为两个不同的东西（ADR-1 弃的是 Claude 私有的 `claude-acp-bridge` 转码桥；现行是开源标准 `agent-client-protocol` v2 驱动第三方 agent，二者共享同一缩写）。

## What Changes

- **core 定义去矛盾**：第 11 行删去「按配置承载通道适配器(飞书 WS 等)」，改述为「IM 适配器宿主是独立的 im 服务；core 不注册任何 IM 适配器」，与第 94 行及 `im-service`/`channels` spec 对齐。
- **dispatch 定义收窄**：第 20 行「出站呈现编排」改为「出站 Out 指令编排（会话执行向；IM 呈现由 sebas-im 前端负责）」。
- **新增「escalate 二义消解」**：在「三义消解」块（或易混对照表）补一条——approval `escalate`（native gated-call 的一次性放行，`ApprovalAnswer::Escalate{reason}`）vs `acp-driver` 挂起检测的 kill ladder（interrupt×3 → SIGTERM → SIGKILL）；并注明 ACP 路径无 escalate 等价、降级 allow_once（见 `unify-permission-approval-vocabulary`）。
- **新增「ACP 二义消解」**：与「router 三义消解」并列——(a) 历史 `claude-acp-bridge`：Claude 私有转码桥，ADR-1（08-06）已弃用并删除，**不再存在**；(b) 现行 `agent-client-protocol`（v2，`sebas-acp` 依赖）：开源标准，驱动原生 ACP 第三方 agent（gemini/copilot/opencode 等）。注明 ADR-1 弃的是 (a) 而非 ACP 标准本身，消解历史文档的表面矛盾。
- **（可选，随 tasks）** 易混对照表补一行 core vs im 服务。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

（无）—— glossary 不在 `openspec/specs/` 下，本次为纯文档修订，无 spec 级行为变化。`skip_specs: true`。

## Impact

- 仅 `openspec/glossary.md` 文本修订；无代码、无 API、无行为变化。
- 依赖关系：与 `unify-permission-approval-vocabulary`（escalate 词表）措辞互引，建议其先行或同期落地。
- 影响读者：所有 spec 的术语引用。

## Non-goals

- 不修订 `docs/design-history.md` 的 ADR 条目（历史归档，另评估「已被取代」标注）。
- 不重写 glossary 其余健康条目（三义消解、capability 命名规则等）。
- 不触及 feishu-reactions 的管线重写（已另立 `respec-reactions-for-im-service`）。
