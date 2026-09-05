## ADDED Requirements

### Requirement: 鉴权开关（auth）

WebUI SHALL 提供 `[watchdog.webui] auth` 配置开关，默认 `true`。
开关为 `true` 时，登录鉴权行为不变：凭据文件存在即启用鉴权门，`/api/*`、
`/gateway/api/*`、`/ws` 需要有效会话。开关为 `false` 时，即使凭据文件存在，
SHALL 对所有路由（含静态资源）完全放行，不要求登录；`GET /api/auth/me`
SHALL 报告 `enabled: false`（前端据此不渲染登录页）。`sebas webui-passwd`
在开关关闭时仍可管理凭据（为重新启用做准备），但不产生任何强制登录效果。

#### Scenario: 默认打开

- **WHEN** 配置未写 `auth` 且凭据文件存在
- **THEN** 未带会话的 `/api/summary` 请求返回 401，行为与无开关时一致

#### Scenario: 测试环境关闭

- **WHEN** 配置设置 `watchdog.webui.auth = false` 且凭据文件存在
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 已存在的凭据立即恢复强制登录，无需重建凭据文件

### Requirement: 非 loopback bind 与开关联动

当 `watchdog.webui.host` 非 loopback 时，webui SHALL 仅在
`auth = true`（或缺省）且登录凭据存在时才允许绑定启动；否则 SHALL 以配置
错误拒绝启动。开关关闭时无论凭据是否存在，SHALL 拒绝非 loopback bind
（防止误关开关叠加公网暴露）。

#### Scenario: 开关关闭拒绝公网 bind

- **WHEN** 配置同时设置 `auth = false` 与 `host = "0.0.0.0"`
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开且凭据存在允许公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
  且凭据文件存在
- **THEN** `sebas webui` 正常绑定并在日志中提示已启用登录鉴权
