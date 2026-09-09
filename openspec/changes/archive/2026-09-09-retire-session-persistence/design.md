# Design — retire-session-persistence

## Context

session-persistence 是 state-store 落地前的过渡 capability，自述「migrating to the core state store」；其两条 requirement 分属两个 domain：默认选择语义（provider 行为，provider-management 的「Set default provider and model from the page」已覆盖页面侧）与运行时状态不持久化边界（持久化载体语义，state-store 的「State methods on the core channel」已含 session map 的载体）。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：两条 requirement 语义无损并入接收方；session-persistence 目录退役；核心集矩阵同步。
- **非目标**：不改源码行为；不新造持久化模型；不重排 provider-management/state-store 既有 requirement 文本。

## Decisions

### 决策 1：按 domain 拆归，不整体并入单一方

「Default selection semantics」随 provider 域进 provider-management；「Runtime state is not persisted」随持久化载体域进 state-store。备选：整体并入 provider-management——会把「不持久化清单 + session map 每变更持久」这类载体契约错误塞进 provider 行为域，弃。

### 决策 2：退役用 retire 记录 change，与 D1 同一模式

D1 证实 openspec 1.10 归档器拒绝「REMOVED 清空整 capability」。本 change 只做两个 MODIFIED delta（ADDED 语义）；session-persistence 的物理删除与 REMOVED 记录并入随后的 retire 记录（或在批次 F 统一退役收口），避免 D1 遇过的「归档器重建空 spec 失败」。
