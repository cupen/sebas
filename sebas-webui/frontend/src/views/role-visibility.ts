/**
 * role→入口可见性映射表（gate-agent-directory-writes 2.1，design「前端隐藏
 * 入口清单」）：呈现层裁剪的唯一事实源——防线在服务端路由层（无权限的已
 * 认证请求 403），这里只决定「无权限的入口不呈现」（spec：呈现层优化，
 * 不构成防线）。
 *
 * `role = null` 的语义是「宿主未启用登录鉴权」（auth=false 的单机形态，
 * `/api/auth/me` 不带角色）：一切入口保持既有可用——映射表只在鉴权开启时
 * 收窄。四个谓词与 spec「RBAC 角色与权限执法」权限矩阵逐行对应：
 *
 * | 入口 | 权限档 | root | admin | member | viewer |
 * |---|---|---|---|---|---|
 * | Settings → Users 分区 | users.manage | ✓ | | | |
 * | Settings → Services 分区 | services.control | ✓ | ✓ | | |
 * | Agents 写入口（New/Edit/Delete） | agents.manage（settings.manage 档） | ✓ | ✓ | | |
 * | Provider/别名/默认选择写入口（fix-webui-qa-round10 3.2） | settings.manage | ✓ | ✓ | | |
 * | 「新建会话」入口（sessions.write） | sessions.write | ✓ | ✓ | ✓ | |
 *
 * （Users 分区在鉴权关闭的宿主上**不**因 null 放开：auth=false 没有用户
 * 面可管理，保持既有隐藏；其余三个入口在 auth=false 的单机形态照旧可用。）
 */

import type { Role } from '../api/client.js'

/** Settings → Users 分区（users.manage）：仅 root。 */
export function canManageUsers(role: Role | null): boolean {
  return role === 'root'
}

/** Settings → Services 分区（services.control）：root/admin。 */
export function canControlServices(role: Role | null): boolean {
  return role === null || role === 'root' || role === 'admin'
}

/**
 * Agents 分区写入口（agents.manage）：归 settings.manage 档（root/admin，
 * design：不新设独立权限位）；agent 目录**读**（列表/详情）保持登录门，
 * 任何已认证角色都可浏览。
 */
export function canManageAgents(role: Role | null): boolean {
  return role === null || role === 'root' || role === 'admin'
}

/**
 * Provider/别名/默认选择的写入口（fix-webui-qa-round10 3.2，C-DEF-02）：
 * 归 settings.manage 档（root/admin）——provider 携带上游凭据，属管理面
 * （spec「Provider and alias mutations are role-gated」）；服务端对变更面
 * 的 403 执法在 `required_permission` 中央表，这里只决定 member/viewer 的
 * 设置页不呈现写控件（新建/编辑/删除/设默认/清默认）。provider 列表**读**
 * 对全部登录角色保留。
 */
export function canManageProviders(role: Role | null): boolean {
  return role === null || role === 'root' || role === 'admin'
}

/** 「新建会话」入口（sessions.write）：viewer 之外全部可建。 */
export function canCreateSessions(role: Role | null): boolean {
  return role === null || role !== 'viewer'
}
