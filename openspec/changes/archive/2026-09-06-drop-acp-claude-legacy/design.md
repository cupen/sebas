## Context

`agent-driver` spec 与 `src/config.rs::migrate_legacy_claude()` 把"`[acp.claude]` 一次性迁移到 `[acp.agents.claude]`" 当作现行行为——这是 multi-third-party-acp-agents 落地时的兼容面。其它全部表面(模板、文档、ansible、spec 其余部分)都用 `[acp.agents.<kind>]`,仅 spec 一条需求与一段 parse 兜底仍存活。按"未发布不留兼容"口径,本次一并拆除。

## Goals / Non-Goals

**Goals:**

- 旧块走 TOML 反序列化拒绝路径,不再静默重写;
- spec 删除 legacy 迁移需求、改写为"旧块必须解析失败"的现行行为;
- 唯一 agent 默认解析逻辑保留(它服务新形态)。

**Non-Goals:**

- 不动 `acp.agents.<kind>` 形态本身;
- 不改 ansible/模板/已使用新名的文档;
- 不重写 agent-driver 其余需求。

## Decisions

- **REMOVED + ADDED 而非 MODIFIED**:移除的"Configuration shape with backward-compatible migration"已有 Reason/Migration(此为 zod/markdown 校验下 REMOVED 必须含的最小骨架)——这是文档化的"删除原因+迁移路径",不会丢失意图;新增"L Legacy `[acp.claude]` block is rejected"用两个场景固化新语义(旧块 fail-parse / 新块仍 implicit-default)。
- **代码侧不写专项错误**:复用 toml 默认反序列拒绝路径(serde 会把多余字段以 unknown variant 报错,错误信息含 `[acp.claude]` 字样),不另写自定义错误文案,避免迁移文字被人手工绕过。

## Risks / Trade-offs

- [serde unknown-field 报错行号与字段名在不同 toml 版本间略有差异] → 用户从错误信息中能直接定位 `[acp.claude]` 表头与迁移指示,即能纠错。

## Migration Plan

无存量部署。归档即生效,用户配置侧如遇旧块按 reason/migration 段落就地修复。