## 1. 删除 core 启停开关

- [x] 1.1 `src/config.rs`：删除 `WatchdogCoreConfig.enabled` 字段（保留 `channel_path` / `secret_file`），并删除/更新引用它的断言（`webui_enabled_by_default_and_core_disabled` 改为断言 webui 默认启用、router 默认停用）。验证：`cargo test --lib config` 全绿。
- [x] 1.2 `src/watchdog.rs`：core spec 恒 `DesiredState::Enabled`，不再读 `config.core.enabled`；注册处注释更新为「core 恒启动」，改用 `services.register_core(core_spec)`。验证：`cargo build` 通过。
- [x] 1.3 `src/watchdog/services.rs`：新增 `register_core`——core entry 忽略 config 与 services.json 覆盖层（读出 `core` 覆盖时 `warn!` deprecation），期望态恒 `Enabled`。验证：新增 `register_core_ignores_persisted_off_override` 通过。
- [x] 1.4 配置兼容：旧 `[watchdog.core] enabled` 键解析不报错（serde 忽略删除键），启动时经 `warn_deprecated_watchdog_keys` 打 deprecation `warn!`。验证：新增 `deprecated_core_enabled_key_is_ignored` 通过。

## 2. fail-fast 覆盖核对（core 场景）

- [x] 2.1 核对既有通用 fail-fast 单测，并补齐 core 专属断言 `core_spawn_failure_hits_limit_and_enters_failed_startup`（core spawn 连败 → failed-startup → 上报 core 事件）。验证：`cargo test --lib watchdog` 全绿。

## 3. 前端：core 行去启停按钮

- [x] 3.1 webui 前端 Services 分区 `renderServiceRow`：core 行仅呈现状态与 restart，不渲染 enable/disable。验证：`pnpm test` 21 项全绿（新增「core row offers only restart」用例）；`pnpm run build` 通过。

## 4. 测试/沙箱注入移除与端到端验证

- [x] 4.1 删除 `tests/support/mod.rs::enable_supervised_core` 注入 `[watchdog.core] enabled = true` 的逻辑（保留 `[storage] data_dir` pin）。验证：`cargo build --tests` 通过；`watchdog_supervised_core_recovery` 在无注入下通过。
- [x] 4.2 `tasks.py` 沙箱配置模板无 `[watchdog.core] enabled` 注入（本就只含 `channel_path`），无需改动。
- [x] 4.3 e2e/验收用例：`cargo test --test testsuite_e2e_test -- --ignored`（11 项）与 `--test testsuite_acceptance_test -- --ignored`（6 项）全绿；核实无「无 core 起 webui」依赖（各用例显式 spawn 进程，非依赖 core 缺省缺席）。
- [x] 4.4 沙箱验证默认面：无 `[watchdog.core] enabled` 时 `sebas run` 拉起 core + webui，`/api/admin/services` 报告 core running / desired enabled、router disabled、无 im 行，`/api/summary` reachability.ok == true。
- [x] 4.5 沙箱验证 core 启动失败面：core 解析失败（垃圾 DB）→ ready 前退出 75 → 监督器标记 **Degraded**（不重试、watchdog 存活）、stderr 打 `startup-failure:` 摘要。**注**：`failed-startup` 终态化 + watchdog exit 75 仅在 spawn 级失败 / 非 75 早退时触发（代码路径），无法由配置构造，由 2.1 的 supervisor 单测（通用 + core 专属）覆盖。
- [x] 4.6 全量回归：`cargo test` 389 passed / 24 ignored（ignored = 已单独 `--ignored` 跑过的进程级套件）。

## 5. 文档同步

- [x] 5.1 `docs/architecture/process-ipc-subcommands.md` 默认值表更新（core 行：无开关、恒启动；三层合成仅适用可开关服务）；`config/config.toml.example` 模板移除 `enabled = true`、改注 `channel_path` / `secret_file`。验证：表格与代码一致。
