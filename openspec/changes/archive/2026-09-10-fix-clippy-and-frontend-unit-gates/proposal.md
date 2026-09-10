## Why

本轮验收（8 道门禁全跑）暴露两道**既有**红灯：`cargo clippy --workspace --all-targets -- -D warnings`（CI 的 push-main 门禁）在 `tests/state_subscription_test.rs` 报 3 处；`pnpm test`（`docs/frontend-dev.md` 定义的前端测试命令）因 2375 个未处理 rejection 退出 1。两者都不是本次改动引入（已二分验证），但修复面都很小；不修则 CI 长期红、前端门禁长期不可见。

## What Changes

- `tests/state_subscription_test.rs`：两个用例补 `#[allow(clippy::await_holding_lock)]`（串行锁有意横跨整个测试，与仓库既有 11 处同约定）；`:168` 的单分支 `match` 改 `if let`。
- 前端垫片收口：把 `workbench-composer.test.ts` / `review-card.test.ts` 内联的 ElementInternals 垫片抽成共享 helper，`settings-modal.test.ts` / `app-shell.test.ts` 接入，消除 2375 个未处理 rejection。
- `.github/workflows/ci.yml` 增 `pnpm test` 步骤，使该门禁可见——否则同类回归会继续静默复发。

## Non-goals

- 不重构测试设计（不把串行 `std::sync::Mutex` 换成异步锁）：锁是刻意的测试串行化手段。
- 不追查 `approval-detached.spec.ts:88` 的 1 例 flaky（另记 bd issue 观察）。
- 不把 Playwright 浏览器套件纳入 CI（需浏览器下载，成本另议）。
- 不改任何产品行为、spec 需求或 HTTP/API 面。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

(无——纯工程/测试卫生，无 spec 级行为变化；`.openspec.yaml` 置 `skip_specs: true`)

## Impact

- 代码：`tests/state_subscription_test.rs`；`sebas-webui/frontend/src/{app-shell,views/settings-modal,views/workbench-composer,components/review-card}.test.ts` + 新增共享垫片模块 `src/test-support/wa-polyfills.ts`。
- CI：`.github/workflows/ci.yml` 增一步（需 pnpm 与 `sebas-webui/frontend` 依赖）。
- 验证：`cargo clippy --workspace --all-targets -- -D warnings`、`pnpm test`、`cargo test --workspace`。
