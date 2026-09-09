## Why

`session-persistence` 的语义已由 `retire-session-persistence` 归档拆归（默认选择 → provider-management、运行时状态边界 → state-store），其主 spec 文件已从 `openspec/specs/` 物理删除。本 change 补显式退役记录，使目录删减与逐 requirement 去向在 change 历史中可追溯。

## What Changes

- 记录 `session-persistence`（2 条 requirement）退役；逐条 Reason/Migration 见 REMOVED delta。
- 无新行为、无 spec 文件变更——主 spec 已随 retire-session-persistence 归档删除，本 change 仅为记录。

## Capabilities

### New Capabilities

### Modified Capabilities

### Removed Capabilities
- `session-persistence`: 语义并入 provider-management + state-store，目录退役

## Impact

- 变更仅落在 `changes/`，`openspec/specs/` 无新增改动。
- **Non-goals**：不复活已删 requirement；不改 provider-management/state-store 文本。
