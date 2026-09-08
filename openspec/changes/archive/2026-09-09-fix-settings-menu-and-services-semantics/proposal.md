## Why

webui 设置弹窗有两个语义错位：**菜单顺序**上 `Settings` 项自身未居首，缺省首项是 `Models`，操作员点开弹窗第一眼看到的不是「设置」本身；**Services 分区语义**上当前填的是 `/api/router` 数据（Router listen / debug / auth），但产品语义上「Services」应该是 watchdog 实际拉起并监督的子进程（Core / WebUi / Router / 可选 IM）。后端 `/api/admin/services` 已具备 enable/disable 动作，前端没用上——错把 router 当 services 是产品语义漏洞，需要修正并把 watchdog 控制面契约前移到 webui。

## What Changes

- **新增**：settings 弹窗分区顺序与缺省首项调整为 `Settings`（弹窗壳/总览，含操作入口、版本信息、快捷恢复）→ `Services`（watchdog 受管子进程）→ `Models`（provider 路由）→ `Appearance` → `Env` → `About`。缺省打开时聚焦 `Settings` 而非 `Models`。
- **新增**：Services 分区改读 `/api/admin/services`，渲染 Core / WebUi / Router / IM（按受管集合动态出现，非受管服务不出现）以及它们的 desired / actual / uptime / 最近错误；无 watchdog adapter 时显式 `adapter_ok: false` 且不暴露 enable/disable 按钮。
- **新增**：Services 分区对每个进程提供 enable / disable / restart 操作，经现有 `/api/admin/services/{name}/enable`、`.../disable`、watchdog restart 路径执行；操作结果内联呈现 success/error；bare-core（无 watchdog）形态下按钮全灰且 tooltip 说明「无 watchdog 控制面」。
- **新增**：settings 弹窗的 `Settings` 分区提供工作区根目录、当前 default agent kind、当前 default provider/model 三个总览项（只读），以及「全部进程重启（watchdog 形态下可见）/ 重置 Settings」两个高危动作的二次确认入口。
- **修改**：把 `/api/router` 的 listen / debug / auth 卡片从 Services 分区搬回 `Models` 分区顶部（Models 分区已有 provider 列表，加 listen 行作为“provider 路由网关”总览）；Models 分区不再称为「Models 与 Services 都来自 /api/router」。
- **修改**：现有后端 `/api/admin/services` 已是契约；本 change 不变更 API 形状、不变更 `ServiceName` 枚举（Core/WebUi/Router/Im），但前端要尊重 IM 名称映射（`service_from_str("im")` 而非 `"feishu"`）。
- **移除**：`renderServices()` 在 settings-modal.ts 里读取 `/api/router` 的逻辑被替换；现有 Services 分区文案“Background services that run alongside sebas.”保留，但实际数据源切换。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `webui`: 设置弹窗分区顺序、缺省首项、Services 分区数据源（`/api/router` → `/api/admin/services`）与 enable/disable/restart 接线；为 `/api/admin/services` 在裸 core 形态下的适配呈现与裸 core 高危动作收口建立规约。
- `watchdog`: 「受管服务发现面」 requirement 明确 IM 字符串名为 `im`（非 `feishu`）；webui→watchdog services 通讯契约在 spec 层显形。

## Impact

- 受影响代码：`sebas-webui/frontend/src/views/settings-modal.ts`（sections 数组顺序、缺省值、renderServices、renderModels）、`sebas-webui/src/api.ts`（types 切到 `AdminService`）、`sebas-webui/src/views/settings-modal.test.ts`（断言调整）。
- 受影响 spec：`openspec/specs/webui/spec.md`（设置弹窗分区 requirement、HTTP 路由面增补 adapter 行为）、`openspec/specs/watchdog/spec.md`（IM 字符串命名）。
- 受影响测试：`tests/testsuite-webui/tests/settings.spec.ts`（Services 分区断言需重写为受管服务面）、`tests/testsuite-webui/README.md`（账本同步）。
- 不影响：被测 webui 后端路由形状（无 304/405 行为变化）、vitest 单测里 settings-modal 的渲染逻辑只调数据源不动断言覆盖、CI workflow。

## Non-goals

- 不重做设置弹窗的视觉（暗色面板、左侧导航、文案不改；只在结构与数据源上修）。
- 不动 `/api/router` 的 listen/debug/auth 字段本身（保留给 Models 分区顶部作为 provider 路由网关总览）。
- 不在 Services 分区里造“服务依赖图、metrics、tracing”类运维面板——本期只做“当前有哪些受管子进程+enable/disable/restart”。
- 不改 watchdog 监督循环、自动回滚策略、崩溃退避（属 `watchdog` spec 既有规约，不在本次范围）。
- 不支持“跨进程排程”（如启用 router 的同时禁用 core）——enable/disable 按进程粒度。
- 不改 IM/飞书产品名（用户仍叫「飞书」），只修 webui 内部字符串映射。