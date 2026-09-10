## MODIFIED Requirements

### Requirement: Admin actions via control plane
Admin mutations SHALL proxy to the watchdog over the control RPC using the `SEBAS_CONTROL_SECRET`, attributed to a local CLI actor — which executes directly without a confirmation round-trip. **补充**：Actions 在原有 update (release)、update dry-run、update dev、rollback、restart core 之外，新增「Services 分区内的 enable/disable/restart」三类——enable/disable 经 `POST /api/admin/services/{name}/enable|disable` 走 `ServiceSet` RPC（仅对辅助服务 webui/router/im 开放；core 恒启动，无 enable/disable 入口）；restart per-process 经既有 watchdog 监督循环（每个受管服务都有 restart 入口）。**补充**：裸 core 形态下所有 admin mutations SHALL 返回 503 "control plane not connected"，Services 分区 SHALL 据此呈现退化。

#### Scenario: restart via admin

- **WHEN** the admin clicks restart on `/admin/update`-style pages with the control secret present
- **THEN** the watchdog receives `RestartCore` and restarts the core; the WebUI itself stays up

#### Scenario: no control plane

- **WHEN** the standalone WebUI has no `SEBAS_CONTROL_SECRET`
- **THEN** admin mutation buttons return 503; Services 分区显示「无 watchdog 控制面」横幅

### Requirement: Services 分区数据源
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）。core 为恒启动服务：其行 SHALL 仅呈现状态与 restart 入口，SHALL NOT 渲染 enable/disable 按钮。无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。Models 分区顶部 SHALL 呈现 provider 路由网关总览（来自既有 `/api/router`：listen / debug / auth 三行），与 Services 分区在产品语义上彻底解耦。

#### Scenario: Services 渲染受管子进程

- **WHEN** watchdog 拉起 core/webui/router/im 四个进程，Services 分区聚焦
- **THEN** 列表呈现 core / webui / router / im 四行及 desired / actual / uptime；im 在配置未启用时不出现该行

#### Scenario: 无 watchdog adapter 退化

- **WHEN** webui 在裸 core 形态下启动（无 SEBAS_CONTROL_SECRET）
- **THEN** Services 分区显示「无 watchdog 控制面」横幅、列表为空、enable/disable/restart 按钮不渲染

#### Scenario: enable 成功

- **WHEN** 操作员对 `router` 点击 enable
- **THEN** 前端 POST `/api/admin/services/router/enable` 收到 200；列表行刷新（desired 变 on）；若后端返回 503 则内联呈现错误且不刷新

#### Scenario: disable 成功

- **WHEN** 操作员对辅助服务（如 `router`）点击 disable（前提：confirm 弹窗已确认）
- **THEN** 前端 POST `/api/admin/services/router/disable` 收到 200；列表行刷新；core 行不渲染 disable 按钮

#### Scenario: restart 操作

- **WHEN** 操作员对 `core` 点击 restart
- **THEN** 前端走 `Admin actions via control plane` 既有 restart-core 路径；成功后列表行 uptime 重置

#### Scenario: Models 分区承载 router 总览

- **WHEN** 操作员聚焦 Models 分区
- **THEN** 主区顶部呈现 provider 路由网关卡片（listen / debug / auth 三行），下方为 provider 列表；不再包含 Services 标签
