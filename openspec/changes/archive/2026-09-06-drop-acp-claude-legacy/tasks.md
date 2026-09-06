## 1. 应用主 specs 对齐

- [x] 1.1 `openspec/specs/agent-driver/spec.md`:删除 "Configuration shape with backward-compatible migration" 需求与 "Existing claude-only config keeps working" 场景;Purpose 行去掉 "配置 schema 从 `acp.claude` 迁移到 `acp.agents.<kind>`" 子句;"Bare default resolves to the sole configured agent" 场景改挂到新"L Legacy `[acp.claude]` block is rejected"需求下作为新块 OK 场景之一;验证:`grep -nE "acp\.claude" openspec/specs/agent-driver/spec.md` 仅剩新增的拒绝相关语句

## 2. 拆除代码兼容层(BREAKING)

- [x] 2.1 `src/config.rs`:删除 `AcpConfig.claude` 字段、`migrate_legacy_claude()` 函数、`Config::parse` 中 `cfg.acp.migrate_legacy_claude()` 调用。验证:`grep -n "migrate_legacy_claude\|self\.claude" src/config.rs` 归零
- [x] 2.2 相关测试改为断言旧块解析报错,新块加载路径保留 implicit-default 单 agent 场景。验证:`cargo test` 全 workspace 通过
- [x] 2.3 `cargo build` 0 本次引入警告,`openspec validate drop-acp-claude-legacy --strict` 通过

## 3. 验证与归档

- [x] 3.1 全量扫描:`grep -rniE "\[acp\.claude\]|migrate_legacy_claude" openspec/ src sebas-*/src` 归零(注释/changelog 历史除外);`cargo test --no-fail-fast` 通过、`cargo test --test core_flow_e2e_test -- --ignored` 与 `--test acceptance_suite_test -- --ignored` 通过
- [x] 3.2 归档本 change,确认主 specs REMOVED + ADDED 落地(agent-driver 场景数从原 4 → 3 + ADDED 2)