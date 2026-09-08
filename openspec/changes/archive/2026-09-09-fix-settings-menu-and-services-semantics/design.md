## Context

`sebas-webui/frontend/src/views/settings-modal.ts` 的 sections 数组（line 50）目前为 `models / services / appearance / env / about`，缺省首项 `models`。前端 `renderServices()` 把 `/api/router` 的 listen / debug / auth 字段当作「服务」渲染，是产品语义错位——后端 watchdog 已有真源 `GET /api/admin/services`（`sebas-webui/src/admin.rs:448`）和 enable/disable 动作（`sebas-webui/src/admin.rs:207-216`），通过 `src/webui_cmd.rs` 装配的 `AdminAdapter`（control RPC）走 watchdog，未连接时 `adapter_ok: false` 503 退化。watchdog 监督循环的受管集合为 `core / webui / router / im`（`ServiceName::Im` 对应 `service_from_str("im")`，**不是 `feishu`**——`src/watchdog/services.rs:482` 明确禁止）。本次只改前端呈现 + 数据源契约，不动后端 API 形状、不动 watchdog 监督策略。

## Goals / Non-Goals

**Goals:**
- 设置弹窗分区顺序：`Settings`（壳/总览）→ `Services` → `Models` → `Appearance` → `Env` → `About`；缺省首项改为 `Settings`。
- Services 分区改读 `/api/admin/services`，展示 watchdog 受管子进程与 enable/disable/restart；裸 core 形态退化。
- Models 分区顶部承接 router 总览（listen / debug / auth 三行）。
- 「Settings 分区」作为总览面板提供工作区根目录 / default agent kind / default provider/model 只读项与两个高危动作入口。
- 新加的 use case 在 spec 层有覆盖；老 settings 单测与 Playwright `settings.spec.ts` 同步更新。

**Non-Goals:**
- 不重做设置弹窗视觉、文案、i18n 框架。
- 不改后端 API 形状（`/api/admin/services`、`/api/router`、`/api/summary`、`/api/agent-defaults` 都不动）。
- 不引入“依赖图/编排/排程”类运维面板。
- 不修裸 core 形态已有 503 行为，只在 UI 上诚实呈现。
- 不动 watchdog 监督策略（崩溃退避、自动回滚、独立重启间隔等都是 watchdog spec 既有规约）。
- 不动 IM/飞书的产品品牌名，只在前端做 i18n 字符串映射（`im` → 「飞书」）。

## Decisions

### D1：缺省首项与顺序硬编码在 `SECTIONS` 数组，分区 id 稳定不变
- 决策：`SECTIONS` 数组顺序调整为上文列举；`section: SettingsSection = 'settings'`（新增类型枚举值）；历史记忆键 `lastSettingsSection` 用 localStorage。
- 依据：现有 sections 数组即 spec 规约的数据结构；新增 `'settings'` 不破坏其它分区 id；分区顺序由数组直接驱动无需新组件。
- 备选：把分区做成注册表对象 → 否决：当前 spec/sections 单层数组已足够，分区注册表会引入动态加载/路由化语义，超出本期范围。

### D2：Services 分区数据源切换为 `/api/admin/services`，保留 `/api/router` 给 Models 顶部
- 决策：`renderServices()` 删除 router 数据耦合，改读 `/api/admin/services`；`renderModels()` 顶部新增 router 总览卡（listen / debug / auth 三行）。
- 依据：产品语义上「Service」应指 watchdog 受管子进程；`/api/router` 的 listen/debug/auth 是「provider 路由网关」事实，归到 Models 顶部比丢进 Services 更准确（既保留可见性又不污染 Services 语义）。
- 备选：彻底删除 listen/debug/auth 卡片 → 否决：它们在调试时有用，操作员常想确认 router 是否在 listen；放在 Models 顶部保留可访问性。

### D3：enable/disable/restart 走既有 REST 端点，不引入新的 webui 内部路径
- 决策：`POST /api/admin/services/{name}/enable|disable`、`POST /api/admin/services/{name}/restart`；name 用 `service_from_str(name)` 之后的字符串（`im` 而非 `feishu`）。
- 依据：`/api/admin/services/{name}/enable|disable` 已存在并经 admin 测试覆盖（`sebas-webui/src/admin.rs:611-615`）；restart 路径复用 watchdog 监督循环已有的 `restart(name)`，通过既有 `Admin actions via control plane` 接入。
- 备选：新建专用 `/api/services/*` 簇 → 否决：路由面扩张无收益、与 admin cluster 同质。

### D4：高危动作 confirm 复用现有 wa-dialog 模式
- 决策：「全部进程重启」「重置 Settings」点击后弹 wa-dialog，描述影响（前者会重启所有受管服务并中断进行中会话；后者清空 localStorage 的 settings 记忆与全部 UI 缓存），用户输入二次确认词或点确认按钮后调用对应后端接口。
- 依据：webui 既有 wa-dialog 用法覆盖（`app-shell.test.ts` 已经测过 wa-dialog hidden 属性读写）；不需要引入新的 modal 框架。
- 备选：使用浏览器原生 `confirm()` → 否决：与现有 UI 风格不一致、不可定制文案、无法承载多字段（"输入确认词"）。

