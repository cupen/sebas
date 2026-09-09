# Tasks: clarify-feishu-optional-credentials

## 1. Spec 归档与同步

- [x] 1.1 归档本 change 前 `openspec validate` 通过；归档时把 delta 合入 `openspec/specs/feishu-option/spec.md`（替换「缺省值 SHALL 为 false」矛盾措辞、并入半配置与 env 时机场景），verify: `openspec validate --specs` 通过且主 spec 含「凭据半配置拒绝启动」场景

## 2. 实现对齐（行为不变，仅校验一致性）

- [x] 2.1 核对 `src/config.rs` validate() 的两条 feishu 校验与 delta 措辞一致（半配置拒绝、enabled=true 凭据缺失拒绝），如注释与 spec 措辞有出入仅调整注释；verify: `cargo test --lib config` 全绿
- [x] 2.2 按 delta 新增场景补断言：env 仅覆盖其一 → 半配置报错（放 `tests/config_env_test.rs`，独立进程避免 set_var 竞争）；TOML+env 双源齐备 → 接入；verify: `cargo test --test config_env_test` 全绿
- [x] 2.3 确认既有矩阵测试覆盖 delta 各场景（`feishu_section_with_empty_credentials_is_optional`、`feishu_explicit_enabled_switch_four_states`、`feishu_optional_but_not_half_configured`），缺口补进 `tests/config_test.rs`；verify: `cargo test --test config_test` 全绿
