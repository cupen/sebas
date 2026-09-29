## 1. 构建管道：BUILD_TIME 注入

- [x] 1.1 根 `build.rs` 注入 `cargo:rustc-env=BUILD_TIME`（UTC 编译时刻，chrono `%Y-%m-%d %H:%M`；chrono 加入根 `[build-dependencies]`）。验证：`cargo build -v` 后在 `target/debug/build/<root-pkg>-*/output` 中可见 `BUILD_TIME=YYYY-MM-DD HH:mm` 行。
- [x] 1.2 `src/upgrade.rs` 新增 `build_time()` 助手（`option_env!("BUILD_TIME")`，缺省 `"unknown"`，与 `current_version()` 同款惯例）+ 单测断言值匹配 `^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$` 或字面 `"unknown"`。验证：`cargo test upgrade` 过。

## 2. sebas-webui 侧 BuildInfo 通道

- [x] 2.1 sebas-webui 定义 `BuildInfo { version, build_time, git_branch, git_hash }`（含 `BuildInfo::unknown()` 兜底），`WebUiState` 增 `build` 字段；`run_full` 与 `run_with_admin_adapter_and_auth` 签名加 `build: BuildInfo` 参数，薄包装 `run` / `run_with_admin_adapter` 传 `unknown()`（design D1：sebas-webui 内 option_env! 读不到根注入值，真值只能走装配点）。验证：`cargo check -p sebas-webui` 过。
- [x] 2.2 about 处理器：`version` 改读 `state.build.version`，响应追加 `build_time` / `git_branch` / `git_hash` 三个字符串字段（只增不改）；跟随现有处理器测试模式补单测——known 与 `BuildInfo::unknown()` 两态各断言一次 JSON 字段（覆盖 spec「构建信息缺失时如实呈现 unknown」场景）。验证：`cargo test -p sebas-webui about` 过。

## 3. 装配点接线

- [x] 3.1 `src/webui_cmd.rs` 与 `src/run.rs` 两处装配点构造 `BuildInfo`（`version` = 根 `CARGO_PKG_VERSION`，`build_time` / git 字段来自 `upgrade.rs` 助手）传入 `run_with_admin_adapter_and_auth`。验证：`cargo check`（全 workspace）过，且 grep 确认两处装配点均已传参（漏一处表现为 About 显示 unknown）。

## 4. 前端呈现

- [x] 4.1 `frontend/src/api/client.ts` 的 `About` 接口追加 `build_time` / `git_branch` / `git_hash` 字段。验证：`pnpm run build`（frontend）类型检查无错。
- [x] 4.2 `frontend/src/views/settings-modal.ts` 的 `renderAbout`：BUILD 段 Version chip 之下新增构建时间行（值后附 `UTC` 标注）与独立一行 Git 信息（`分支名@hash`），现状行（Uptime、Rust toolchain、Router listen、Providers）不动（design D4 行序）。验证：`pnpm run build` 过，dev:sandbox 打开 Settings → About 目检两行渲染与行序。
  - 备注（分工约定）：代码完成，GUI 目检移交主 agent（settings-modal.test.ts 已补两行渲染/行序/unknown 兜底的组件单测，`pnpm run test` 701 全过）。

## 5. 联调验收（AGENTS.md 联调规约）

- [x] 5.1 沙箱 HTTP 验证：`cargo build` 后按 AGENTS.md 沙箱菜谱起 bare core（`--webui`，auth=false、状态全钉沙箱目录），`GET /api/about` 断言新字段存在、`version` 与 `sebas --version` 同值、时间格式为 `YYYY-MM-DD HH:mm`。验证：curl 断言输出贴入汇报。
  - 断言结果（端口 9877，进程已停、临时目录已删）：8/8 PASS——新三字段在场（build_time=`2026-09-28 20:05`、git_branch=`main`、git_hash=`71e2ca6`，与 `git rev-parse` 一致）；version=`0.1.0` 与 `sebas --version` 同值；时间格式 `YYYY-MM-DD HH:mm` 正则过；既有字段（uptime/rustc/router_listen/provider_count）不变。
- [x] 5.2 回归收口：`cargo test` 全绿 + `cargo clippy` 无新告警；对照 specs 增量 MODIFIED 需求逐场景核对实现（BUILD 段两行呈现、unknown 兜底、现状行不变）。验证：测试输出与逐场景核对清单。
  - `cargo test`：737 passed / 85 ignored / 0 failed（known-flaky `slow_stream_exposes_full_fsm_via_debounced_pump` 全量跑过 + 单跑复核均过）。
  - `cargo clippy` / `--all-targets`：与 stash 基线逐条 diff，新增告警 0 条。
  - 前端：`pnpm run test` 701/701 过、`pnpm run build` 过、`tsc --noEmit` 干净。
  - 逐场景核对：BUILD 段两行呈现（组件单测钉行序 Version→Build time→Git→现状行）✓；unknown 兜底（组件 unknown@unknown 照实渲染 + API unknown 字段在场）✓；现状行不变（Uptime/Rust/Router/Providers 既有用例全过）✓；`/api/about` 只增字段、RBAC 表未动 ✓。
