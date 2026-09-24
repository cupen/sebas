## 1. 状态库文件权限

- [x] 1.1 `sebas-db` 的 open 在创建后把库文件设为 0600（Unix），并在打开既有库时校验权限、过宽则收紧；验证：新增单测断言新建库权限为 0600，且把一个 0644 的既有库打开后变为 0600
  - 证据：`sebas-db/src/conn.rs` 新增 `OWNER_ONLY_FILE = 0o600` / `OWNER_ONLY_DIR = 0o700` / `pub type ChmodFn` / `real_chmod` / `fn tighten_with`（只在更宽时收紧；缺失路径或 chmod 失败 → `tracing::warn!` 不致命）/ `fn sibling` / `pub fn secure_directory`；`open()` → `open_inner(path, &real_chmod)` 收紧库文件 + `-wal` + `-shm`。单测 `conn.rs:267 new_db_file_is_owner_only_0600`、`conn.rs:281 existing_loose_db_file_is_tightened_on_open`。命令：`cargo test -p sebas-db` → passed=48 failed=0。
  - 目录收紧是**独立显式**动作（`secure_directory`，0700），接在状态目录所有者处：`src/run.rs` 状态库初始化块（`settings_path.parent()`）。理由：通用 `open()` 不得 chmod 共享父目录（如 `/tmp`）。
- [x] 1.2 `-wal` / `-shm` 与目录权限一并核对；验证：单测断言 WAL 模式下 `-wal` 文件权限不宽于库文件
  - 证据：`conn.rs:298 wal_sidecar_is_never_wider_than_the_db_file`（保持首个连接存活再 chmod `-wal`——SQLite 在最后一个连接关闭时会删掉 `-wal`，否则 `NotFound`）；`conn.rs:327 state_directory_is_tightened_to_owner_only`。`cargo test -p sebas-db` → passed=48 failed=0。
- [x] 1.3 权限收紧失败时告警但不中止启动；验证：单测模拟不可 chmod 的场景，断言启动继续且日志有 warn
  - 证据：`conn.rs:353 tighten_failure_warns_but_startup_continues`（注入必失败的 `ChmodFn`，断言 `open()` 仍成功返回连接）；实现侧 `tighten_with` 对 chmod 错误只 `tracing::warn!("failed to tighten state file permissions to owner-only (continuing startup)")`。`cargo test -p sebas-db` → passed=48 failed=0。

## 2. 不做遗留导入（核对项）

- [x] 2.1 确认三处遗留文件的值**不被导入**：`card_config` / `runtime_state` / providers 一律以库为唯一权威，库为空即取默认；验证：单测构造「文件有值 + 库空」，断言启动后库仍为空、行为取默认值（**不**出现导入标记）
  - 证据：`sebas-dispatch/src/state_store.rs:1054 legacy_files_are_not_imported_and_not_touched`——盘上写含值的 `state.json` / `providers.json`，装**空**库引擎，断言 `load() == PersistedState::default()`（值未进状态）且两个文件逐字节未变。`card_config` 侧由 `sebas-dispatch/tests/settings_handler_test.rs::settings_set_reports_unavailable_when_store_is_missing`（库不可用 → 「保存失败/不可用」，不落文件）+ `src/run.rs::load_card_config` 的库空 → TOML 引导值分支覆盖。`cargo test -p sebas-dispatch` → passed=332 failed=0。
  - 设计取舍：tasks.md 2.1/2.2 与 design.md 的 Risks/Migration Plan 相抵触（后者仍提一次性导入 `card_config`）——**以 tasks.md 为准：完全不做遗留导入**。
- [x] 2.2 确认没有引入任何导入标记键；验证：`grep -rn "imported\|_import" src/sebas_state/` 只剩既有 `defaults_imported`（`defaults.json` 那条与本次无关）
  - 证据：`grep -rn "imported\|_import" src/sebas_state/` 命中仅 `defaults_import.rs` / `mod.rs` 的 `defaults_import`（模块名 + `defaults_imported` 键）与 `defaults_import.rs:154 absent_file_completes_the_import_phase_without_values`——即 `defaults.json` 那条既有导入，与本次无关。未新增任何导入标记。

## 3. 退休 state.json 与 providers.json

