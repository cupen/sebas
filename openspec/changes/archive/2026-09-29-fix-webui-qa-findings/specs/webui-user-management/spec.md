## MODIFIED Requirements

### Requirement: 用户管理（root 专用）

SHALL 提供用户管理 HTTP 面（列表/创建/改角色/重置密码/启停/删除），
全部仅限 root 角色调用（非 root 一律 403，即使已认证）。约束：

- 不能删除或禁用**最后一个启用的 root**（400）；
- root 不能删除自己（400；禁用自己同样被最后-root 规则拦截）；
- root SHALL NOT 能通过用户管理面对**自己的账户**执行降权、禁用或删除：
  服务端 SHALL 以 40x 显式拒绝这类自指操作，前端 SHALL 对自己所在的用户行
  禁用这些危险控件（降权下拉的降权项、禁用开关、删除按钮），并保留对
  自己行的改密/重置自己密码路径；
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

#### Scenario: root 对自己行危险操作被拒

- **WHEN** root 对自己的账户提交降权、禁用或删除
- **THEN** 服务端显式拒绝（40x）且状态不变，前端对自己行的对应控件呈
  禁用态

#### Scenario: root 仍可改自己密码

- **WHEN** root 为自己重置密码
- **THEN** 该路径保持可用（自锁防护不挡合法的自助改密）

#### Scenario: 重置密码踢会话

- **WHEN** root 重置某在线用户的密码
- **THEN** 该用户既有的会话 cookie 立即失效，后续 API 返回 401

#### Scenario: root 创建用户

- **WHEN** root 提交 `{username, password, role: "member"}`
- **THEN** 用户创建成功并可立即用该凭据登录，权限按 member 生效