### D5：历史记忆用 localStorage 单一键 `lastSettingsSection`，不做服务端同步
- 决策：`localStorage.lastSettingsSection = id`；每次弹窗 `updated()` 时校验 id 在 `SECTIONS` 中否则回退缺省 `settings`。
- 依据：单用户本地 webui，localStorage 同步、无并发写风险；不需要服务端持久化（用户在多设备开 webui 是各自独立）。
- 备选：用 `URL hash` 持久化 → 否决：弹窗是 overlay 不应改 URL；用户复制带 hash 的链接无意义。

### D6：adapter 缺失态用「横幅+按钮全灰」组合呈现，不另起 error section
- 决策：当 `/api/admin/services` 返回 `{adapter_ok: false, services: []}`，Services 分区顶部渲染单行 `无 watchdog 控制面` 横幅，三个动作按钮全部 disabled 且 tooltip 说明原因。
- 依据：现有 admin cluster 退化语义（webui spec line 73：「/api/admin/* reads report adapter_ok: false」）已经定义；UI 落地延续同一规约即可。
- 备选：渲染一个虚构「unavailable」service 卡 → 否决：违反 spec「无适配器时返回空数组」。

### D7：IM/飞书命名映射为前端 i18n 一行
- 决策：前端维护 `SERVICE_DISPLAY_NAME: Record<string,string>`，`im` → 「飞书 IM」；其它服务名沿用小写原名。
- 依据：`service_from_str("feishu")` 已在 services.rs:482 测试明确返回 None；前端禁止用 `feishu` 调用任何 REST；只在显示文案上做 i18n 映射。
- 备选：后端把 service name 直接返回为「feishu」 → 否决：会破坏 `ServiceName` 枚举一致性、波及 RPC 协议；改名面比加映射大得多。

### D8：仅改 settings-modal.ts 与其测试；前端路由不变
- 决策：弹窗仍是居中 overlay，不引入路由；分区切换由 `section` state 内部驱动。
- 依据：`app-shell.ts:476` 已用 `<sebas-settings-modal>` 作为静态挂载点；改路由会破坏既有 IA-v1 退役规约（webui spec line 18）。
- 备选：分区改路由 → 否决：IA-v1 已退役 `/settings` 等路径并 canonical 到 `/`，再开新路由会回弹。

## Risks / Trade-offs

- [R1] 改 sections 数组顺序破坏现有 settings 单测断言 → mitigation：先跑 `pnpm --dir sebas-webui/frontend test -- settings-modal.test.ts` 拿到全部失败断言，再针对性改；不在本 change 顺手重构断言覆盖。
- [R2] Playwright `settings.spec.ts` 旧断言指向 Services 看到 router listen 信息 → mitigation：在 settings.spec.ts 新增/修改 S5 条款断言「Services 列表含 core/webui/router/im 四行」（裸 core 形态断言「无 watchdog 控制面横幅」）；S2/S3/S4 条款不动。
- [R3] 缺省首项改 `settings` 可能让“想看 Models”的高频用户多一次点击 → mitigation：localStorage 记忆可让用户首次切到 Models 后下次直达；视觉上 `Models` 仍是导航第二位。
- [R4] enable/disable 操作误触（特别是 disable core）→ mitigation：所有三个动作包在 wa-dialog 二次确认；confirm 文案必须含受影响进程名与「不可撤销」字样；core enable/disable 需二次输入确认词。
- [R5] `/api/admin/services` 在裸 core 形态返回 `{services: [], adapter_ok: false}` 但 user 不理解为何按钮全灰 → mitigation：横幅文案明确写「无 watchdog 控制面；启用请运行 `sebas run`」并提供跳转/链接到 docs。
- [R6] Services 改读 admin services 后，`/api/router` 不再在 Settings 弹窗出现 → 调试 operator 可能一时找不到 listen/debug/auth 信息 → mitigation：放在 Models 顶部，README 与 settings.spec 同步标注；该需求被刻意从 Services 迁出，spec scenario 明确写出。

## Migration Plan

按三笔 commit 顺序独立可回滚：

1. **数据源迁移（commit 1）**：`settings-modal.ts` 内 `renderServices` 改读 `/api/admin/services`；`renderModels` 顶部新增 router 总览卡；前端类型从 `RouterInfo` 切到 `AdminService`。本笔 commit 跑 `pnpm --dir sebas-webui/frontend test` 与 `invoke testsuite-webui --case settings` 各 1 次确认无回归。
2. **分区顺序与 Settings 壳（commit 2）**：`SECTIONS` 数组调整为新顺序；新增 `'settings'` 分区渲染总览 + 高危动作；localStorage `lastSettingsSection` 记忆。跑 `pnpm test` + `invoke testsuite-webui --case settings` 全绿。
3. **Playwright + 账本（commit 3）**：`tests/testsuite-webui/tests/settings.spec.ts` 新增/调整 S5 断言；`tests/testsuite-webui/README.md` 与 `tests/acceptance/COVERAGE.md` 同步本 change 索引；3 连绿验收。

回滚：commit 1 可独立 revert（前端仅是数据源切换）；commit 2 可独立 revert（纯 UI 结构改动）；commit 3 仅改测试/账本可独立 revert。

## Open Questions

无。契约层面后端不需要任何新增 API；前端 i18n 边界由 D7 收口。