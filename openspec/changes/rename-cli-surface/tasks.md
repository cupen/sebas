## 1. 会话分发让位（router→dispatch，必须最先做）

- [x] 1.1 `git mv sebas-router sebas-dispatch`；改目录 Cargo.toml `name = "sebas-dispatch"` + workspace members
- [x] 1.2 全仓 `sebas_router::`→`sebas_dispatch::`、`use sebas_router`（src/ 18 文件）；`RouterHandle`→`DispatchHandle`
- [x] 1.3 配置节 `[router]`→`[dispatch]`（state_file），旧 `[router]` 键解析告警兼容
- [x] 1.4 `src/native_router_bridge.rs`→`src/native_dispatch_bridge.rs` 及内部 `NativeSessionBridge` 提法同步
- [x] 1.5 replay-debug / webui spec 提法的代码侧类型名（`RouterHandle` 字符串）随迁

## 2. 模型路由入位（gateway→router）

- [x] 2.1 `git mv sebas-gateway sebas-router`；改 Cargo name + workspace members；`sebas_gateway::`→`sebas_router::`（7 文件）
- [x] 2.2 `gateway_cmd.rs`→`router_cmd.rs`；`GatewayArgs`→`RouterArgs`；`Cmd::Gateway`→`Cmd::Router` + `#[command(alias = "gateway")]`
- [x] 2.3 `--gateway` flag→`--router`（run.rs 内嵌形态 + GatewayConfig→RouterConfig + `GatewaySpawner`→`RouterSpawner` + spawn_aux argv）
- [x] 2.4 配置节 `[gateway]`→`[router]`（provider_overlay/usage_file）+ 旧键告警；env `SEBAS_GATEWAY_PROVIDER_OVERLAY`→`SEBAS_ROUTER_PROVIDER_OVERLAY`（新名优先、旧名回退+告警）；`SEBAS_AGENT_GATEWAY_URL/AUTH`→`SEBAS_AGENT_ROUTER_URL/AUTH`（同策略）
- [x] 2.5 `ProviderMode::Gateway`→`Router`（serde `rename = "router", alias = "gateway"`）；`GatewayAction`→`RouterAction`（含 feishu `/gateway` 命令词→`/router`，解析器保留 `/gateway` 别名）
- [x] 2.6 控制面服务名 `"gateway"`→`"router"`（watchdog/services.rs、executor.rs、control_rpc.rs + 断言）
- [x] 2.7 webui 后端 `/api/gateway`→`/api/router`、`/gateway/api/*`→`/router/api/*`、`GatewayInfo`→`RouterInfo`、`gateway_listen`→`router_listen`；前端 client.ts、settings 卡片、app-shell.test.ts fixtures（retired SPA 路由 `/gateway` 保留）
- [x] 2.8 log 行 "gateway started (run --gateway)" → "router started (core --router)"（run.rs）

## 3. core/run 对调

- [x] 3.1 cli.rs：`Cmd::Run(RunArgs)`→`Cmd::Core(CoreArgs)`；`Cmd::Watchdog(WatchdogArgs)`→`Cmd::Run(RunArgs)` + `#[command(alias = "watchdog")]`；help 文案改写
- [x] 3.2 lib.rs：`CORE_SUBCOMMAND="core"`、新增 `RUN_SUBCOMMAND="run"`、`ROUTER_SUBCOMMAND="router"`；watchdog.rs core spawn 改用常量（去硬编码）；service.rs ExecStart 用 `RUN_SUBCOMMAND`；log 行 `启动 sebas core 子进程: … core --config`
- [x] 3.3 service.rs:193 错误提示 "use `sebas run` directly"→`sebas core`；main.rs error hints `sebas watchdog`→`sebas run`
- [x] 3.4 main.rs parse 测试同步（run→core）+ 新增 `watchdog`/`gateway` 别名用例；service.rs unit 渲染断言（watchdog→run）与测试名 `unit_runs_watchdog_not_core` 更新

## 4. 测试与脚本

- [x] 4.1 tests/support/mod.rs：spawn argv `run`→`core`、`--gateway`→`--router`、配置模板 `[gateway]`→`[router]`/`[router]`→`[dispatch]`、usage_file、SEBAS_* env
- [x] 4.2 tests/sigterm_cleanup_test.rs spawn argv；tests/acceptance/COVERAGE.md 提法
- [x] 4.3 scripts/test_watchdog_debug_upgrade.sh：`watchdog --debug`→`run --debug`、gateway spawn→router、`pgrep -f 'run --config'`→`'core --config'`
- [x] 4.4 scripts/test_webui_sandbox.sh 扫 `[gateway]`/env 残留
- [x] 4.5 Dockerfile `CMD ["run",…]`→`["core",…]` + 注释

## 5. 文档

- [x] 5.1 README（quickstart/图/docker 示例）、docs/architecture.md 角色表、docs/frontend-dev.md
- [x] 5.2 AGENTS.md：沙箱菜谱 `sebas run`→`sebas core`、`--gateway`→`--router`、"gateway started" log 锚点、SEBAS_GATEWAY_PROVIDER_OVERLAY→SEBAS_ROUTER_PROVIDER_OVERLAY、/v1/messages 菜谱提法
- [x] 5.3 glossary.md：core/run/router/webui/dispatch 条目更新 + 三义消解（CLI router=模型路由、sebas-dispatch=会话分发、前端 router.ts=URL 路由）
- [x] 5.4 活跃 change 文案：wire-webui proposal/design `run --webui`→`core --webui`；add-ansible-deploy design.md ExecStart 措辞对齐

## 6. 验证与收尾

- [x] 6.1 `cargo build` + `cargo test` 全绿
- [x] 6.2 `invoke e2e` 全套通过
- [x] 6.3 `bash scripts/test_watchdog_debug_upgrade.sh` 通过
- [x] 6.4 冒烟：`sebas watchdog -c` 别名启动；config 旧键告警；debug router POST /v1/messages model=test → 200
- [x] 6.5 清零扫描：`sebas_gateway|GatewayConfig|GatewayAction|SEBAS_GATEWAY|sebas_router\b|RouterHandle`（排除别名定义/archive/历史 log）归零
- [x] 6.6 `openspec validate --all` 全绿；归档时手工 `git rm` 清空的 6 个旧 spec 目录
