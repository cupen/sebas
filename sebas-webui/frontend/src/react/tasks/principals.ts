/**
 * 任务协作方身份判定（agent-task-center 7.3/8.3）.
 *
 * beads 的 assignee/author 是自由字符串；前端用 agent 目录（`/api/agents`）
 * 区分「人」与「agent kind」——人类走首字母圆头像，agent 走类型图标 + `agent`
 * chip。
 */

import type { BeadsIssue } from '../../api/client.js'

export function isAgentPrincipal(name: string | null | undefined, agentIds: ReadonlySet<string>): boolean {
  const value = (name ?? '').trim()
  return value !== '' && agentIds.has(value)
}

/** 头像首字（人类）：大写首字符。 */
export function initialOf(name: string | null | undefined): string {
  const value = (name ?? '').trim()
  return value === '' ? '?' : value.slice(0, 1).toUpperCase()
}

/** issue 的指定 reviewer（存 metadata）。 */
export function reviewerOf(issue: BeadsIssue): string | null {
  const reviewer = issue.metadata?.reviewer
  return typeof reviewer === 'string' && reviewer.trim() !== '' ? reviewer.trim() : null
}

/** `author` 的展示归类：人 / agent / 未知（未署名）。 */
export type PrincipalKind = 'human' | 'agent' | 'unknown'

export function principalKind(
  name: string | null | undefined,
  agentIds: ReadonlySet<string>,
): PrincipalKind {
  const value = (name ?? '').trim()
  if (value === '') return 'unknown'
  return isAgentPrincipal(value, agentIds) ? 'agent' : 'human'
}

/** `updated_at`（RFC3339）→ 相对时间；解析失败回原串/未知。 */
export function relativeFromIso(iso: string | null | undefined, now = Date.now()): string {
  const value = (iso ?? '').trim()
  if (value === '') return '未知'
  const stamp = Date.parse(value)
  if (Number.isNaN(stamp)) return value
  const seconds = Math.max(0, Math.floor((now - stamp) / 1000))
  if (seconds < 60) return `${seconds} 秒前`
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)} 小时前`
  return `${Math.floor(seconds / 86_400)} 天前`
}
