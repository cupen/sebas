## Why

`agent-driver` spec 把 `[acp.claude]` legacy 块迁移+告警当作现行需求描述（"Configuration shape with backward-compatible migration"），代码侧 `src/config.rs::migrate_legacy_claude()` 仍在 parse 时静默兜底——这是 multi-third-party-acp-agents 留下的兼容面，与本次已确立的“未发布不留兼容”口径不符。其它模板/文档（`config/config.toml.example`、`docs/acp-opencode-*`、ansible 模板）一致使用 `[acp.agents.<kind>]`，仅 spec 与兜底代码两处例外，应一并拆除。

## What Changes

- **BREAKING** `agent-driver` spec:删除"Configuration shape with backward-compatible migration"需求及其两个场景；Purpose 行去掉“从 `acp.claude` 迁移到 `acp.agents.<kind>`”措辞（schema 现状即新形态，无需陈述迁移）。
- **BREAKING** `src/config.rs::AcpConfig`:删除 `claude: Option<AcpClaudeConfig>` 字段与 `migrate_legacy_claude()` 函数；parse 阶段不再迁移旧块——配置含 `[acp.claude]` 段则按 serde 未知键报错（`toml::de::Error` 路径，错误信息含 `[acp.claude]` 字样）。
- `agent-driver` spec "Bare default resolves to the sole configured agent" 场景保留：`default` 缺省且仅一个 agent 时仍隐式取它（这是新形态的现行行为，不是兼容层）。

## Non-goals

- 不动 `AcpConfig.agents.<kind>` 形态本身的语义。
- 不改 ansible 模板与 `config/config.toml.example`（已用新名）。
- 不重写 agent-driver 其余需求。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `agent-driver`:删除 Configuration shape with backward-compatible migration 需求及其两个场景；Purpose 措辞去掉迁移语句。

## Impact

- Spec:`openspec/specs/agent-driver/spec.md`(删除 1 个需求 + 2 个场景 + 修 Purpose 一行)。
- 代码:`src/config.rs`(删除 `AcpConfig.claude` 字段、`migrate_legacy_claude()` 函数与 parse 中的调用,约 30 行);相关测试如 `serde_legacy_claude_block_migrates` 类断言改为断言旧块解析报错。
- 配置:`[acp.claude]` 块 TOML 解析失败——无存量用户(未发布),属可接受的硬切。