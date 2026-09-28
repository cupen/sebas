## Context

- 根 `build.rs` 已注入 `GIT_BRANCH`（`--abbrev-ref`）与 `GIT_HASH`（`--short`），仅根 crate 可读（`cargo:rustc-env` 只对拥有 build 脚本的包生效）；`src/upgrade.rs` 的 `current_version()` 在消费。
- `GET /api/about` 处理器在 sebas-webui crate，`version` 现读 sebas-webui 自身的 `CARGO_PKG_VERSION`；构建时刻不存在任何管道。
- root 经 `sebas_webui::run_with_admin_adapter_and_auth`（`src/webui_cmd.rs:299`、`src/run.rs:494` 两个装配点）启动 webui，状态聚在 `WebUiState`。
- 主 spec「Settings 分区总览」场景已预告 BUILD 段含「版本、commit、构建时间」，实现缺后两样——本 change 是补齐并固化形状。
- 立项拷问已拍板：展示位改 About 页（放弃页脚方案）、git 信息单独一行（`分支名@短hash`）、数据走扩展 `/api/about`（仅认证后）、构建日期取编译时刻到分钟。

## Goals / Non-Goals

- Goals：About BUILD 段可见构建时间 + git 信息；`/api/about` 成为唯一数据通道；构建信息单点注入。
- Non-Goals：见 proposal Non-goals（不做页脚、不加公开端点、不动 `/health` 与 RBAC）。

## Decisions

### D1. 构建信息经现有装配边界 DI 下发（不新增 crate、不重复注入）

sebas-webui 新增 `BuildInfo { version, build_time, git_branch, git_hash }`（含 `BuildInfo::unknown()` 兜底），挂到 `WebUiState`；root 两个装配点在启动时构造真值传入。`run` / `run_with_admin_adapter` 两个薄包装传 `unknown()`（它们拿不到根 crate 的编译期 env——`option_env!` 在 sebas-webui 内读不到根 build.rs 注入的值，这是本次最关键的约束）。

- 否决「新微型 build-info crate」：为 ~20 行 git 采集新建 crate 过重，且运行时读取仍需落在消费 crate（`option_env!` 按编译 crate 求值），共享收窄为 build 脚本助手，收益不成比例。
- 否决「sebas-webui/build.rs 重复注入」：出现两份 git 采集逻辑，与仓库单一定义习惯相悖。
- 附带语义修正：`version` 改读 root 传入值后与 `sebas --version` 同源（今日两 crate 均为 0.1.0，数值不变；版本号管理统一属 `add-tag-release-pipeline`，不在本 change）。

### D2. BUILD_TIME：根 build.rs 注入 UTC 编译时刻，分钟精度

根 build.rs 以 `SystemTime::now()`（经 chrono 格式化 `%Y-%m-%d %H:%M`，chrono 已在工作区依赖，加进 `[build-dependencies]`）注入 `cargo:rustc-env=BUILD_TIME`。

诚实边界：`rerun-if-changed` 只盯 `.git/HEAD` 与 `.git/refs/heads`，**无新提交的本地增量重编不会刷新 BUILD_TIME**（build 脚本不重跑）；干净 checkout 的发布构建总是新鲜。陈旧性以 git hash 区分代码版本，分钟粒度让陈旧肉眼可辨——这正是立项拷问中用户拍板「编译时刻到分钟」时接受的语义，About 照实显示、不做运行时补偿。

### D3. 字段形状与 unknown 兜底

`/api/about` 响应追加 `build_time` / `git_branch` / `git_hash` 三个字符串字段（缺省 `"unknown"`，沿用 `upgrade.rs` 的 fallback 惯例），只增不改，旧前端兼容。前端 Git 行渲染 `分支名@hash`；构建时间行附 `UTC` 标注（标注放展示层，API 值保持纯 `YYYY-MM-DD HH:mm` 便于机器比对）。

### D4. UI 行序

renderAbout 的 BUILD 段行序：Version chip → 构建时间 → Git 信息 → 现状行（Uptime、Rust toolchain、Router listen、Providers）不动。新增两行紧跟版本号，读序「是什么版本 → 何时构建 → 来自哪个提交」。

## Risks / Trade-offs

- BUILD_TIME 在增量构建下可能陈旧 → 接受（D2），git hash 承担代码版本区分；spec 场景只约束「如实呈现」。
- run 系列签名继续加参（已有 `too_many_arguments` 豁免）→ 接受，不为此引入参数 struct 大重构（超出本 change 范围）。
- 两处装配点（`webui_cmd` / `run.rs`）都要接线 → 漏一处表现为 About 显示 unknown，单测 + sandbox 冒烟双闸覆盖。

## Migration Plan

无存量数据迁移；纯字段追加与 UI 增行，回滚即恢复旧响应形状。

## Open Questions

（无——立项拷问已收束，剩余细节按假设记录：UTC 标注放展示层；unknown 照实显示不隐藏。）
