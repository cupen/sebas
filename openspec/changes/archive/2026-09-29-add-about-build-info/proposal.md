## Why

Settings → About 的 BUILD 段目前只呈现版本号，而主 spec 场景早已写明 BUILD 段应呈现「版本、commit、构建时间」——实现一直缺后两样。排障时（本地增量构建与发布构建混用）无法辨认运行中的二进制来自哪个提交、何时构建。git 分支名与短 hash 已由根 `build.rs` 注入（升级检查在用），只差暴露到 About；构建时刻尚无任何管道。

## What Changes

- 根 `build.rs` 新增注入 `BUILD_TIME`（UTC 编译时刻，`YYYY-MM-DD HH:mm` 分钟精度），复用现有 GIT_BRANCH / GIT_HASH 注入机制。
- `GET /api/about` 响应新增 `build_time`、`git_branch`、`git_hash` 字段；鉴权与 RBAC 不变（认证即可读，viewer 可读）。
- 构建信息由 root 经现有启动装配边界（`run_with_admin_adapter_and_auth` → `WebUiState`）传入 sebas-webui；`/api/about` 的 `version` 改读传入值，与 `sebas --version` 同源。
- Settings → About 的 BUILD 段：Version chip 之下新增「构建时间」行与独立一行 Git 信息（`分支名@短hash`）；Uptime、Rust toolchain、Router listen、Providers 现状行不变。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: 「设置弹窗分区与缺省首项」——BUILD 段内容固化：版本 chip、UTC 构建时间行（附 UTC 标注）、Git 行形态（`分支名@短hash`）与构建信息缺失时的 unknown 兜底。

## Impact

- 后端：根 `build.rs`（新增 BUILD_TIME 注入，chrono 进 build-dependencies）、`src/upgrade.rs`（构建信息助手）、`src/webui_cmd.rs` 与 `src/run.rs`（装配点构造 BuildInfo 传入）、`sebas-webui/src/server.rs`（`BuildInfo` struct、`WebUiState` 字段、run 系列签名）、`sebas-webui/src/api.rs`（about 处理器）。
- 前端：`sebas-webui/frontend/src/api/client.ts`（About 接口加字段）、`sebas-webui/frontend/src/views/settings-modal.ts`（renderAbout 新增两行）。
- 无破坏性变更：`/api/about` 只增字段，旧前端不受影响；不新增公开端点；`/health` 契约不动。

## Non-goals

- 不做页脚 / 登录页 / 首启页的构建信息展示（立项拷问中用户明确改道 About 页）。
- 不新增公开版本端点（如未认证可读的 `GET /api/version`）。
- 不改 `/health` 契约与 RBAC 矩阵。
- 不引入 vergen 等构建信息全家桶；不统一 root 与 sebas-webui 的版本号管理（属 `add-tag-release-pipeline` 领域）。
- 不改动升级检查（upgrade.rs 现有行为保持）。