- [x] 3.1 删 `provider` overlay 的写入路径与损坏隔离（`src/provider.rs`）；验证：`cargo test -p sebas` 全绿，且 `grep -rn "providers.json" src/` 只在注释或测试夹具中残留
  - 证据：`src/provider.rs` 删除 `overlay_path()`、`validate_legacy_overlay`、`backup_broken_overlay`、自愈块及 `PathBuf`/`SystemTime`/`UNIX_EPOCH` 导入；`build_form` 改为 `RouterConfig::parse` 种子 + `FileStore::load("(state store)", ID_FIELD, seed)`。测试替换为 `build_form_with_empty_store_uses_config_seed` / `build_form_merges_store_values_over_the_config_seed` / `legacy_providers_json_is_neither_read_nor_moved` / `unavailable_store_presents_empty_seed_and_rejects_writes`。`grep -rn "providers.json" src/` 仅剩注释（`spawn_env.rs:43`、`run.rs:141`、`provider.rs:7/324/800`）与测试夹具（`provider.rs:807/838`）。`cargo test`（根 crate）→ passed=690 failed=0 ignored=88。
- [x] 3.2 删 `state_store` 的文件回退（`load_at` / `save_at` 的调用路径改为仅库），保留 legacy v0/v1 的**读入拒绝**语义；验证：`cargo test -p sebas-dispatch` 全绿，且「库不可用」时呈现 unavailable 而非文件派生值
  - 证据：`sebas-dispatch/src/state_store.rs` 删除整块文件回退内部实现（`RuntimeSide`/`load_at`/`save_at`/`load_runtime_side`/`providers_raw_present`/`load_overlay_sections`/`RuntimeWire`/`OverlayWire`/`LegacyState`/`parse_legacy_state`/`write_runtime`/`save_overlay`/`write_json_atomic`/`state_path()`/`providers_path()`）；`load()` 走 engine，缺失时 `PersistedState::default()` + warn；`save()`/`update()` 返回 typed error。新增 `pub fn unavailable_cause()`、`retired_file_env_vars_present()`。单测 `legacy_files_are_not_imported_and_not_touched`、`retired_file_env_vars_have_no_effect`、`unavailable_store_reads_default_and_rejects_writes`、`engine_round_trips_and_repairs_on_load`。`cargo test -p sebas-dispatch` → passed=332 failed=0。
  - 副作用（必要）：文件回退是原先唯一的「每测试隔离」机制；退休后新增 `sebas-dispatch/src/test_engine.rs`（`install_fresh`/`install_fresh_with`/`install_none`，可重入全局串行锁 + `Box::leak` 换引擎），`engine()` 签名保持不变（零生产调用方改动）。
- [x] 3.3 删 `src/run.rs` 的 `state.json` / `providers.json` 启动读取回退；验证：删除两个文件后启动行为不变（库为权威）
  - 证据：`src/run.rs:136-151` 改为退休变量提示（经 `retired_file_env_vars_present()` + `RETIRED_STATE_FILE_VAR`/`RETIRED_PROVIDER_OVERLAY_VAR`），不再有文件读取分支；`src/run.rs:152` 起为 `sebas_db::conn::secure_directory(dir)`。删除两文件后启动行为不变由 `tests/testsuite_e2e_test.rs::core_owned_provider_reaches_router_without_restart`（沙箱内三文件从不被创建/读取仍全绿）+ 6.3 沙箱验证（三文件逐字节未变、流程照跑）覆盖。`invoke testsuite-e2e` → 74 passed 0 failed。
- [x] 3.4 退休 `SEBAS_STATE_FILE` 与 `SEBAS_ROUTER_PROVIDER_OVERLAY`；验证：设置这两个变量后启动，路径解析与不设置时**完全一致**（单测覆盖）
  - 证据：两个变量**删除而非留作 no-op**（design D5：留成惰性比删掉更危险）。`state_store.rs` 新增 `RETIRED_STATE_FILE_VAR`/`RETIRED_PROVIDER_OVERLAY_VAR` + `retired_file_env_vars_present()`；单测 `state_store.rs:1081 retired_file_env_vars_have_no_effect`（把变量指向含值文件，`load()` 仍取库）。`tests/spawn_env_store_authority_test.rs` 的 `write_conflicting_file` 现在**同时**导出这两个变量并断言库仍胜出、文件逐字节未变（3 passed）。`cargo test -p sebas-dispatch` → passed=332 failed=0；`cargo test`（根）→ 690 passed。
