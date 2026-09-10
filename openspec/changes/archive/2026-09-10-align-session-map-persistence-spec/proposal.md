## Why

`state-store` spec「Runtime state boundaries for persisted session state」声称「agent session map SHALL be persisted in the state store and written per mutation」且「survives unclean exit」——但 `add-state-store` change 的设计把 session-map 迁移标记为预留（「内容先行，迁移后至」），DB 里的 `session_map` 表是一个预留占位（字段形状与现存的 ChannelKey→DTO 映射不符，缺 `acp_session_id/current_model/pending_kind`），代码实测是 shutdown-only 写 `~/.config/sebas/sessions.json`。spec 过早承诺了设计明确推迟的能力，且 DB 表是死代码。

Glossary 第三节已确立「reactions 内容先行，迁移后至」的运行手册约定是合法的；session-map 持久化应同样如实记录为 deferred。

## What Changes

- **`state-store`**：把「Runtime state boundaries」requirement 重写为——session map 现由 shutdown-only JSON 快照持久化（session-lifecycle capability 为权威行为源），state store 的 `session_map` 表为预留占位、尚未承载会话；删除「written per mutation」「survives unclean exit（反映最后提交状态）」这两条未实现的 SHALL。
- 注明迁移路径：session-map 落 DB 留待 `add-state-store` 设计里的 mapping change 实施时按 mapping 结构定表。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `state-store`: 「Runtime state boundaries for persisted session state」requirement 收紧为如实反映 shutdown-only JSON 持久化 + 预留占位表。

## Impact

- 纯 spec 文本收紧；无代码变化。`session_boot.rs` / `dump_json` 行为不变。
- 解决经审计发现的 state-store spec 与 `session-lifecycle` spec（后者如实描述文件机制）之间的内部矛盾。
- C1（newer-DB 拒绝启动）已在代码侧修复并附回归测试，与本 spec delta 无关。

## Non-goals

- 不在本 change 实现 per-mutation DB 持久化（属 deferred 的 mapping change）。
- 不删除预留的 `session_map` 表（设计与迁移约定保留它）。
- 不改 `session-lifecycle` 的 restart-recovery requirement（它已如实描述文件机制）。
