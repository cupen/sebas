## MODIFIED Requirements

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

provider 管理面（`/api/providers*`、`/api/provider-presets`、
`/api/provider-defaults`、`/api/model-aliases*`）SHALL 不纳入角色执法：
仅要求有效登录（登录门开启时），并保持其既有守卫（POST-only + origin
检查）。router 进程自身的下游 token 鉴权（`[router] auth_token`，默认关
闭）不在本能力范围，暂不改动。

#### Scenario: viewer 只读

- **WHEN** viewer 会话 `POST /api/sessions`（或任何会话/项目写操作）
- **THEN** 返回 403

#### Scenario: member 不能碰设置与服务

- **WHEN** member 会话调用 `POST /api/settings` 或 `/api/admin/services`
  类控制面
- **THEN** 返回 403

#### Scenario: router BFF 仅登录门

- **WHEN** member 会话调用 `/api/providers`（写，原 `/router/api/*` 面）
- **THEN** 不因角色被 403（登录门与自身 origin 守卫照常生效）

#### Scenario: admin 不能管用户

- **WHEN** admin 会话调用用户管理端点
- **THEN** 返回 403

#### Scenario: 禁用用户即刻失效

- **WHEN** root 禁用某在线用户
- **THEN** 该用户后续 API 请求返回 401，重新登录被拒绝
