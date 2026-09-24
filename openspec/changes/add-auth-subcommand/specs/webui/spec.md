## MODIFIED Requirements

### Requirement: 鉴权开关（auth）与首启用户引导

WebUI SHALL 提供 `[service.webui] auth` 配置开关，默认 `true`。开关为 `true` 时，鉴权门 SHALL 恒在：`/api/*`、`/router/api/*`、`/ws` 需要有效 会话；用户库（auth.db）零用户时 SHALL 不自动生成任何凭据，改为进入 首启引导流程（见 `webui-user-management` 能力：设置页或环境变量建立 root），期间 `GET /api/auth/me` SHALL 报告 `needs_setup: true`。开关为 `false` 时，无论用户库是否存在用户，SHALL 对所有路由（含静态资源） 完全放行，不要求登录且不触发引导；`GET /api/auth/me` SHALL 报告 `enabled: false`（前端据此不渲染登录页）。`sebas auth`（`add`/`passwd`/`list`）在开关关闭时仍可管理用户（为重新启用做准备），但不产生任何强制登录效果。

#### Scenario: 默认打开且有用户

- **WHEN** 配置未写 `auth` 且用户库存在启用用户
- **THEN** 未带会话的 `/api/summary` 请求返回 401，行为与无开关时一致

#### Scenario: 默认打开且零用户进入设置流程

- **WHEN** 配置未写 `auth`、用户库零用户、且未设凭据环境变量
- **THEN** `GET /api/auth/me` 返回 `needs_setup: true`（前端渲染首启
设置页而非登录页），受保护 API 仍对未带会话请求返回 401

#### Scenario: 环境变量引导 root

- **WHEN** 用户库零用户且 `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`
非空
- **THEN** 启动时建立名为该用户名的 root 用户，`needs_setup` 不再出现

#### Scenario: 测试环境关闭

- **WHEN** 配置设置 `service.webui.auth = false`
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 用户库中已有的用户立即恢复强制登录，无需重建用户