- [x] 3.5 删 router 的 overlay 文件读取（`sebas-router/src/config.rs:751,773`）与文件监视（`hot_reload.rs`）；验证：`cargo test -p sebas-router` 全绿，且 provider 变更经 channel 通知仍热生效
  - 证据：`config.rs` 删除 `provider_overlay` 字段 / `default_provider_overlay` / `merge_provider_overlay` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 分支，改为 warn-only `warn_deprecated_provider_overlay_key`（`[router]` 无 `deny_unknown_fields`，故不报错、只告警；单测 `config.rs:2006 retired_provider_overlay_key_is_warned_and_ignored`）；5 个 overlay 测试重写为 `projection_*` 驱动 `apply_overlay_value`。`hot_reload.rs` 缩到只剩 `ReloadStatus`（watcher/notify/polling/`mark_admin_write`/`is_admin_write` 全删，`notify` 依赖从 `Cargo.toml` + `Cargo.lock` 移除）；`admin.rs` 的 `overlay_path()` 与簿记删除；`server.rs` 两处 `spawn_watcher` 删除，新增 `server.rs:341 channel_projection_applies_provider_changes_without_restart`。`cargo test -p sebas-router` → passed=296 failed=0。
  - **顺带修出的真实缺陷（本 task 验证所必需）**：channel 成为唯一热重载路径后，`admin::rebuild_from_seed` 每次 reload 都要重读 `config.toml` 取 `[provider.*]` 种子，而 `config_source` 只能从 `SEBAS_ROUTER_CONFIG` env 认出（缺省 `~/.sebas/config.toml`）——独立 router 进程不设该 env 时，第一次投影后第二次 reload 就会去读操作员的真实配置（或直接失败），provider 变更静默不生效。两处修复：`src/router_cmd.rs` 把实际 `-c` 路径记进 `cfg.config_source`；`sebas-router/src/admin.rs::rebuild_from_seed` 回填 `core.cfg.config_source`（与 listen/usage_db 等同款「启动期字段沿用」），单测 `admin.rs rebuild_from_seed_keeps_the_live_config_source`。
- [x] 3.6 从 `cli-service`「Config precedence and environment variables」描述的 override 集合中移除 `SEBAS_ROUTER_PROVIDER_OVERLAY`（该要求已在本 change 的 delta 中更新）；验证：该 spec 文本不再把它列为生效变量，且实现里无读取点
  - 证据：实现侧无读取点——`grep -rn "SEBAS_ROUTER_PROVIDER_OVERLAY" src/ sebas-*/src/` 仅剩 `state_store.rs` 的退休常量定义与 `run.rs` 的退休提示文案；`sebas-webui/src/api.rs` 的 `ENV_VAR_SPECS` 已删该条与 `SEBAS_STATE_FILE`（并同步 `env_endpoint_tests` 的钉名单断言）；`sebas-webui/frontend/src/views/settings-modal.test.ts` 夹具行换成 `SEBAS_STATE_DIR`/`SEBAS_HANG_TIMEOUT_SECS`。delta spec `openspec/changes/retire-legacy-state-json/specs/cli-service/spec.md` 的 MODIFIED 要求已移除该变量并含「retired state-file variables are not honored」场景。`cargo test -p sebas-webui` → 全绿；`npx vitest run src/views/settings-modal.test.ts` → 84 passed。

## 4. settings.json 退休

- [x] 4.1 core 读 card 设置改为只读库（删 `src/run.rs` 的文件回退分支）；验证：删除 `settings.json` 后卡面渲染配置取库中默认值（库为空时即默认主题）
  - 证据：`src/run.rs:744` 起 `fallback_settings(cfg)` 改为**纯** TOML `[card]` 引导值（调用方已先 `.await` 读库）；`src/run.rs:243-276` 三分支：库有值 → 用之；`Ok(None)`（库空）→ TOML 引导；`Err` → warn + TOML 引导；库不可用 → warn `unavailable_cause()` + TOML 引导。**绝不**用 `CardConfig::default()`。`cargo test`（根）→ 690 passed 0 failed。
