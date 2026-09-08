# tasks — fix-settings-menu-and-services-semantics

> 状态注记（2026-09-08 实测）：§1–§3 声称的代码增量在树上已存在（`api/client.ts` 5 方法、`settings-modal.ts` 的 SECTIONS 顺序/renderSettings/renderServices/网关卡/i18n 映射）——§1–§3 不重做，跑验证绿了即勾选；本 change 真正剩余工作只有 §4（spec + 账本）。勾选前必须实跑，不得凭本注记直接勾。

## 1. 数据源迁移：Services 从 `/api/router` 切到 `/api/admin/services`

- [x] 1.1 在 `sebas-webui/frontend/src/api/client.ts` 内引入 `AdminService` 类型（与 `sebas-webui/src/admin.rs:448` 的 JSON 形状对齐：name / status / desired / uptime_secs），并在 `/api/admin/services` 失败时返回 `{adapter_ok: false, services: []}`；类型导出给 settings-modal 用；运行 `pnpm --dir sebas-webui/frontend test -- settings-modal.test.ts` 验证（验证：编译通过、全绿；若有失败说明树上代码与本 task 描述有漂移，先对齐代码再勾选，不顺手改单测）——2026-09-08 实测：树上已存在且全绿（settings-modal.test.ts 新 IA 断言 142 passed），勾选
- [x] 1.2 重写 `sebas-webui/frontend/src/views/settings-modal.ts` 的 `renderServices()`：删除对 `/api/router` 的依赖，改读 `/api/admin/services`；每行展示 name（含 im→「飞书 IM」 i18n 映射）/ desired / actual / uptime / 最近错误（来自 `/api/admin/events`）；无 adapter 时展示「无 watchdog 控制面」横幅；运行 `pnpm test` 验证 settings 单测全绿（验证：renderServices 内无 `router` 字符串、调用 `.adminServices()`）——2026-09-08 实测：已存在且单测断言 `.router` 未被调用、全绿，勾选
- [x] 1.3 在 `renderModels()` 顶部新增「provider 路由网关」总览卡，三行分别渲染 listen / debug / auth（数据来自 `/api/router`）；保留 provider 列表渲染不变；运行 `pnpm test -- settings-modal.test.ts` 验证 Models 区断言通过（验证：Models 渲染内出现 `Router 路由网关` 标题与三行字段）——2026-09-08 实测全绿，勾选

## 2. 分区结构：顺序与缺省首项 + Settings 总览壳

- [x] 2.1 在 `settings-modal.ts` 调整 `SECTIONS` 数组顺序为 `settings / services / models / appearance / env / about`；`SettingsSection` 类型新增 `'settings'`；`section` 缺省值改为 `'settings'`；运行 `pnpm test` 验证无新增断言失败（验证：types 编译通过、缺省断言指向 `'settings'`）——2026-09-08 实测全绿，勾选
- [x] 2.2 在 `settings-modal.ts` 实现 `renderSettings()`：渲染工作区根目录（来自 `/api/summary`，含复制按钮）/ default agent kind（来自 `/api/agent-defaults`）/ default provider-model（同上 + 跳转 Models 链接）三项只读总览；运行 `pnpm test` 验证渲染（验证：单测断言三类数据字段均在渲染输出内）——2026-09-08 实测全绿（工作区根实际来自 `/api/fs/browse-dirs` 回显根，`/api/summary` 无该字段；行为已按诚实数据源断言），勾选
- [x] 2.3 在 `renderSettings()` 底部新增「全部进程重启」「重置 Settings」两个高危动作按钮；点击各自弹 wa-dialog 二次确认；确认后调用 `POST /api/admin/restart`（restart-core 路径）与清空 localStorage `lastSettingsSection`；无 watchdog adapter 时按钮 disabled + tooltip「无 watchdog 控制面」；运行 `pnpm test -- settings-modal.test.ts` 验证按钮渲染与 disabled 状态（验证：两种状态下按钮存在/disabled 切换正确、confirm 弹窗在点击后挂载）——2026-09-08 实测全绿，勾选
- [x] 2.4 在 `settings-modal.ts` 的 `updated()` 钩子加 localStorage `lastSettingsSection` 读写；首次打开用缺省 `'settings'`，后续打开恢复上次分区；分区 id 不在 `SECTIONS` 时回退 `'settings'`；运行 `pnpm test` 验证记忆回放（验证：mock localStorage 写入 `'services'` 后打开应聚焦 Services；写入不合法值应回退 Settings）——2026-09-08 实测全绿，勾选

## 3. 前端类型与适配层

- [x] 3.1 在 `sebas-webui/frontend/src/api/client.ts` 增加 `.adminServices(): Promise<AdminService[]>`、`.adminEvents(): Promise<AdminEvent[]>`、`.enableService(name)`、`.disableService(name)`、`.restartService(name)` 五个方法（enable/disable/restart 复用现有 admin HTTP 路径）；类型完整、错误码处理区分 401/403/503/network；运行 `pnpm test -- api/client.test.ts` 验证（验证：5 个方法编译通过、单测覆盖 503 路径返回 `adapter_ok: false`）——2026-09-08 实测：树上五方法 + Safe 变体已存在，api/client.test.ts 全绿，勾选
- [x] 3.2 在 `sebas-webui/frontend/src/api/client.ts` 删除仅供 Settings 使用的 `/api/router` 旧 fields 引用（保留 `listen / debug / auth` 给 Models 总览卡）；运行 `pnpm test` 验证无破坏（验证：grep 不到 `renderServices` 调用 `router` 字段；Models 总览渲染路径完整）——2026-09-08 实测：renderServices 只调 adminServicesSafe/adminEventsSafe（单测断言 router 未调用），Models 网关卡全绿，勾选

