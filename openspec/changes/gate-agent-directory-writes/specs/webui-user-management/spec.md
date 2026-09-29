## MODIFIED Requirements

### Requirement: RBAC 角色与权限执法

角色集 SHALL 固定为四档，角色到权限的映射由服务端代码定义（不可经
API 修改）：

| 权限 | root | admin | member | viewer |
|---|---|---|---|---|
| 用户管理（users.manage） | ✓ | | | |
| 系统设置（卡片/显示偏好写，settings.manage） | ✓ | ✓ | | |
| agent 目录写（agents 增/改/删，agents.manage） | ✓ | ✓ | | |
| 服务控制（watchdog 服务启停/升级/回滚，services.control） | ✓ | ✓ | | |
| 会话与项目写（创建/发消息/归档/恢复/项目增删，sessions.write） | ✓ | ✓ | ✓ | |
| 只读（工作台读面 + `/ws` 事件流） | ✓ | ✓ | ✓ | ✓ |

执法 SHALL 完全在服务端路由层完成：无对应权限的已认证请求得到 403
（而非仅前端隐藏入口）。前端 SHALL 按当前用户角色隐藏其无权限的入口
（含 viewer 的「新建会话」入口与 Agents 分区的写入口），但这只是呈现层
优化，不构成防线。被禁用用户的登录 SHALL 被拒绝（401，与凭据错误同
文案），其既有会话立即失效。

provider 管理面（`/api/providers*`、`/api/provider-presets`、
`/api/provider-defaults`、`/api/model-aliases*`）SHALL 不纳入角色执法：
仅要求有效登录（登录门开启时），并保持其既有守卫（POST-only + origin
检查）。router 进程自身的下游 token 鉴权（`[router] auth_token`，默认关
闭）不在本能力范围，暂不改动。

agent 目录读（列表/详情）SHALL 保持登录门即可——角色只约束写。

#### Scenario: viewer 只读

- **WHEN** viewer 会话 `POST /api/sessions`（或任何会话/项目写操作）
- **THEN** 返回 403

#### Scenario: viewer 不能写 agent 目录

- **WHEN** viewer 会话对 `/api/agents` 执行增、改、删任一写操作
- **THEN** 返回 403，agent 目录不变

#### Scenario: member 不能写 agent 目录

- **WHEN** member 会话对 `/api/agents` 执行增、改、删任一写操作
- **THEN** 返回 403

#### Scenario: member 不能碰设置与服务

- **WHEN** member 会话调用 `POST /api/settings` 或 `/api/admin/services`
  类控制面
- **THEN** 返回 403

#### Scenario: admin 可写 agent 目录

- **WHEN** admin（或 root）会话对 `/api/agents` 执行增、改、删
- **THEN** 操作生效（与现状一致）

#### Scenario: router BFF 仅登录门

- **WHEN** member 会话调用 `/api/providers`（写，原 `/router/api/*` 面）
- **THEN** 不因角色被 403（登录门与自身 origin 守卫照常生效）

#### Scenario: admin 不能管用户

- **WHEN** admin 会话调用用户管理端点
- **THEN** 返回 403

#### Scenario: 无权限入口在 UI 隐藏

- **WHEN** viewer 登录进入工作台
- **THEN** 「新建会话」入口与 Settings → Agents 的写入口（New/Edit/Delete）
  不呈现；只读浏览（agent 列表、会话读面）不受影响

#### Scenario: 禁用用户即刻失效

- **WHEN** root 禁用某在线用户
- **THEN** 该用户后续 API 请求返回 401，重新登录被拒绝

### Requirement: 会话绑定用户

登录建立的会话 SHALL 绑定到具体用户 id；角色不快照进会话，SHALL 按
用户库实时解析（用户被删/禁用时按无权限的失效会话处理）。会话
cookie 语义（HttpOnly、SameSite=Lax、24h 不活动过期、按 IP 登录限速）
保持不变。`GET /api/auth/me` 在已认证时 SHALL 返回该用户的用户名与
角色。修改某用户角色后，其新请求 SHALL 按新角色执法（无需重新登录）。

`/api/auth/me` 返回的 `role` SHALL 在界面有展示位（如侧栏用户区），
操作员能随时看到当前生效角色。

#### Scenario: me 返回身份与角色

- **WHEN** 已认证会话请求 `GET /api/auth/me`
- **THEN** 响应包含 `username` 与 `role` 字段

#### Scenario: 角色调整即时生效

- **WHEN** root 将某在线 member 降级为 viewer
- **THEN** 该用户下一个写操作请求返回 403，无需重新登录

#### Scenario: 当前角色可见

- **WHEN** 任一已登录用户查看工作台界面
- **THEN** 界面展示其当前角色（与 `/api/auth/me` 一致），登出后不残留

#### Scenario: 注销与会话过期

- **WHEN** 用户注销或会话超过 24h 不活动
- **THEN** 会话失效，后续请求返回 401 并回到登录页
