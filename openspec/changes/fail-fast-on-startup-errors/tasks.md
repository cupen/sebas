# tasks — fail-fast-on-startup-errors

## 1. 启动失败摘要 + 退出码 75

- [x] 1.1 在 `src/cli.rs` 或各子命令入口（`core`、`webui`、`router`、`run`、`update`）加入 startup-failure 检测：在 `tokio::main` 入口包一层 try-catch / 顶层 Result；任何未达 ready 的 fatal 都走统一退出路径；运行 `cargo test --workspace` 验证既有单元（验证：现有 happy-path 单测不破坏；fatal 路径触发 75）——落地为 `src/startup_failure.rs` + `main.rs::startup_failure_exit`，core/webui/router/run/update/im 六个子命令 Err 一律走统一出口（这些命令 ready 后常驻到信号，返回 Err 即启动失败）；`cargo test --workspace` 全绿（227+ lib 用例）。
- [x] 1.2 在 startup-failure 退出路径中：写入 stderr 末行 `startup-failure: <可读原因>`；当 `SEBAS_STARTUP_ERROR_FILE` 环境变量设置时同步写入该文件（覆盖写）；运行 `cargo test --workspace` 验证两个 sink 都被命中（验证：临时设 env 后启动失败，文件与 stderr 末行一致）——`src/startup_failure.rs` 单测 6 项（前缀、覆盖写、env 未设 no-op、读取、闩锁清除）；进程级断言（stderr 末行 + 文件内容一致）由任务 4.2 的 e2e 承担。
- [x] 1.3 把当前零散的"启动失败"返回路径（配置错误、port bind、state DB 不可写、control secret 缺失）统一走 startup-failure 退出；不在这些路径里调 `process::exit(1)`；运行 `cargo test` 验证（验证：错误注入测试触发各路径均得到 75）——config 解析（main.rs）、webui bind（`webui_cmd.rs` 统一走 `exit_startup_failure`，仍 75=Degraded 语义）、state DB 不可写（`run.rs` 改为 fatal，不再静默退回文件存储）、standalone webui 缺 `SEBAS_CORE_SECRET`（`webui_cmd.rs` fatal）；`run.rs` 优雅关闭 dump 失败改为 warn 不外抛，避免运行期失败冒充启动失败。

## 2. watchdog 终态化

- [ ] 2.1 在 `src/watchdog/supervisor.rs` 给每个受管 service 的 spawn task 接入 `[watchdog] max_spawn_failures`（默认 3）：连续失败达到 N 时，service 状态置 `failed-startup`、watchdog 进入 shutdown 路径；窗口内每次失败写结构化日志（含 stderr 摘要）。运行 `cargo test --workspace`（验证：现有 supervisor 单测覆盖 1/3/10 三档；fail 路径 75 退出）
- [ ] 2.2 在 `src/watchdog/updater.rs` 把 "rollback failure tolerated: continues supervision loop" 改为「rollback 失败时 watchdog 进入 `failed-startup`、退出 75、写 startup-failure 摘要」。运行 `cargo test --workspace`（验证：现有 rollback 单测改写后通过；无 backup 路径触发 75）
- [ ] 2.3 在 `src/run.rs` 让 watchdog 整体以 75 退出时打 startup-failure 摘要；systemd unit 看到 75 不应无限快速重启；运行 `invoke testsuite-e2e --case startup_failure_core` 验证（验证：沙箱内 `target/debug/sebas core -c /tmp/garbage.toml` 收到 75；`target/debug/sebas run -c /tmp/garbage.toml --router --debug` 启动后内部 core 连续 3 次 spawn fail 触发 watchdog 退出 75）
- [ ] 2.4 扩展 `sebas ctl status` 输出 `startup_failure: { service, count, last_stderr, at }` 字段（无失败时缺省空对象）；同步在 `GET /api/summary` 的 `reachability.cause` 在 core 启动失败时携带摘要。运行 `cargo test --workspace` 验证（验证：单测覆盖 fail 字段存在/缺省两种情况；CLI 输出向后兼容）

## 3. webui web_spawn inline

- [ ] 3.1 在 `sebas-webui/src/session_backend.rs` 改写 `web_spawn`：spawn 失败立即通过 dispatch 推到 transcript 作为错误事件（而非延后到 Removed）。同步删除 `sebas-webui/src/session_backend.rs:412` 的 "web_spawn never fails structurally" 注释。运行 `pnpm --dir sebas-webui/frontend test` 验证（验证：session-backend 单测覆盖 spawn 失败 → transcript 出现 error 事件）
- [ ] 3.2 在 `sebas-dispatch/src/dispatch.rs` 把 `web_spawn: acp_spawn_and_activate failed` 的 `warn!` 改成把错误推到 dispatch 事件流（与 3.1 同步）；不再 swallow。运行 `cargo test --workspace` 验证（验证：dispatch 单测覆盖错误传播；happy-path 不变）
- [ ] 3.3 在 `sebas-webui/frontend/src/views/transcript-view.ts` 渲染 spawn-failed 错误事件为 transcript 内的可读错误（与现有错误事件渲染一致）；同一会话相邻 N 秒内的同类失败事件合并为一条带计数的错误。运行 `pnpm test -- transcript-view.test.ts` 验证（验证：合并/单独两种分支都覆盖）

## 4. 账本与端到端验收

- [ ] 4.1 在 `tests/testsuite-webui/tests/errors.spec.ts` 新增 "spawn failure inline" 旅程：触发器让 acp 子进程 spawn 失败、断言 transcript 内显出现错误事件、Removed 事件作为次要信号补充（仍按顺序呈现）。运行 `invoke testsuite-webui --case errors` 验证（验证：1 次全绿）
- [ ] 4.2 在 `tests/testsuite-e2e_test.rs`（或新增 `tests/startup_failure_test.rs`）新增进程级端到端用例：构造 `garbage.toml`、`SEBAS_STARTUP_ERROR_FILE=<tmp>` 启动 `sebas run`，断言 watchdog 退出 75、错误文件含 `startup-failure: ` 摘要。运行 `invoke testsuite-e2e --case startup_failure` 验证（验证：1 次全绿）
- [ ] 4.3 更新 `tests/acceptance/COVERAGE.md` 把本期 change 加进 `testsuite-process-e2e` 与 `testsuite-webui-browser` 索引；运行 `openspec validate --changes --strict` 验证（验证：delta 字段完整、scenario 全 SHALL/MUST）
- [ ] 4.4 跑 `invoke testsuite-webui` 全量 3 连绿 + `invoke testsuite-e2e` 1 次全绿（验证：与既有稳定性门槛一致；新增用例就位）

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change fail-fast-on-startup-errors --json` 验证四个 artifact 全部 `done`（验证：proposal/specs/design/tasks 状态均为 done；isPlanningComplete: true）
- [ ] 5.2 跑 `openspec validate --changes --strict` 无报错（验证：delta 字段完整、scenario 全 SHALL/MUST）
- [ ] 5.3 在 `tests/acceptance/COVERAGE.md` 段落末尾追加 `fail-fast-on-startup-errors` 一行指向本期 commit hash 与本 tasks（验证：账本自身可追溯）