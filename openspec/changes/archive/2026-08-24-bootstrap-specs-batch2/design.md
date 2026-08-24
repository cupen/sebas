## Context

承接 `bootstrap-specs`（已归档）确立的模板：spec 英文 + `## Purpose` + ADDED Requirements + `#### Scenario` WHEN/THEN；proposal/design 中文。本批 4 个 capability 是接入与会话的地基层，彼此关系：

```
feishu-bridge ──入站文本/事件──▶ router-commands ──命令判定──▶ session-lifecycle
       ▲                              │                            │
       │出站卡片/反应                  │ /upgrade 等                 │ spawn env
       │                              ▼                            ▼
       └──────────────────── session-persistence ◀── 状态读写（映射/默认选择）
```

边界约定（与试点 D2 同思路——按行为环路切，不按 crate 切）：
- `feishu-bridge` 止于「事件已解析成路由层输入」；卡片怎么渲染归 `feishu-cards`（下一批）
- `router-commands` 止于「命令已判定去向」；watchdog 收到命令后干什么归 `watchdog`（第四批）
- `session-lifecycle` 止于「映射/会话状态机」；state.json 什么时候写、怎么迁移归 `session-persistence`
- `session-persistence` 只管**文件格式与存取语义**，不管业务字段含义（default_selection 的业务语义归 `provider-management`，下一批）

## Goals / Non-Goals

**Goals:**

- 每份 spec 只描述**可观察行为**；实现细节（函数名、结构体字段）不出现
- 把竞态防护（Spawning 占位）、失败兜底（损坏回退默认、WS 断线重连）这类**不变量**写成显式 requirement
- 与试点保持完全一致的格式，验证批量化回填的可行性

**Non-Goals:**

- 不覆盖 watchdog RPC 协议、卡片渲染、媒体处理、provider 三模式（后续批次）
- 不追历史——只写当前行为，包括已知缺陷也是「当前行为」如实记录（若发现 spec 与代码矛盾，以代码为准并记录）

## Decisions

### D1: 四份 spec 的切分以「数据流职责」为准

feishu-bridge（通道）→ router-commands（决策）→ session-lifecycle（状态机）→ session-persistence（落盘）。备选「按 crate 切」被否：router crate 同时承载命令解析与状态机，切不开；ws_loop 在 src/ 而 events 在 feishu/，跨 crate。

### D2: 命令行为逐条核实，不照抄 HELP_TEXT

HELP_TEXT 是文案不是行为。每个命令的无会话行为（发帮助卡？静默？新起会话？）必须从路由代码+测试核实后写入 scenario。这是本批最容易出现「spec 想当然」的地方。

### D3: 持久化 spec 写「什么时候写」的触发点，不只写格式

state.json 的写触发时机（会话事件后立即写 vs 关闭时写）是行为契约的一部分——重启恢复的可达性取决于它。从代码核实实际触发点写入。

### D4: 已知怪癖如实入 spec

如 double-slash 转义（`//text` → 透传 prompt）、群聊必须 @bot、thread 内回复走 thread——这些是用户可观察的契约，即使像实现怪癖，也写进 scenario。

### D5: 沿用试点的语言与粒度决策

spec 英文（D6 of pilot）、每 requirement 一个行为簇、每 scenario 一个可观察分支。不再重新讨论。

## Risks / Trade-offs

- [研究 agent 转述失真] → Mitigation: 要求 file:line 引用；写 spec 时对关键行为抽查源码；不确定的标 Open Question 或降格为保守表述
- [四份一次归档，review 负担大] → Mitigation: 每份 spec 独立成文件，可逐份审；批内无交叉引用，改一份不影响其他
- [行为与测试断言不一致（预存失败测试）] → 以代码当前行为为准写入 spec，spec 是「当前真相」不是「期望」

## Migration Plan

纯文档。`openspec validate bootstrap-specs-batch2 --strict` → 审阅 → `openspec archive`。回滚 = 删 `openspec/specs/` 下对应四个目录。

## Open Questions

无——四块都有充足代码与测试可核实。
