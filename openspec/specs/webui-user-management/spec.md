# webui-user-management Specification

## Purpose
WebUI 多用户账户体系：独立 SQLite 用户库（auth.db）、首启 root 引导、
root 的用户管理界面，以及 RBAC 角色权限的服务端执法与会话绑定。

## Requirements

### Requirement: 多用户存储（auth.db）

用户 SHALL 存储在独立 SQLite 文件中：默认 `~/.sebas/auth.db`，
`SEBAS_WEBUI_AUTH_DB` 环境变量可覆盖（沙箱/测试隔离用）。每个用户
记录 SHALL 至少包含：唯一用户名、角色、PBKDF2-HMAC-SHA256 密码哈希
（随机盐 + 迭代次数随记录保存）、启用标志、创建/更新时间。明文密码
SHALL 绝不落盘。用户名唯一性 SHALL 大小写不敏感（`Alice` 与 `alice`
视为同一用户名，拒绝重复注册）。

#### Scenario: 首次访问建库

- **WHEN** auth.db 不存在且 webui 以鉴权开启启动
- **THEN** 文件被创建并初始化为零用户状态，不产生默认账户

#### Scenario: 密码不落明文

- **WHEN** 任一用户经任意途径（设置页、CLI、环境变量）建立或改密
- **THEN** auth.db 文件内容中检索不到该明文密码，只存在盐与哈希

#### Scenario: 用户名大小写不敏感唯一

- **WHEN** 已存在用户 `alice`，再尝试创建 `Alice`
- **THEN** 创建被拒绝并提示用户名已存在

#### Scenario: 旧单账户凭据文件被忽略

- **WHEN** 磁盘上存在旧版 `webui-auth.json`（或设置了
  `SEBAS_WEBUI_AUTH_FILE`）
- **THEN** webui 不读取、不写入、不迁移该文件；用户库仍为零用户并
  进入首启设置流程（旧系统不兼容，未正式发布）

### Requirement: 首启 root 引导

鉴权开启且用户库零用户时，SHALL 提供两条引导路径，且不自动生成任何
默认凭据：

1. **设置页**：`GET /api/auth/me` 报告 `needs_setup: true`，前端渲染
   首启设置页（自定义用户名 + 密码 + 确认密码）；`POST /api/auth/setup`
   接受该表单并创建 **root** 角色用户，成功后自动建立会话进入工作台。
   该端点在用户库非零时 SHALL 一律拒绝（409）。
2. **环境变量**：`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` 在启动时
   建立 root，优先于设置页。

设置页与 setup 端点 SHALL 与其它受保护 API 一样受非安全方法同源校验，
且创建的 root 密码 SHALL 满足最小长度 8（不满足即 400，不做静默降级）。

#### Scenario: 设置页创建 root

- **WHEN** 零用户时 `POST /api/auth/setup` 提交 `{username: "cupen",
  password: "long-enough"}` 且两次输入一致
- **THEN** 创建 root 成功，响应携带会话 cookie，`/api/auth/me` 报告
  该用户已认证

#### Scenario: 已有用户后 setup 拒绝

- **WHEN** 用户库非零时任何来源调用 `POST /api/auth/setup`
- **THEN** 返回 409，不创建任何用户

#### Scenario: 弱密码拒绝

- **WHEN** setup 表单提交长度小于 8 的密码
- **THEN** 返回 400 并提示密码过短，用户库保持零用户

### Requirement: 用户管理（root 专用）

SHALL 提供用户管理 HTTP 面（列表/创建/改角色/重置密码/启停/删除），
全部仅限 root 角色调用（非 root 一律 403，即使已认证）。约束：

- 不能删除或禁用**最后一个启用的 root**（400）；
- root 不能删除自己（400；禁用自己同样被最后-root 规则拦截）；
- 重置密码与禁用 SHALL 使该用户的既有会话立即失效；
- 删除用户 SHALL 同时清除其所有会话；
- 创建用户 SHALL 指定角色（root/admin/member/viewer）与初始密码。

#### Scenario: root 列表用户

