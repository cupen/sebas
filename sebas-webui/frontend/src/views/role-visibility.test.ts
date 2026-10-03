// @vitest-environment jsdom
/**
 * role→入口可见性映射表单测（gate-agent-directory-writes 2.1）：四档角色
 * （+ null = 鉴权关闭宿主）× 四个受裁剪入口的**逐格钉死**——映射改动必须
 * 显式过这里，并与服务端 RBAC 矩阵（sebas-webui/src/rbac.rs 的
 * permission_matrix_matches_spec）同口径。防线在服务端 403 执法；本表只
 * 管呈现层「无权限的入口不呈现」。
 */
import { describe, expect, it } from 'vitest'
import {
  canCreateSessions,
  canControlServices,
  canManageAgents,
  canManageProviders,
  canManageSkills,
  canManageUsers,
} from './role-visibility.js'
import type { Role } from '../api/client.js'

/** 映射表的一格：role → 各入口是否呈现。 */
interface Row {
  role: Role | null
  users: boolean
  services: boolean
  agentsWrite: boolean
  newSession: boolean
}

/** spec 权限矩阵 + design「前端隐藏入口清单」的呈现层投影（逐格对照）。 */
const TABLE: readonly Row[] = [
  // auth=false 宿主（/api/auth/me 不带角色）：除 Users（无用户面可管理）
  // 外全部保持既有可用——「无 auth 一切照旧」。
  { role: null, users: false, services: true, agentsWrite: true, newSession: true },
  // root：全量。
  { role: 'root', users: true, services: true, agentsWrite: true, newSession: true },
  // admin：+settings.manage 档（agents 写）与 services，无用户管理。
  { role: 'admin', users: false, services: true, agentsWrite: true, newSession: true },
  // member：只有 sessions.write——agents 写入口隐藏，会话入口保留。
  { role: 'member', users: false, services: false, agentsWrite: false, newSession: true },
  // viewer：只读——写入口全部隐藏。
  { role: 'viewer', users: false, services: false, agentsWrite: false, newSession: false },
]

describe('role→入口可见性映射表（gate-agent-directory-writes 2.1）', () => {
  it('逐格钉死：每档角色的四个受裁剪入口与 spec 矩阵同口径', () => {
    for (const row of TABLE) {
      expect(canManageUsers(row.role), `users @ ${row.role}`).toBe(row.users)
      expect(canControlServices(row.role), `services @ ${row.role}`).toBe(row.services)
      expect(canManageAgents(row.role), `agentsWrite @ ${row.role}`).toBe(row.agentsWrite)
      expect(canCreateSessions(row.role), `newSession @ ${row.role}`).toBe(row.newSession)
    }
  })

  it('agents 写入口与 settings.manage 同档（root/admin），读不受影响', () => {
    // agents.manage 行引用 settings.manage 同档——谓词值必须逐角色相等。
    for (const role of [null, 'root', 'admin', 'member', 'viewer'] as const) {
      expect(canManageAgents(role as Role | null)).toBe(
        role === null || role === 'root' || role === 'admin',
      )
    }
  })

  it('「新建会话」入口只对 viewer 隐藏（sessions.write 不含 viewer）', () => {
    for (const role of [null, 'root', 'admin', 'member'] as const) {
      expect(canCreateSessions(role as Role | null)).toBe(true)
    }
    expect(canCreateSessions('viewer')).toBe(false)
  })

  // （fix-webui-qa-round11 2.2，A-1/D2）Skills 删除入口归 settings.manage 档：
  // 删除是改仓动作，与服务端 DELETE /api/skills/{name} 的 403 执法同键——
  // 呈现层对 member/viewer 不渲染删除控件；auth=false（null）保持既有可用。
  it('skills 删除入口与 settings.manage 同档（root/admin/null），A-1 逐格钉死', () => {
    for (const role of [null, 'root', 'admin'] as const) {
      expect(canManageSkills(role as Role | null)).toBe(true)
    }
    for (const role of ['member', 'viewer'] as const) {
      expect(canManageSkills(role)).toBe(false)
    }
    // 与 provider 变更面同一权限键——两谓词逐角色取值一致（同一矩阵行）。
    for (const role of [null, 'root', 'admin', 'member', 'viewer'] as const) {
      expect(canManageSkills(role as Role | null)).toBe(canManageProviders(role as Role | null))
    }
  })
})
