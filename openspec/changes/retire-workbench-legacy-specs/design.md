# Design — retire-workbench-legacy-specs

## Context

`openspec archive consolidate-workbench-session-actions` 已创建 `workbench`（18 条 ADDED）并从主树删除三个旧 spec 目录。openspec 1.10 的归档器不支持"REMOVED 清空整 capability"（拒绝空 spec 重建），故退役动作拆成两步：consolidate（ADDED + 物理删除）与 retire（REMOVED 记录）。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：为三目录退役留下 change 级记录，逐 requirement 说明去向；让 `changes/archive/` 里的 consolidate 变更可独立理解。
- **非目标**：不复活已删 spec；不改 `workbench`/`webui` 文本；不做命名族对齐。

## Decisions

### 决策 1：退役记录 = 独立 change 的 REMOVED delta，不做二次归档

三个 REMOVED delta 在 retire change 中仅作记录；归档器运行会因"目标 spec 已不存在"而失败，因此本 change 用 `skip_specs` 语义处理——delta 保留在 change 内作为记录，change 以文档形式归档（或随 D 批整体收口归档）。备选：把 REMOVED 塞回 consolidate 已归档包——污染既有归档、且工具已拒绝，弃。

### 决策 2：REMOVED delta 的目标路径对已删 spec 只作记录定位

`specs/agent-workbench/spec.md` 等 delta 文件名仅为记录归属，不指向现存主 spec；归档器不会消费它们。备选：无——工具无整能力退役路径，记录价值由文件位置承载。
