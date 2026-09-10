# Design: fix-clippy-and-frontend-unit-gates

## Context

验收实测（2026-09-09，基线 `d15ea65`）：

- clippy：3 处 / 1 文件（`tests/state_subscription_test.rs:95`、`:168`、`:199`）。该文件在 rebase 前后与 `origin/main` **字节一致**，2026-09-05 引入——既有红灯。
- 前端单测：148/148 用例通过，但 2375 个未处理 rejection 导致 vitest 退出 1。二分验证：rebase 前 `d2ec931` 同样退出 1（897 个错误）——既有红灯，本次窗口重写两个测试文件后错误数放大到 2.6 倍。
- 仓库既有约定：`tests/upgrade_dev_test.rs:32` 等 **11 处**对同类串行锁显式 `allow(clippy::await_holding_lock)`；ElementInternals 垫片在 `workbench-composer.test.ts` 内联实现，`review-card.test.ts` 注释自述「same shim」——**复制而非共享**。
- CI 现状（`.github/workflows/ci.yml`）：仅 `clippy` + `build` + `cargo test`，不含前端单测，故该红灯从未被门禁捕获。

## Goals / Non-Goals

**Goals:**

- 八道门禁全绿，且绿灯可持续（回归能被 CI 挡住）。
- 改动面最小、与既有约定一致，不动产品行为。

**Non-Goals:**

- 见 proposal Non-goals：不重构测试锁、不追 flaky、不纳 Playwright 进 CI。

## Decisions

**D1：clippy 用显式 `#[allow]` + `if let`，不重构测试。**
理由：串行 `std::sync::Mutex` 是测试的刻意设计（整个测试体持锁以防并发串改全局 engine/DB），仓库已有 11 处同样豁免并带注释说明；改成异步锁会改变测试语义。`single_match` 属机械修复。
备选：改用 `tokio::sync::Mutex`——语义变了，且与既有约定不一致，否决。

**D2：WA 渲染垫片抽成共享 helper（`src/test-support/wa-polyfills.ts`）。**
理由：漏配的根因正是「复制到每个测试文件」这一模式——2 处有、2 处无。实现中发现 jsdom 缺口**不止一个**：`ElementInternals.setValidity` → `HTMLDialogElement.showModal` → `Element.getAnimations` 逐个暴露；因此不再逐个导出给测试文件，而提供聚合入口 `installWaDomPolyfills()`——新增测试文件只需一行 import，后续再遇缺口只改这一处。
备选：继续内联复制（改动最小）——但第三次仍会漏，否决。
约束：helper 命名不含 `.test.`，且 `vite.config.ts` 的 `include: ['src/**/*.test.ts']` 不会把它当测试收集。

**D3：`pnpm test` 纳入 CI。**
理由：不纳入则本变更修好的门禁下一次仍会静默变红；该套件不依赖后端（`docs/frontend-dev.md`），CI 成本可接受。
备选：只修不纳（最小）——但问题根因是可见性，否决。

**D4：flaky 用例不入本变更。**
理由：`approval-detached.spec.ts:88`（allow path）首次 `toBeVisible()` 失败、重试通过，属稳定性观察项，与「门禁红灯」不同性质；另记 bd issue 跟踪。

## Risks / Trade-offs

- 抽 helper 后若某测试文件依赖「垫片安装时机」，import 顺序可能影响结果——用幂等安装 + 原型标记规避，并以 15 files / 148 tests / 0 errors 回归验证。
- CI 增 pnpm 步骤拉长流水线：仅跑前端单测（约 2s 级），可接受。
- `app-shell.test.ts` 已有模块 mock：只补垫片，不替换其 mock 策略。
