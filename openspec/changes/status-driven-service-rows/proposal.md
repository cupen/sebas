# Proposal: status-driven-service-rows

## Why

Settings → Services 分区当前每行同时渲染 ▶/■/⟳ 三个按钮，不看服务实际状态；`/api/admin/services` 还混入 watchdog 后端伪造的 `watchdog` / `updater` 两行合成数据，它们渲染出动作按钮但点击必被 RPC 层拒绝（wire 只收 ManagedService 枚举）；core 行的 restart 按钮实际绑定的是必拒的通用路径（前端真正可用的 `/api/admin/restart` 从未被调用）——一颗哑弹；且各行动作区宽度不一，状态列在行与行之间横向漂移。

## What Changes

- **后端**：watchdog `service_status()` 不再合成 `watchdog` / `updater` 两行，`ServiceStatus` 只报告真实受管服务（core / webui / router / im）。三个消费面（webui、CLI `sebas ctl services`、IM `/services`）同时只见真实服务；「不允许人工控制 watchdog/updater」由 wire 层枚举 + 列表不出现共同结构保证。
- **前端动作互斥**：非 core 行按 actual status 驱动按钮——`running` 只显 ■；`stopped` / `disabled` 只显 ▶；`starting` / `restarting` 显禁用过渡占位（不可点、列宽不变）；`degraded` / `failed-startup` 显 ■ + ⟳。⟳ 在非过渡态保留；busy 期间全部禁用；disable/restart 确认弹窗与 router force 停止保护流程不变。
- **core 行完全只读**：不渲染任何动作按钮（连 ⟳ 也移除）；core 重启只经 CLI / 升级流程。顺带删除前端死方法 `api.restartCore()`（后端 `/api/admin/restart` 路由保留）。
- **纵向对齐**：动作区定宽、flex 填充占位，所有行的状态圆点与文字对齐到同一 x；core / 过渡行的空缺同样占位。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：「Services 分区与 router 状态归属」——core 行从「状态 + restart 入口」改为纯只读；新增动作按钮随 status 互斥显示的规则与过渡态呈现；新增各行状态列纵向对齐要求。
- `watchdog`：ServiceStatus 报告面——明确列表 SHALL NOT 含合成行（watchdog 自身与 updater 是监督器内部角色，不是受管服务）。

## Impact

- 后端：`src/watchdog/executor.rs`（`service_status()` 及其钉死合成行的测试）。
- 前端：`sebas-webui/frontend/src/views/settings-modal.ts`（`renderServiceRow`、相关 CSS）、`src/api/client.ts`（删 `restartCore`）及两处测试。
- API 兼容性：`GET /api/admin/services` 响应条目收窄为 4 种受管名（消费方仅 webui 前端）；CLI/IM 渲染面随之少两行，非破坏（其行为如实反映列表）。

## Non-goals

- 不改控制 RPC 的执法语义（`ServiceSet`/`ServiceRestart` 的 ManagedService 枚举拒绝行为原样保留）。
- 不动 router 停止保护（force 出口流）与确认弹窗交互。
- 不新增 admin API 路由或过滤参数（不搞 `?include=synthetic`）。
- 不做 i18n / 视觉主题改版，只动 Services 行的动作区与对齐。
- 不改升级/回滚（updater）本身——只是不再把它当"服务"展示。
