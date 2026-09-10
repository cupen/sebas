## Why

`core-session-channel/spec.md` is structurally corrupted and factually wrong in three places (sebas-zer): **(1)** the「Session drive methods」requirement is duplicated — a stale copy (lines 120–171, claiming no `cancel`, no ensure/attachments, "spawns a real ACP session") is interleaved into the current one (which adds `cancel`, IM-frontend ensure semantics, and attachments), plus orphaned duplicate scenario lines (173–179) — a reader cannot tell which is authoritative; **(2)** the channel transport requirement claims the default socket path is `~/.sebas/core.sock`, but the code default is `$XDG_RUNTIME_DIR/sebas/core.sock` with a per-uid temp fallback (`src/core_channel/server.rs:31-43`, `src/config.rs:373-376`); **(3)** the ensure-message resume scenario promises a `Revived` event that does not exist (SessionEvent is only `Created/Updated/Removed/Resync`; a dormant resume is delivered as `Updated`, confirmed by `src/core_channel/tests.rs:1322`).

## What Changes

- **`core-session-channel`**: 重构「Session drive methods」为单一权威块（含 create + 可选 project dir + 可选执行体 hint + cancel、IM-frontend ensure 语义、附件），删除陈旧重复块与孤立重复场景行。
- 修正「Channel transport and authentication」默认 socket 路径为 `$XDG_RUNTIME_DIR/sebas/core.sock`（per-uid 临时回退），删除 `~/.sebas/core.sock` 错误声称。
- 修正 ensure 复活场景：`Revived` → `Updated`（订阅流无独立 Revived 帧）。

## Capabilities

### New Capabilities
（无）

### Modified Capabilities
- `core-session-channel`: 上述三个 requirement 修正。

## Impact

- 纯 spec 文本修复；无代码变化。socket 路径、Revived→Updated、Session drive 行为均已是现状代码。

## Non-goals

- 不改 `Rejected` 的 serde 标签（`code`）或 StateSubscribe 形状（另有 MED 发现，留待单独 change）。
- 不触碰其它 spec。