## 4. Playwright 与账本同步

- [x] 4.1 在 `tests/testsuite-webui/tests/settings.spec.ts` 修改 S5 条款断言：Services 分区列出的行名集合 SHALL 与 `/api/admin/services` 响应一致（响应即真源，不枚举具体服务——沙箱装配是变量；只断言"每一行名都在响应集合内、响应集合内每一行都被渲染"）；运行 `invoke testsuite-webui --case settings` 验证全绿（验证：S1/S2/S3/S4 不变、S5 断言改写后通过）——2026-09-08 落地说明：tasks 的 "S5" 实为文件内的 S1 用例（`S1 services rows match /api/admin/services truth`，双向对账；编号漂移在此记录）；S2/S3/S4 断言不变（S4/S5a/S5b 仅追加 `openSection('Models')` 导航以适配缺省 Settings 首项；`models.spec.ts` 3.2 同理）；`invoke testsuite-webui --case settings` 7 passed（`1e4a807`），勾选
- [x] 4.2 在 `tests/testsuite-webui/tests/settings.spec.ts` 新增 S6 条款：裸 core 形态（沙箱无 watchdog）下 Services 分区显示「无 watchdog 控制面」横幅、按钮 disabled；验证（验证：S6 单独 1 次全绿）——2026-09-08 实测：S6 断言横幅 + 零行 + 零行动作 + Settings 总览「全部进程重启」disabled/tooltip（重置 Settings 保持可用），随 --case settings 7 passed 通过（`1e4a807`），勾选
- [x] 4.3 更新 `tests/testsuite-webui/README.md` 与 `tests/acceptance/COVERAGE.md`：把本期 change hash 与本 tasks 路径记入 `testsuite-webui-browser` 段；运行 `openspec validate --changes` 验证 delta 通过（验证：validate 2 passed 0 failed）——2026-09-08：两账本已记 `1e4a807` + tasks §4；另补 README 漏记的 errors spawn 行（与 COVERAGE 对齐）；`openspec validate --changes` 实测 6 passed 0 failed（树上共 6 个 change，本 change 通过；tasks 预期的 "2 passed" 为立项时基数，现如实记录）
- [x] 4.4 跑 `invoke testsuite-webui` 全量 3 连绿；it 基数更新为 34（converge 收敛时 33 + 本期 S6 新增 1；S5 改写不增数）；COVERAGE 的"行数等于 it 总数"校验按 34 执行（验证：与既有三期验收一致的稳定性门槛；本 change 涉及用例全绿）——2026-09-08 BLOCKED（非本 change 回归）：全量 run1 31 passed + `projects 增删`  deterministically red（full + 独立 --case projects 共 4 连败）；本 change 全部 7 个 settings 用例 + models 3.2 全绿。基数实测：改前树上已 34 its（非 33），本期 +S6 后 35；两账本已按 35 对齐。详见 BLOCKER 备注（tasks 末尾）。——2026-09-08 解除（授权修测试侧，见下方 BLOCKER 解除记录）：`projects.spec.ts` 增删断言改 basename 文本 + title 全路径后，全量 3 连绿（每轮 32 passed 主 config + 3 passed auth config = 35/35）；基数实测 `grep '^\s*test('` = 35，两账本 35 行/35 it 对齐成立。

## BLOCKER（2026-09-08，§4.4 门槛外失败，与本 change 无关）——同日解除

- `tests/testsuite-webui/tests/projects.spec.ts:64`（项目管理覆盖「增删」）期望
  `sebas-dashboard .project-header .path` 文本为完整沙箱路径，但已提交的产品代码
  `sebas-webui/frontend/src/views/dashboard.ts:327-333` 确定性只渲染 basename
  （完整路径仅放 `title` 悬浮）。双方均为已提交代码，本分支未动这两处。
- 原文失败：`Error: expect(locator).toHaveText(expected) failed / Locator:
  locator('sebas-dashboard .project-header .path') / Expected:
  "/tmp/sbtestsuite.k_gf2e5r" / Received: "sbtestsuite.k_gf2e5r" / Timeout:
  10000ms`
- 按授权（产品文件 believed correct、非本 scope 不改产品/他人用例）未做任何修改，
  留给所属方修（产品改 header 或用例改断言二选一）。本 change 的 §4.4 全绿门槛待其
  修复后重跑 3x。

- **解除记录（2026-09-08，授权决策：修测试侧）**：产品行为确认为刻意（commit
  `204a901` "chore(webui): drop chevron, drop handle, tighten project rail layout"
  收紧项目 rail；`dashboard.ts:327-334` 只渲染 basename，完整路径放 `title` 悬浮），
  故按授权修测试侧：`projects.spec.ts` 增删断言改为 `.path` 文本期望 basename、
  新增 `title` 属性携带完整路径的断言（路径身份仍被锚定），并注明与
  dashboard.ts basename 渲染对齐；产品代码零改动。解除后全量 3 连绿
  （每轮 32 + 3 = 35，详见 §4.4）。

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change fix-settings-menu-and-services-semantics --json` 验证四个 artifact 全部 `done`（验证：proposal/specs/design/tasks 状态均为 done；isPlanningComplete: true）
- [ ] 5.2 跑 `openspec validate --changes --strict` 无报错（验证：delta 字段完整、scenario 全 SHALL/MUST）
- [ ] 5.3 在 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 段落末尾追加 `Settings 语义修正（fix-settings-menu-and-services-semantics）` 一行指向本期 commit hash 与本 tasks（验证：账本自身可追溯）