/**
 * 任务动作缝（agent-task-center 7.4/8.2）：把看板列/详情按钮映射成 HTTP 调用。
 *
 * 权限裁决在 WebUI API 层（design D10），前端只负责「按状态与调用者身份条件
 * 渲染」以及把拒绝原因原样呈现——不做第二套规则。
 */

import { api, type BeadsComment, type BeadsIssue, type Role } from '../../api/client.js'
import { reviewerOf } from './principals.js'
import { taskColumn, type TaskColumn } from './task-store.js'

export type TaskActionName = 'claim' | 'release' | 'review' | 'close' | 'cancel'

export const ACTION_LABEL: Record<TaskActionName, string> = {
  claim: '认领',
  release: '释放',
  review: '进入评审',
  close: '完成',
  cancel: '取消',
}

export interface TaskActionApi {
  claim: (projectId: string, taskId: string) => Promise<BeadsIssue>
  release: (projectId: string, taskId: string) => Promise<BeadsIssue>
  review: (projectId: string, taskId: string) => Promise<BeadsIssue>
  close: (projectId: string, taskId: string, reason?: string | null) => Promise<BeadsIssue>
  comment: (projectId: string, taskId: string, text: string) => Promise<BeadsComment>
  show: (projectId: string, taskId: string) => Promise<BeadsIssue>
}

export const defaultTaskActions: TaskActionApi = {
  claim: (projectId, taskId) => api.taskClaim(projectId, taskId),
  release: (projectId, taskId) => api.taskRelease(projectId, taskId),
  review: (projectId, taskId) => api.taskReview(projectId, taskId),
  close: (projectId, taskId, reason) => api.taskClose(projectId, taskId, reason),
  comment: (projectId, taskId, text) => api.taskComment(projectId, taskId, text),
  show: (projectId, taskId) => api.taskShow(projectId, taskId),
}

/** 复合视角的调用者身份（GUI 是人类，但名字可能恰好是 agent kind）。 */
export interface TaskViewer {
  username: string | null
  role: Role | null
  agentIds: ReadonlySet<string>
}

const READ_ONLY_ROLES: ReadonlySet<Role> = new Set<Role>(['viewer'])

export function isReadOnlyViewer(viewer: TaskViewer): boolean {
  return viewer.role !== null && READ_ONLY_ROLES.has(viewer.role)
}

/** GUI 调用者名字落在 agent 身份集合 → 按 agent 的 close 闸门要求 reviewer。 */
export function isAgentCaller(viewer: TaskViewer): boolean {
  const name = (viewer.username ?? '').trim()
  return name !== '' && viewer.agentIds.has(name)
}

export function mayClose(issue: BeadsIssue, viewer: TaskViewer): boolean {
  if (isReadOnlyViewer(viewer)) return false
  if (!isAgentCaller(viewer)) return true
  return reviewerOf(issue) === (viewer.username ?? '').trim()
}

/** 当前状态 + 身份下应渲染的动作按钮（spec「Action buttons follow state and identity」）。 */
export function availableActions(issue: BeadsIssue, viewer: TaskViewer): TaskActionName[] {
  if (isReadOnlyViewer(viewer)) return []
  switch (taskColumn(issue)) {
    case 'unassigned':
      return ['claim']
    case 'in_progress': {
      const actions: TaskActionName[] = []
      actions.push((issue.assignee ?? '').trim() === '' ? 'claim' : 'release')
      actions.push('review')
      return actions
    }
    case 'in_review':
      return mayClose(issue, viewer) ? ['close', 'cancel'] : ['cancel']
    case 'done':
    case 'cancelled':
      return []
  }
}

/**
 * 拖到某列对应的动作：`unassigned ← release`、`in_progress ← claim`、
 * `in_review ← review`、`done ← close`；Done/Cancelled 不接收拖动。
 */
export function actionForDrop(target: TaskColumn): TaskActionName | null {
  switch (target) {
    case 'unassigned':
      return 'release'
    case 'in_progress':
      return 'claim'
    case 'in_review':
      return 'review'
    case 'done':
      return 'close'
    case 'cancelled':
      return null
  }
}

export async function runTaskAction(
  seam: TaskActionApi,
  action: TaskActionName,
  projectId: string,
  issue: BeadsIssue,
): Promise<BeadsIssue> {
  switch (action) {
    case 'claim':
      return seam.claim(projectId, issue.id)
    case 'release':
      return seam.release(projectId, issue.id)
    case 'review':
      return seam.review(projectId, issue.id)
    case 'close':
      return seam.close(projectId, issue.id)
    case 'cancel':
      return seam.close(projectId, issue.id, 'cancelled')
  }
}

/** 拖拽落点是否合法（Done 需要 close 权限；其余按动作可见性）。 */
export function canDrop(
  issue: BeadsIssue,
  target: TaskColumn,
  viewer: TaskViewer,
): boolean {
  if (isReadOnlyViewer(viewer)) return false
  const action = actionForDrop(target)
  if (action === null) return false
  if (action === 'close' && !mayClose(issue, viewer)) return false
  return true
}
