## 1. Token 与 store 基建

- [ ] 1.1 `tokens.css` 新增 notice token 组：`--sebas-notice-info/warn/error`（alias `--sebas-status-starting/working/failed`）+ `--sebas-notice-warn-bg` / `--sebas-notice-fatal-bg`（深底、对白字 ≥ AA）；验证：`pnpm test` 全绿且 tokens.css 无既有 status 语义改动
- [ ] 1.2 新增 `src/notify.ts` store：`notify({ level, message, duration?, dedupeKey? })` + 订阅广播；栈上限 3（挤最旧瞬时条，error 驻留不参与）、8s 窗口同文案去重；验证：vitest 覆盖上限挤占 / 去重 / 订阅退订

## 2. 通知层组件

- [ ] 2.1 新增 `sebas-notice-layer`：挂 `wa-toast`（`placement="top-center"`、`--width: 28rem`）并接 store——映射表 brand/info→5s、warning/warn→8s、danger/error→`duration=0` 驻留（3.12.0 无 closable，关闭按钮内建）；accent 经 `--sebas-notice-*` 覆盖、icon 走 `wa-icon`；验证：vitest 断言四级变体/时长/accent 与 live role（danger=alert/assertive，其余 status/polite）
- [ ] 2.2 新增 `sebas-notice-banner`（持续态横幅）：warn 断线态与 fatal 态（role=alert、不可关、`tabindex=-1` 可聚焦）；fatal 文案按 kind 三档 + cause 原文小字、kind 缺失退化通用文案；验证：vitest 覆盖 kind 分档与退化
- [ ] 2.3 层布局：视口 fixed top-center 纵向堆叠、横幅在场时经 `--sebas-notice-top-offset` 下移 toast 栈、z-index 取 wa-dialog overlay 实测值 +1、≤480px 全宽贴顶；验证：vitest 断言偏移变量生效与叠放次序
- [ ] 2.4 `api/client.ts` 的 `ReachabilityInfo` 补 `kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'`；app-shell 5s 轮询接 store：`reachability.ok=false` 进 fatal、恢复出 fatal + 弹「核心已恢复」info；移除 app-shell 中 cover-A kind 的既有 TODO 注释；验证：vitest 模拟 summary 假/真两态

## 3. fatal 锁定与 WS 收编

- [ ] 3.1 fatal 期间对 `wa-split-panel.frame` 施加 `inert` + 遮罩（`--wa-color-overlay-modal` 同款半透明 + 居中原因卡与轮询提示）；锁定时焦点移入横幅、恢复归还（元素已失落 body）；auth 门禁页（login/setup 渲染分支）不受影响；验证：vitest 断言 `inert` attribute 进出与焦点落点
- [ ] 3.2 `sebas:ws-state` 断线接 store 呈现为持续 warn 驻留横幅（重连即消、既有 `sebas:refetch` 不动）；删除 `app-shell.ts` 旧 `ws-banner` / `core-banner` 及 stacked 逻辑；验证：vitest 覆盖断线→横幅、重连→消失 + refetch 触发

## 4. client.ts 统一拦截

- [ ] 4.1 `ApiError` 抛出前统一拦截未豁免调用并按影响面自动 notify（网络失败 / 5xx 于操作调用 → warn）；豁免名单集中一处并以注释互链本 change（settings 表单、composer 提交、dashboard/列表加载、`/api/summary` 轮询、401 跳登录路径不入通知层）；验证：vitest 覆盖未豁免必弹 / 豁免不弹 / 401 不入层
- [ ] 4.2 各内联错误调用点打 opt-out 标（保持现有内联呈现，不改文案）；验证：现有视图 vitest 全绿、无一处失败双弹

## 5. 测试迁移与验收

- [ ] 5.1 迁移旧 `data-testid="core-unreachable-banner"` 相关断言到新横幅 testid；更新 `app-shell.test.ts`；验证：`pnpm test` 全绿
- [ ] 5.2 `testsuite-webui-browser` 增补旅程场景：core 停止 → fatal 横幅 + 工作台不可交互；core 恢复 → 解锁 + 「核心已恢复」；未豁免调用失败 → warn toast；按 AGENTS.md 沙箱菜谱（`invoke testsuite-webui-sandbox` / fake provider）跑通；验证：acceptance 场景通过并如实上报沙箱边界（fake 答案而非真模型）
- [ ] 5.3 全量门禁：`pnpm test` + `pnpm build`（tsc/vite）；核对 specs delta 每条 Scenario 均有对应测试或如实注明缺口；验证：CI 级命令零红