- [x] 4.2 standalone webui 与 im 改为经 state 方法取 card 设置；验证：两个进程的既有用例全绿，且不可达时呈现 unavailable 状态
  - 证据：`src/webui_cmd.rs::load_card_config` 改 async，经 `core_channel::client::snapshot_domain_once(&socket_path(cfg), &secret, "settings")`；`src/im_cmd.rs::load_card_config` 同款（再经 serde 往返 `sebas_dispatch::CardConfig` → feishu 镜像）；`sebas-webui/src/server.rs` 的 `card_config` 文档改写。`cargo test -p sebas-webui` 全绿；`cargo test`（根，含 `webui_cmd`/`im_cmd` 面）→ 690 passed 0 failed。
- [x] 4.3 删 `sebas-dispatch/src/settings.rs` 的文件读写；验证：`grep -rn "settings.json" src/ sebas-*/src/` 无生产代码命中
  - 证据：删除 `sebas-dispatch/src/settings.rs`（62 行：`settings_path`/`load_settings`/`save_settings` + chmod 0600）与 `sebas-dispatch/tests/settings_test.rs`；`lib.rs` 去掉 `pub mod settings;`。`engine/inbound.rs::handle_settings` 去掉 `path` 参数，改经 `state_store::engine()` → `engine.save_settings(...)`，不可用/序列化/引擎错误都回诚实「保存失败: ...」。`grep -rn "settings.json" src/ sebas-*/src/` 仅剩注释（无生产代码命中）；`sebas-dispatch/tests/settings_handler_test.rs` 重写为 4 个不写文件的用例。`cargo test -p sebas-dispatch` → passed=332 failed=0。

## 5. 测试、脚本与文档

- [x] 5.1 改 `tests/state_persistence_test.rs` / `tests/spawn_env_store_authority_test.rs`：不再裸写这些文件驱动状态；验证：两个测试文件全绿，且其中不再有 `std::fs::write` 到 legacy 路径
  - 证据：`tests/state_persistence_test.rs`：`OVERLAY_LOCK` → `STATE_DIR_LOCK`，两个 legacy 用例改为钉 `SEBAS_STATE_DIR`（不再写 `providers.json` 做重定向），`defaults.json` 导入/一次性断言保留。`tests/spawn_env_store_authority_test.rs`：`write_conflicting_file` 仍写一份遗留 `providers.json` **但只为断言它被忽略**（同时导出两个退休变量），`store_beats_conflicting_legacy_file` 追加「文件不得被搬走」断言。两个目标全绿：`spawn_env_store_authority_test` 3 passed、`state_persistence_test` 5 passed；`cargo test`（根）→ 690 passed 0 failed。
- [x] 5.2 改 `scripts/e2e_gateway_admin.sh` 的「外部改写 providers.json 测热更新」用例（改为经 channel 通知，或删除该用例并说明理由）；验证：脚本可跑且断言与新的权威来源一致
  - 证据：脚本头部标注为**已死**（`sebas gateway` 子命令不存在，脚本无法端到端执行），热更新用例删除并留指针注释（指向 `sebas-router/src/server.rs::tests::channel_projection_applies_provider_changes_without_restart` 与 `tests/testsuite_e2e_test.rs::core_owned_provider_reaches_router_without_restart`）；`"providers":3` → `"providers":2`；步骤回显重编号 `[1/5]…[5/5]`。验证方式诚实降级：`bash -n` 通过 + 静态一致性核对（脚本本身不可运行）。
- [x] 5.3 保留并复核 `tests/testsuite_e2e_test.rs` 的「providers.json 从不被创建」断言；验证：断言通过，且新增对 `state.json` / `settings.json` 同款缺席断言
  - 证据：`tests/testsuite_e2e_test.rs:2870-2884`（`core_owned_provider_reaches_router_without_restart` 内）原 `providers.json` 缺席断言保留，新增三文件循环缺席断言（`state.json` / `settings.json` / `providers.json`）。`invoke testsuite-e2e` → 74 passed 0 failed，其中 `router_and_provider::core_owned_provider_reaches_router_without_restart ... ok`。
