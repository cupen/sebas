## 1. 覆盖键表与推导纯函数

- [x] 1.1 在 `src/spawn_env.rs` 声明 `CLAUDE_CODE_SUBAGENT_MODEL` 常量并复用 `sebas_router::models::map_to_env` 给出的 4 个 `ANTHROPIC_MODEL*` 槽；写单测 `model_cover_env_single_model_flattens_all_tiers` / `model_cover_env_multi_model_maps_strong_to_weak_and_subagent` 覆盖单模型打平与多模型强弱映射
- [x] 1.2 实现纯函数 `model_cover_env(models: &[String]) -> Vec<(String, String)>`：非空时 5 键全量返回且 `CLAUDE_CODE_SUBAGENT_MODEL` 回退到 haiku 值；空时返回空；`model_cover_env_empty_yields_no_injection` / `model_cover_overrides_inherited_value_semantics` 断言语义

## 2. 决议链接入

- [x] 2.1 在 `session_boot.rs::spawn_overrides` 的公共入口 `spawn_env::resolve_spawn_overrides` 追加 `model_cover_env` 到 extra_env（fresh/resume 共用），并新增 `effective_provider_models` 使其只在 Direct / Off-with-default 生效、Router 与裸 Off 不强制
- [x] 2.2 判定 Router 模式不注入模型覆盖（透传分内事），新增 `resolve_spawn_overrides_router_mode_injects_no_model_cover` 单测锁定
- [x] 2.3 readonly：`--model` 与 `ANTHROPIC_MODEL` 不强制同源（spec 允许 default_selection 与 provider 默认模型分离），以 `directive_uses_mode_provider_over_default_selection_provider`、`direct_default_selection_model_overrides_overlay_default_model` 等既有单测佐证 parity 语义

## 3. 全链路单测与测试

- [x] 3.1 `resolve_spawn_overrides_direct_preset_injects_5_key_model_cover`、`resolve_spawn_overrides_bare_off_injects_no_model_cover`、`resolve_spawn_overrides_router_mode_injects_no_model_cover` 等 e2e 单测覆盖「状态 → 决议 → extra_env 5 键 / 空覆盖」
- [x] 3.2 `cargo test -p sebas --lib spawn_env` 40 / 40 全绿；`cargo test -p sebas-acp` 通过；`cargo clippy -p sebas --lib` 无告警
- [x] 3.3 在 tasks 层、spec 层和单测层共同确认 OS 残留 `ANTHROPIC_MODEL` 会被 `model_cover_env` 生成的值压掉（`model_cover_overrides_inherited_value_semantics` + `override beats inherited values` 场景）；真实沙箱动端暂定延后到 apply 后的集成旅程中开启
