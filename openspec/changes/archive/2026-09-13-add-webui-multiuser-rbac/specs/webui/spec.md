## MODIFIED Requirements

### Requirement: Local-only binding

The standalone WebUI SHALL default to a loopback bind (`127.0.0.1:9797`).
The legacy `core --webui` path binds hard-coded `127.0.0.1`. A non-loopback
`watchdog.webui.host` SHALL be refused with a configuration error unless the
authentication switch is enabled and at least one enabled user exists in the
user store (see「鉴权开关（auth）与首启用户引导」and「非 loopback bind 与
开关联动」below).

#### Scenario: non-loopback refused without auth

- **WHEN** the config sets `watchdog.webui.host = "0.0.0.0"` while the user
  store holds no enabled user, or while `auth = false`
- **THEN** `sebas webui` exits with a configuration error rather than
  binding

### Requirement: 非 loopback bind 与开关联动

当 `watchdog.webui.host` 非 loopback 时，webui SHALL 仅在 `auth = true`
（或缺省）且用户库存在至少一个启用用户时才允许绑定启动。零用户时
SHALL 先经环境变量引导（`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`）
建立 root，否则以配置错误拒绝启动——防止公网下「先访问者注册
root」。开关关闭时无论用户库为何，SHALL 拒绝非 loopback bind
（防止误关开关叠加公网暴露）。

#### Scenario: 开关关闭拒绝公网 bind

- **WHEN** 配置同时设置 `auth = false` 与 `host = "0.0.0.0"`
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开但零用户拒绝公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
  用户库零用户且未设凭据环境变量
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开且凭据存在允许公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
  且用户库存在启用用户（含环境变量引导建立的 root）
- **THEN** `sebas webui` 正常绑定并在日志中提示已启用登录鉴权

### Requirement: Optional admin authentication

The `/api/admin/*` control-plane surface SHALL require its own admin session
(a separate cookie from the main login) when `SEBAS_CONTROL_SECRET` is
configured: unauthenticated admin API requests get a JSON 401 (the HTML
`/admin/*` pages are retired; unauthenticated page paths fall through the
SPA fallback). A successful admin login sets an HttpOnly, SameSite=Lax
cookie with a 24 h TTL, and login attempts are rate-limited to 5 per 30 s.
When no control secret is configured, admin reads are loopback-only and
mutations report the control plane as disconnected. The main session APIs
are governed by the「鉴权开关（auth）与首启用户引导」requirement, not this
one.

#### Scenario: password-gated admin API

- **WHEN** the admin credential is set and an unauthenticated request hits
  `/api/admin/status`
- **THEN** the response is a JSON 401 (not a redirect)

#### Scenario: login lockout

- **WHEN** 6 login attempts with a wrong password arrive within 30 s
- **THEN** the attempts are rejected by the rate limiter

## REMOVED Requirements

### Requirement: 鉴权开关（auth）与凭据自动引导

**Reason**: 单账户凭据模型随多用户化整体替换；「凭据缺失时自动生成
随机密码（用户名 admin）」的引导路径删除（BREAKING），凭据文件
（webui-auth.json）不再是鉴权数据源。
**Migration**: 鉴权开关本身语义不变（`[watchdog.webui] auth` 默认
`true`，`false` 全放行），由新需求「鉴权开关（auth）与首启用户引导」
接替；零用户时经首启设置页或环境变量建立 root（见
`webui-user-management` 能力）；旧凭据文件不读取不迁移（未正式发布，
无兼容负担），可手工删除。

### Requirement: 单字段登录（token 或密码）

**Reason**: 多用户下单个密码字段无法定位账户（密码不再唯一标识
用户），「secret 既可能是 token 也可能是单账户密码」的自动识别语义
随之失效。
**Migration**: 登录形态改为 `{"username", "password"}`（登录页两
字段），由新需求「多用户登录形态」接替；`{"secret"}` 单字段与
`SEBAS_WEBUI_TOKEN` 移除（未正式发布，不保留兼容形态）。

## ADDED Requirements

### Requirement: 鉴权开关（auth）与首启用户引导

WebUI SHALL 提供 `[watchdog.webui] auth` 配置开关，默认 `true`。开关为
`true` 时，鉴权门 SHALL 恒在：`/api/*`、`/router/api/*`、`/ws` 需要有效
会话；用户库（auth.db）零用户时 SHALL 不自动生成任何凭据，改为进入
首启引导流程（见 `webui-user-management` 能力：设置页或环境变量建立
root），期间 `GET /api/auth/me` SHALL 报告 `needs_setup: true`。开关为
`false` 时，无论用户库是否存在用户，SHALL 对所有路由（含静态资源）
完全放行，不要求登录且不触发引导；`GET /api/auth/me` SHALL 报告
`enabled: false`（前端据此不渲染登录页）。`sebas webui-passwd` 在开关
关闭时仍可管理用户（为重新启用做准备），但不产生任何强制登录效果。

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

- **WHEN** 配置设置 `watchdog.webui.auth = false`
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 用户库中已有的用户立即恢复强制登录，无需重建用户

### Requirement: 多用户登录形态

`POST /api/auth/login` SHALL 只接受 `{"username", "password"}` 一种
形态（旧单字段 `{"secret"}` 移除，缺失字段返回 400）。成功即建立
绑定该用户的会话 cookie；凭据失败统一 401，不区分「用户不存在」与
「密码错误」，限速策略不变（按来源 IP）。登录页 SHALL 呈现用户名 +
密码两个字段。

#### Scenario: 用户名密码登录

- **WHEN** `{"username", "password"}` 提交到 `/api/auth/login` 且凭据
  正确
- **THEN** 登录成功并建立绑定该用户的会话，响应携带该用户名

#### Scenario: 旧单字段形态不可用

- **WHEN** `{"secret": "..."}` 提交到 `/api/auth/login`
- **THEN** 返回 400（登录请求缺用户名/密码字段）

#### Scenario: 失败不泄漏用户存在性

- **WHEN** 提交不存在的用户名或错误密码
- **THEN** 响应统一为 401，文案不区分两种失败，响应时序无可区分的
  快慢差

#### Scenario: 登录页两字段

- **WHEN** 前端渲染登录门
- **THEN** 登录表单有用户名与密码两个输入框，401 就地提示「凭据错误」