- [x] 5.4 更新 `tasks.py` 的沙箱 env 集合与 `AGENTS.md` 的沙箱菜谱（移除已退休的两个变量与对应「必钉」说明）；验证：沙箱仍能起、且 `AGENTS.md` 不再声称这两个变量会生效
  - 证据：`tasks.py::_sandbox_env` 与 `smoke_real` 去掉两个退休变量条目；`AGENTS.md` 6 处外科式修改（24 行 diff）——沙箱菜谱不再钉这两个变量、不再声称其生效，`[router] provider_overlay` 键标注退休，运行命令去掉两变量。验证：`grep -n "SEBAS_STATE_FILE\|SEBAS_ROUTER_PROVIDER_OVERLAY\|not retired yet" AGENTS.md` 无命中；沙箱仍能起由 6.1（74 passed）与 6.3 沙箱实测（core + router 起、流程跑通）证明。
  - 另同步清理同类夹具：`tests/support/mod.rs`（去掉两个退休 env 钉 + 死键 `provider_overlay`，注释改写）、`tests/real_agent_e2e_test.rs`（同款）、`scripts/verify-legacy-63.sh`（新增，6.3 用）。
- [x] 5.5 复核并更新 `openspec/specs/session-persistence/spec.md` 的 Purpose（它还声称拥有 `state.json` / `providers.json` 的磁盘布局）；验证：Purpose 与 `specs/session-persistence` 的新增要求一致
  - 证据：`openspec/specs/session-persistence/spec.md` Purpose 第 4 行改写，不再声称拥有 `state.json` / `providers.json` 磁盘布局；与 delta `specs/session-persistence/spec.md` 的新增要求一致。

## 6. 全量回归

- [x] 6.1 跑 `invoke testsuite-e2e`；验证：全绿
  - 证据：`invoke testsuite-e2e` → `✅ e2e — 74 passed · 42.1s [cargo]`（0 failed）。首轮曾在高负载下出现 7 个 timing/超时类假红（`claude_delta_gap_*` 388ms 间隔、`two_sessions_spawn_and_turn_concurrently`、`pending_and_queue.*` 4 例、`test_model_tool_loop_*`）；按既有约定单独复跑 + 干净全量复跑，干净run 全绿，以干净 run 为准。前置：`cargo build` 不构建依赖 crate 的 bin，需先 `cargo build -p sebas-node -p sebas-acp`（否则 `acp_session_mapping_test` 4 例因缺 `fake-acp-agent` 假红——已实证并补建）。
- [x] 6.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
  - 证据：首轮 `✅ acceptance — 9 passed, 1 failed`，红在 `model_and_provider.provider_governance_journey`——该旅程靠写 `providers.json` overlay 驱动，overlay 退休后必然红（正是「回到对应步定位」要抓的）。改写为经 webui BFF 把 provider + model alias 写进**状态库**、router 经 core 通道热生效（并显式注入 watchdog 同款 `SEBAS_CORE_SOCKET`），顺带修出 3.5 记录的 `config_source` 两跳缺陷。终态：`invoke testsuite-acceptance` → `✅ acceptance — 10 passed · 42.7s [cargo]`（0 failed）。
- [x] 6.3 遗留文件不被读取也不被改动：在含三个遗留文件的机器（沙箱模拟）上启动并跑完整流程；验证：三个文件逐字节未变，且库中不出现它们的值（无导入）
  - 证据：新增 `scripts/verify-legacy-63.sh`，在一次性 `/tmp/sebas-legacy-63-*` 沙箱里预置三个各带 `POISON_*` 毒值的遗留文件，起 core（含 webui）+ standalone router，经 webui BFF 写 provider + alias，等 router 经 core 通道热生效，然后断言：三个文件 sha256 前后一致、状态库（`settings.db`/`projects.db`）无 `POISON` 字节、三文件仍存在。实跑输出：`PASS: 6.3 遗留文件既不被读取、也不被改动；provider/alias 走状态库权威`（alias 第 2 次尝试即命中，上游收到 `stub-model`）。沙箱绝不触碰操作员真实实例（全部路径在一次性目录、端口 19771/18771/18772，未设 `SEBAS_CORE_SECRET`）。

---

## 状态备注

### 已完成
32 个 task 全部完成并逐条留证（见上）。核心改动：`sebas-db` 库文件权限收紧（1.x）；`sebas-dispatch` 删文件回退 + 退休变量（2.x/3.2/3.4）；`src/provider.rs` 删 overlay 写入与自愈（3.1）；`src/run.rs` 去文件回退（3.3）；`sebas-router` 删 overlay 读取与文件监视、channel 成为唯一热重载路径（3.5）；`settings.json` 全链退休（4.x）；测试/脚本/文档同步（5.x）；三段全量回归（6.x）。

