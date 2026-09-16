## 1. router.routes 作废（sebas-router）

- [x] 1.1 config.rs：删 `RawRouterConfig.routes` / `RouterConfig.routes` / `RouteGroup` / routes 编译与校验；raw `[router]` 表扫描 `routes` 键 → eprintln + tracing 双通道 warn；验证 `cargo check -p sebas-router` 通过
- [x] 1.2 routing.rs / admin.rs：`RouteTable` 不再装载 config routes（删 exact/glob 匹配层，保留 namespace → alias → default 兜底），`/admin/stats` 删 `routes` 计数；验证路由单测通过
- [x] 1.3 测试与配置面：contract/rate_limit/debug_provider/process_e2e 四个测试文件与 config.rs 内联测试去 routes 化；`config/config.toml`、`config.toml.example`、`README.md:209` 删 routes；验证 `cargo test -p sebas-router` 全绿

## 2. watchdog 三节更名 service.*（根配置）

- [x] 2.1 config.rs：`Config` 拆出 `service: ServiceConfig { core, webui, router }`（`ServiceCoreConfig`/`ServiceWebUiConfig`/`ServiceRouterConfig` 更名），`WatchdogConfig` 瘦身为 `{ im, upgrade, storage, max_spawn_failures }`；`warn_deprecated_watchdog_keys` 扩展扫描 `[watchdog.core|webui|router]` 表 → 双通道 warn；验证 `cargo check -p sebas` 通过
- [x] 2.2 访问链与文案：`cfg.watchdog.{core,webui,router}` → `cfg.service.*`（webui_cmd/run/watchdog/services/executor/core_channel/im_cmd/node_link_cmd），报错与帮助文案引用同步改；im/upgrade/storage 路径不动；验证 `cargo test -p sebas` config 相关全绿
- [x] 2.3 配套面：`config/config.toml` + `config.toml.example`、`README.md`、`AGENTS.md`、`tasks.py`、`docs/architecture/process-ipc-subcommands.md`、`tests/support/mod.rs`、`tests/real_agent_e2e_test.rs`、`scripts/test_watchdog_debug_upgrade.sh`、sebas-webui/src 三处注释同步新键

## 3. OpenSpec 与验证

- [x] 3.1 specs delta 已建（router-core / watchdog / webui / feishu-option / testsuite-process-e2e 五能力）；验证 `openspec validate --changes simplify-service-config` 通过
- [x] 3.2 全量回归：`cargo build`（隔离 target）+ `cargo test -p sebas` + `-p sebas-router` + `-p sebas-webui`；新旧配置沙箱冒烟（新键生效、旧键 warn 忽略、routes 键 warn 忽略）
