## MODIFIED Requirements

### Requirement: Webui-primary deployment shape

The watchdog SHALL always register and spawn the core (session authority) — the core is the permanent session core every channel client depends on, not an optional service; there is no `[watchdog.core] enabled` switch. Webui SHALL remain the primary control service (`[watchdog.webui] enabled` default `true`). An optional IM service (hosting channel adapters) SHALL by default follow its channel's enablement — enabled channel launches the IM service, disabled channel does not — and SHALL be overridable via `[watchdog.im]`. Webui and the IM service SHALL both be clients of the core session channel and SHALL stay alive across core restarts; they are the core's two channels (`web` and any IM channel), interacting through the channel abstraction.

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值且飞书未启用
- **THEN** `sebas run`（watchdog 守护）恒拉起 core，并默认拉起 webui；router 不启动

#### Scenario: 通道启用时 im 服务默认拉起

- **WHEN** 配置启用某 IM 通道且无 `[watchdog.im]` 覆盖
- **THEN** `sebas run` 拉起 im 服务（其内注册该通道适配器），webui 与 im 跨 core 重启保持存活

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者尝试通过配置或服务页停用 core
- **THEN** 无此开关/入口；core 恒由 watchdog 拉起，仅支持重启（RestartCore 确认路径），不响应 enable/disable
