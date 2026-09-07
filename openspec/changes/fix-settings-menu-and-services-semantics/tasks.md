# tasks — fix-settings-menu-and-services-semantics

## 1. 数据源迁移：Services 从 `/api/router` 切到 `/api/admin/services`

- [ ] 1.1 在 `sebas-webui/frontend/src/api/client.ts` 内引入 `AdminService` 类型（与 `sebas-webui/src/admin.rs:448` 的 JSON 形状对齐：name / status / desired / uptime_secs），并在 `/api/admin/services` 失败时返回 `{adapter_ok: false, services: []}`；类型导出给 settings-modal 用；运行 `pnpm --dir sebas-webui/frontend test -- settings-modal.test.ts` 验证无新增失败（验证：编译通过、原有 settings 单测仅失败与数据源相关的部分）
- [ ] 1.2 重写 `sebas-webui/frontend/src/views/settings-modal.ts` 的 `renderServices()`：删除对 `/api/router` 的依赖，改读 `/api/admin/services`；每行展示 name（含 im→「飞书 IM」 i18n 映射）/ desired / actual / uptime / 最近错误（来自 `/api/admin/events`）；无 adapter 时展示「无 watchdog 控制面」横幅；运行 `pnpm test` 验证 settings 单测全绿（验证：renderServices 内无 `router` 字符串、调用 `.adminServices()`）
- [ ] 1.3 在 `renderModels()` 顶部新增「provider 路由网关」总览卡，三行分别渲染 listen / debug / auth（数据来自 `/api/router`）；保留 provider 列表渲染不变；运行 `pnpm test -- settings-modal.test.ts` 验证 Models 区断言通过（验证：Models 渲染内出现 `Router 路由网关` 标题与三行字段）

## 2. 分区结构：顺序与缺省首项 + Settings 总览壳

- [ ] 2.1 在 `settings-modal.ts` 调整 `SECTIONS` 数组顺序为 `settings / services / models / appearance / env / about`；`SettingsSection` 类型新增 `'settings'`；`section` 缺省值改为 `'settings'`；运行 `pnpm test` 验证无新增断言失败（验证：types 编译通过、缺省断言指向 `'settings'`）
- [ ] 2.2 在 `settings-modal.ts` 实现 `renderSettings()`：渲染工作区根目录（来自 `/api/summary`，含复制按钮）/ default agent kind（来自 `/api/agent-defaults`）/ default provider-model（同上 + 跳转 Models 链接）三项只读总览；运行 `pnpm test` 验证渲染（验证：单测断言三类数据字段均在渲染输出内）
- [ ] 2.3 在 `renderSettings()` 底部新增「全部进程重启」「重置 Settings」两个高危动作按钮；点击各自弹 wa-dialog 二次确认；确认后调用 `POST /api/admin/restart`（restart-core 路径）与清空 localStorage `lastSettingsSection`；无 watchdog adapter 时按钮 disabled + tooltip「无 watchdog 控制面」；运行 `pnpm test -- settings-modal.test.ts` 验证按钮渲染与 disabled 状态（验证：两种状态下按钮存在/disabled 切换正确、confirm 弹窗在点击后挂载）
- [ ] 2.4 在 `settings-modal.ts` 的 `updated()` 钩子加 localStorage `lastSettingsSection` 读写；首次打开用缺省 `'settings'`，后续打开恢复上次分区；分区 id 不在 `SECTIONS` 时回退 `'settings'`；运行 `pnpm test` 验证记忆回放（验证：mock localStorage 写入 `'services'` 后打开应聚焦 Services；写入不合法值应回退 Settings）

## 3. 前端类型与适配层

- [ ] 3.1 在 `sebas-webui/frontend/src/api/client.ts` 增加 `.adminServices(): Promise<AdminService[]>`、`.adminEvents(): Promise<AdminEvent[]>`、`.enableService(name)`、`.disableService(name)`、`.restartService(name)` 五个方法（enable/disable/restart 复用现有 admin HTTP 路径）；类型完整、错误码处理区分 401/403/503/network；运行 `pnpm test -- api/client.test.ts` 验证（验证：5 个方法编译通过、单测覆盖 503 路径返回 `adapter_ok: false`）
- [ ] 3.2 在 `sebas-webui/frontend/src/api/client.ts` 删除仅供 Settings 使用的 `/api/router` 旧 fields 引用（保留 `listen / debug / auth` 给 Models 总览卡）；运行 `pnpm test` 验证无破坏（验证：grep 不到 `renderServices` 调用 `router` 字段；Models 总览渲染路径完整）

## 4. Playwright 与账本同步

- [ ] 4.1 在 `tests/testsuite-webui/tests/settings.spec.ts` 修改 S5 条款断言：Services 分区列出的行名集合与 `/api/admin/services` 响应一致（沙箱 backend = `sebas core --router --debug --webui` 单进程，应至少有 core 与 router；webui 是被测进程本身不列入受管集合；IM 在配置中 disabled 故不出现）；运行 `invoke testsuite-webui --case settings` 验证全绿（验证：S1/S2/S3/S4 不变、S5 断言改写后通过）
- [ ] 4.2 在 `tests/testsuite-webui/tests/settings.spec.ts` 新增 S6 条款：裸 core 形态（沙箱无 watchdog）下 Services 分区显示「无 watchdog 控制面」横幅、按钮 disabled；验证（验证：S6 单独 1 次全绿）
- [ ] 4.3 更新 `tests/testsuite-webui/README.md` 与 `tests/acceptance/COVERAGE.md`：把本期 change hash 与本 tasks 路径记入 `testsuite-webui-browser` 段；运行 `openspec validate --changes` 验证 delta 通过（验证：validate 2 passed 0 failed）
- [ ] 4.4 跑 `invoke testsuite-webui` 全量 3 连绿（验证：与既有三期验收一致的稳定性门槛；本 change 涉及的 33 it 全部 + 新增条款 1 it 不破坏总数）

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change fix-settings-menu-and-services-semantics --json` 验证四个 artifact 全部 `done`（验证：proposal/specs/design/tasks 状态均为 done；isPlanningComplete: true）
- [ ] 5.2 跑 `openspec validate --changes --strict` 无报错（验证：delta 字段完整、scenario 全 SHALL/MUST）
- [ ] 5.3 在 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 段落末尾追加 `Settings 语义修正（fix-settings-menu-and-services-semantics）` 一行指向本期 commit hash 与本 tasks（验证：账本自身可追溯）