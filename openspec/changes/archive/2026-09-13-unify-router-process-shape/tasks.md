# Tasks: unify-router-process-shape

## 1. 内嵌 router 退役

- [x] 1.1 `src/cli.rs` 删 `run` 的 `--router`/`--debug` 旗标；`src/run.rs`
  删内嵌 router 启动块（随机端口、`router started (core --router)` 日志、
  router_info 装配联动）；`cargo build` 通过、`sebas core --router` 报
  unknown-argument；受影响单测更新
- [x] 1.2 全仓清点 `--router` 残留引用（`tests/`、`tasks.py`、脚本），
  逐处迁移或删除；`cargo test --workspace` 相关用例绿

## 2. router 停止保护（watchdog + core）

- [x] 2.1 core：session channel snapshot 增 `router_activity` domain
  （活跃 routed 会话计数：provider mode = Router 且状态非终态）；
  `cargo test -p sebas -p sebas-dispatch` 覆盖计数语义（Running/Warming
  计入、Queue/Done/Archive 不计入）
- [x] 2.2 control wire：`ServiceSet` 增 `force: bool`（serde 默认 false，
  非 router-stop 组合忽略）；watchdog executor 停 router 前经 core
  channel 查计数，非零且未 force → `Rejected { code:
  "active_routed_sessions", count }`；force 或 core 不可达 → 放行；
  executor 单测覆盖三态（拒/force 过/不可达过）
- [x] 2.3 webui 后端 admin adapter 透传 force 与拒绝载荷（HTTP 语义：
  拒绝 → 400 + code + count）；`gateway_bff_test.rs`/api 测试补透传用例

## 3. 前端服务页强制出口

- [x] 3.1 `settings-modal.ts` services 分区：router 停止被拒（code
  `active_routed_sessions`）→ 二层对话框（计数 + 中断后果文案 +
  「强制停止」/取消），强制走 force 重发；取消不发请求；
  `settings-modal.test.ts` 补三态用例（直接成功/拒绝后强制/拒绝后取消）
- [x] 3.2 `pnpm --dir sebas-webui/frontend test` 全绿

## 4. 测试设施与文档

- [x] 4.1 `tasks.py` 浏览器套件装配改两进程（core + `sebas router
  --config <SB>/config.toml --debug`），清理 SIGTERM 两进程；
  `invoke testsuite-webui` 冒烟通过
- [x] 4.2 e2e `testsuite_e2e_test.rs` 装配同步两进程化；
  `invoke testsuite-e2e` 通过
- [x] 4.3 AGENTS.md 沙箱菜谱改两进程写法；`config/config.toml.example`
  router 段注释核对（默认关、服务页开关、独立进程说明）；
  `openspec validate unify-router-process-shape --strict` 通过