- **WHEN** root 会话请求用户列表
- **THEN** 返回全部用户（用户名、角色、启用状态、时间戳；不含哈希）

#### Scenario: 非 root 被拒

- **WHEN** member 会话请求任一用户管理端点
- **THEN** 返回 403，操作不生效

#### Scenario: 最后一个 root 受保护

- **WHEN** 库中仅有一个启用的 root，尝试删除、禁用或将其降级为
  member
- **THEN** 操作被拒绝（400），该 root 保持原状

#### Scenario: 重置密码踢会话

- **WHEN** root 重置某在线用户的密码
- **THEN** 该用户既有的会话 cookie 立即失效，后续 API 返回 401

#### Scenario: root 创建用户

- **WHEN** root 提交 `{username, password, role: "member"}`
- **THEN** 用户创建成功并可立即用该凭据登录，权限按 member 生效

### Requirement: RBAC 角色与权限执法

角色集 SHALL 固定为四档，角色到权限的映射由服务端代码定义（不可经
API 修改）：

| 权限 | root | admin | member | viewer |
|---|---|---|---|---|
| 用户管理（users.manage） | ✓ | | | |
| 系统设置（卡片/显示偏好写，settings.manage） | ✓ | ✓ | | |
| 服务控制（watchdog 服务启停/升级/回滚，services.control） | ✓ | ✓ | | |
| 会话与项目写（创建/发消息/归档/恢复/项目增删，sessions.write） | ✓ | ✓ | ✓ | |
| 只读（工作台读面 + `/ws` 事件流） | ✓ | ✓ | ✓ | ✓ |

执法 SHALL 完全在服务端路由层完成：无对应权限的已认证请求得到 403
（而非仅前端隐藏入口）。前端 SHALL 按当前用户角色隐藏其无权限的入口，
但这只是呈现层优化，不构成防线。被禁用用户的登录 SHALL 被拒绝（401，
与凭据错误同文案），其既有会话立即失效。

`/router/api/*`（router BFF 面）SHALL 不纳入角色执法：仅要求有效登录
（登录门开启时），并保持其既有守卫（POST-only + origin 检查）。router
进程自身的下游 token 鉴权（`[router] auth_token`，默认关闭）不在本
能力范围，暂不改动。

#### Scenario: viewer 只读

- **WHEN** viewer 会话 `POST /api/sessions`（或任何会话/项目写操作）
- **THEN** 返回 403

#### Scenario: member 不能碰设置与服务

- **WHEN** member 会话调用 `POST /api/settings` 或 `/api/admin/services`
  类控制面
- **THEN** 返回 403

#### Scenario: router BFF 仅登录门

- **WHEN** member 会话调用 `/router/api/providers`（写）
- **THEN** 不因角色被 403（登录门与自身 origin 守卫照常生效）

#### Scenario: admin 不能管用户

- **WHEN** admin 会话调用用户管理端点
- **THEN** 返回 403

#### Scenario: 禁用用户即刻失效

- **WHEN** root 禁用某在线用户
- **THEN** 该用户后续 API 请求返回 401，重新登录被拒绝

### Requirement: 会话绑定用户

登录建立的会话 SHALL 绑定到具体用户 id；角色不快照进会话，SHALL 按
用户库实时解析（用户被删/禁用时按无权限的失效会话处理）。会话
cookie 语义（HttpOnly、SameSite=Lax、24h 不活动过期、按 IP 登录限速）
保持不变。`GET /api/auth/me` 在已认证时 SHALL 返回该用户的用户名与
角色。修改某用户角色后，其新请求 SHALL 按新角色执法（无需重新登录）。

#### Scenario: me 返回身份与角色

- **WHEN** 已认证会话请求 `GET /api/auth/me`
- **THEN** 响应包含 `username` 与 `role` 字段

#### Scenario: 角色调整即时生效

- **WHEN** root 将某在线 member 降级为 viewer
- **THEN** 该用户下一个写操作请求返回 403，无需重新登录

#### Scenario: 注销与会话过期

- **WHEN** 用户注销或会话超过 24h 不活动
- **THEN** 会话失效，后续请求返回 401 并回到登录页