### 未完成 / 未验证（诚实清单）
- `scripts/e2e_gateway_admin.sh` **无法端到端执行**（`sebas gateway` 子命令不存在，脚本此前已死）。5.2 的验证只做到 `bash -n` + 静态一致性核对，未实跑断言。
- 真实凭据下的 ACP 回合未验证（沙箱零真实凭据，`tests/real_agent_e2e_test.rs` 两条 `--ignored` 未跑，`native` 执行体在沙箱恒 `ok: false`）。与本次改动无关，但按约定如实标注。
- `invoke smoke-real` 未跑（operator 手跑入口，agent 禁触）。

### 与 design.md 的偏差
1. **不做任何遗留导入**：design.md 的 Risks/Migration Plan 仍提一次性导入 `card_config`；tasks.md 2.1/2.2 与之相抵触，按 tasks.md 执行——完全不做遗留导入，库为空即取默认。
2. **退休变量删除而非留 no-op**：依 design D5，`SEBAS_STATE_FILE` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 直接删除语义（保留常量仅为告警文案），留成惰性被认为更危险。
3. **目录权限 0700 而非 0600**，且由 `secure_directory()` 在状态目录所有者处显式调用（通用 `open()` 不得 chmod 共享父目录如 `/tmp`）。
4. **router `provider_overlay` 配置键 warn-only 而非报错**：`[router]` 没有 `deny_unknown_fields`，为不破坏既有测试配置，采用原始 TOML 扫描 + 告警（与既有 `routes` 键同款处理）。
5. **超出 task 字面的必要修复**：`src/router_cmd.rs` 记录 `-c` 路径进 `config_source` + `admin::rebuild_from_seed` 回填 `config_source`（3.5 的验证前提，见 3.5 证据）；`sebas-dispatch/src/test_engine.rs` 新增（文件回退是原唯一每测试隔离机制，退休后必须有替代）；`sebas-dispatch/Cargo.toml` 移除不再使用的 `dirs` 依赖。

### 影响面 / 涟漪
- 删除文件回退迫使大量 crate 内与集成测试改造：`provider_card.rs`（约 24 处调用点）、`crud.rs`、`provider_test.rs`、`spawn_env.rs`（约 24 处 `write_overlay`）、`state_persistence_test.rs`、`spawn_env_store_authority_test.rs`。
- 所有触达状态库的 `#[tokio::test]` 必须改 `flavor = "multi_thread"`（`block_on_engine` 用 `block_in_place`）；同时给 `block_on_engine` 补了「无运行时」分支（临时 current-thread runtime），保住「`load()` 是任意处可用的同步函数」契约。
- 全局引擎槽改为可换（`OnceLock<RwLock<Option<&'static dyn ...>>>` + `Box::leak`），使 `engine()` 签名零变化（生产调用方零改动）而测试获得 `test_engine::install_fresh()` 隔离；测试锁做成**可重入**（同线程二次获取不阻塞、不提前释放），避免夹具在同一作用域连续装引擎时自锁死。
- 排查期间修掉两个自己引入的死锁：`spawn_env.rs::end_to_end_mode_setting_flows_through_to_spawn_env` 同作用域连续 `install_fresh_with` 造成自锁；`state_store.rs::retired_file_env_vars_have_no_effect` 的「引擎锁 → env 锁」与 `crud.rs` 的「env 锁 → 引擎锁」构成跨线程锁序反转（已统一为 env 锁在外）。

### 残留风险 / 后续
- `tests/testsuite-webui/tests/helpers/detached.ts:65-66` 与 `openspec/specs/{testsuite-process-e2e,webui}/spec.md` 仍提到退休变量；变量已惰性，无害，但属本次范围外的文档债。
- `src/provider.rs` 多处仍向 `FileStore::load` 传 `dir.path().join("providers.json")` 作为**已被忽略**的路径参数（历史残留，无语义）。
- `sebas-router/src/config.rs` 的 `provider_overlay` 键目前只 warn；后续若要收口，应在 `[router]` 上加 `deny_unknown_fields` 并按协议演进三规则处理。
- `sebas-webui/src/api.rs` 的 env 清单与前端夹具已同步，但若还有其它面（文档/截图）列举这两个变量，需一并清理。
