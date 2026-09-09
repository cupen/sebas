# Design — clarify-driver-layer-boundary

## Context

agent-driver 与 acp-driver 的历史文本互指模糊：agent-driver 的 Purpose 声称 trait 实现（ClaudeDriver/AcpDriver）各自 spawn 子进程，acp-driver 则自称管「one Claude Code subprocess per session」却含通用 ACP requirement（session/load、config options、model switch）。两个 capability 都描述 ACP 子进程生命周期，读者无法定位归属。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：确立「抽象/策略层（agent-driver）vs ACP 子进程运行时层（acp-driver）」边界；两侧 Purpose 与关键 requirement 文本澄清；不动 SHALL 语义。
- **非目标**：不迁移/不合并/不拆分任何 requirement；不改源码；不改目录名。

## Decisions

### 决策 1：分「抽象/策略层」与「运行时层」，不重构代码假设

agent-driver 拥有的真实职责是 kind 解析、开放注册、权限跨驱动路由、可达性（其 5 条 requirement 全是这些）；acp-driver 拥有的是单子进程生命周期与 ACP 协议细节（13 条 requirement）。把 agent-driver 的「AcpDriver spawns native ACP」解读为「通过运行时层执行」而非「自己实现运行时」，两侧即无重叠。

### 决策 2：Purpose 修订走主 spec 直改，requirement 澄清走 delta

openspec 归档器不支持 Purpose 级 delta；按 instructions 约定，Purpose 修订直接编辑主 spec，requirement 的边界澄清以 MODIFIED delta 记录（归档时合入）。被 MODIFIED 的 requirement（AgentDriver abstraction / One subprocess per session）全文保留既有 SHALL 语义，仅加边界句与一个新场景（delegates to runtime / serves any kind）。
