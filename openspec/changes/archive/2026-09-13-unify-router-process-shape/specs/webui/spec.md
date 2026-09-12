## MODIFIED Requirements

### Requirement: Services 分区与 router 状态归属
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）。core 为恒启动服务：其行 SHALL 仅呈现状态与 restart 入口，SHALL NOT 渲染 enable/disable 按钮（enable-core-by-default）。无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。router 的运行状态（desired / actual / uptime）SHALL 仅由本分区呈现；Models 分区 SHALL NOT 呈现 router 网关总览、listen / debug / auth 或任何 router 运行状态，provider 管理面与 router 运行状态在产品语义上分离。

停止 `router` SHALL 走强制出口流：停止请求被拒（存在活跃 routed 会话，响应携带会话计数）时，前端 SHALL 呈现确认对话框——显示活跃会话计数与后果（流式中断），操作员可取消或选择「强制停止」；强制停止 SHALL 以 `force: true` 重发并被服务端放行。前端 SHALL NOT 预先查询活跃会话数（避免竞态窗口），拒绝驱动弹窗即可；并发竞态（确认期间会话增减）由服务端再次执法兜底。

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
- **THEN** 前端 POST `/api/admin/services/router/disable` 收到 200；列表行刷新；core 行不渲染 enable/disable 按钮

#### Scenario: router 停止被拒呈现强制出口

- **WHEN** 操作员停止 `router` 被拒且响应携带活跃 routed 会话计数
- **THEN** 前端呈现确认对话框（计数与流式中断后果），操作员选择「强制停止」后以 force 重发并成功，列表行刷新为 stopped

#### Scenario: router 停止被拒后取消

- **WHEN** 操作员在强制出口对话框中取消
- **THEN** 不发送任何请求，router 行保持原状

#### Scenario: restart 操作

- **WHEN** 操作员对 `core` 点击 restart
- **THEN** 前端走 `Admin actions via control plane` 既有 restart-core 路径；成功后列表行 uptime 重置

#### Scenario: router 状态只在 Services 呈现

- **WHEN** 操作员分别聚焦 Models 与 Services 分区
- **THEN** Models 分区只呈现 provider 列表与管理入口，不渲染 router 的 listen / debug / auth 或可达性；router 的 desired / actual / uptime 只在 Services 分区呈现，且该分区在无 watchdog adapter 时如实说明不可用
