# Tasks: fix-clippy-and-frontend-unit-gates

## 1. clippy 门禁修复

- [x] 1.1 `tests/state_subscription_test.rs`：`:95`、`:199` 两个用例加 `#[allow(clippy::await_holding_lock)]` + 一行理由注释（与 `tests/upgrade_dev_test.rs:32` 同写法），验证：`cargo clippy --workspace --all-targets -- -D warnings` 不再报该 lint
- [x] 1.2 同文件 `:168`：`match frame { Changed { scope } => …, _ => {} }` 改 `if let StateStreamFrame::Changed { scope } = frame`，验证：`cargo test --test state_subscription_test` 通过（2 passed）
- [x] 1.3 全量 clippy 复跑确认无其它 offender（验收已初筛：`pump_unit` / `routing_paths` / `proxy_smoke` / `admin_test` / `session_endpoints` 均用 `tokio::sync::Mutex` 或同步 helper 持锁），验证：`cargo clippy --workspace --all-targets -- -D warnings` 退出 0

## 2. 前端单测垫片收口

- [x] 2.1 新增 `sebas-webui/frontend/src/test-support/wa-polyfills.ts`：导出幂等的 `installElementInternalsPolyfill()` / `installDialogPolyfill()` / `installWebAnimationsPolyfill()` 与聚合入口 `installWaDomPolyfills()`（原型标记防重复安装）
- [x] 2.2 `views/workbench-composer.test.ts` / `components/review-card.test.ts` 改为 import 共享 helper 并删除内联副本（composer 的 sanity-check 改用 `elementInternalsPolyfillInvoked()`），验证：两文件单测全绿
- [x] 2.3 `views/settings-modal.test.ts` / `app-shell.test.ts` 接入聚合入口（`app-shell.test.ts` 保留其既有模块 mock，仅补垫片），验证：两文件 0 未处理 rejection
  - 实现中发现计划漏项（超出原任务描述，已并入本变更）：`wa-dialog` 需要 `HTMLDialogElement.showModal`，补上后又暴露 `Element.getAnimations`（jsdom 无 Web Animations API）。为免逐文件漏配，改为单一聚合入口 `installWaDomPolyfills()`。
- [x] 2.4 前端单测全量回归：15 files / 148 tests / **0 errors**，验证：`pnpm --dir sebas-webui/frontend test` 退出码 0

## 3. 门禁可见性

- [x] 3.1 `.github/workflows/ci.yml` 增 `frontend` job（`pnpm/action-setup@v4` 钉 11.24.0 → node 22 → `pnpm install --frozen-lockfile` → `pnpm test`），验证：本地同一序列 rc=0、YAML 解析通过
- [x] 3.2 flaky 用例不重复建单：既有 bd 任务 `sebas-q5k`（detached 审批 allow 路径冷窗口首试丢帧）已含根因与验收标准，本变更只确认其状态未变，验证：`bd show sebas-q5k`（仍 OPEN）

## 4. 验收

- [x] 4.1 八道门禁全量重跑（`CARGO_INCREMENTAL=0 bash /tmp/sebas-verify.sh`），验证：SUMMARY 全 PASS（8/8）
- [x] 4.2 `openspec validate fix-clippy-and-frontend-unit-gates --strict` 通过
