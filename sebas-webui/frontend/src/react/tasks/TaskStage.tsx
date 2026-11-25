import { useEffect, useMemo, useState } from 'react'
import { Alert, Button, Spin } from 'antd'
import { errorText, type BeadsIssue, type SessionRow } from '../../api/client.js'
import {
  actionForDrop,
  defaultTaskActions,
  isReadOnlyViewer,
  runTaskAction,
  type TaskActionApi,
  type TaskActionName,
  type TaskViewer,
} from './actions.js'
import { TaskBoard } from './TaskBoard.js'
import { TaskDetail } from './TaskDetail.js'
import { TaskStore, type TaskColumn } from './task-store.js'
import { useStore } from '../state/use-store.js'
import styles from './tasks.module.css'

export interface TaskStageProps {
  projectId: string
  store: TaskStore
  viewer: TaskViewer
  /** 项目内的会话行（关联会话列表按 `task_id` 过滤）。 */
  sessions: SessionRow[]
  openTaskId: string | null
  onOpenTask: (taskId: string) => void
  onCloseTask: () => void
  onOpenSession: (encodedKey: string) => void
  onStartSession: (task: { id: string; title: string }) => void
  actions?: TaskActionApi
}

/** stage 的「任务」视图（agent-task-center 7.x/8.x）：看板 ⇄ 详情，持有
 *  store 订阅生命周期与动作派发。 */
export function TaskStage({
  projectId,
  store,
  viewer,
  sessions,
  openTaskId,
  onOpenTask,
  onCloseTask,
  onOpenSession,
  onStartSession,
  actions = defaultTaskActions,
}: TaskStageProps) {
  const project = useStore(store.store, (state) => state.projects[projectId])
  const tasks = project?.tasks ?? []
  const loading = project?.loading ?? false
  const storeError = project?.error ?? null

  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)
  const [actionBusy, setActionBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  // 面板进入 → 订阅 + 载入；离开 → 取消订阅（后端事件泵随接收者消失退出）。
  useEffect(() => {
    void store.ensure(projectId)
    return () => {
      void store.release(projectId)
    }
  }, [store, projectId])

  // 详情页需要评论正文：列表快照不带 comments，打开时按 id 拉一次完整快照
  // （这是唯一允许的 show 调用；WS 更新仍直接替换快照）。
  useEffect(() => {
    setActionError(null)
    setDetailError(null)
    if (openTaskId === null) return
    let cancelled = false
    setDetailLoading(true)
    // 先等在途的全量载入收尾，再拉详情：反过来的话列表快照会覆盖掉刚拉到的
    // 评论正文（列表不带 comments）。
    void store
      .ensure(projectId)
      .then(() => actions.show(projectId, openTaskId))
      .then((issue) => {
        if (!cancelled) store.upsert(projectId, issue)
      })
      .catch((failure: unknown) => {
        if (!cancelled) setDetailError(errorText(failure))
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [actions, projectId, openTaskId, store])

  const sessionCounts = useMemo(() => {
    const counts: Record<string, number> = {}
    for (const session of sessions) {
      const taskId = (session.task_id ?? '').trim()
      if (taskId === '') continue
      counts[taskId] = (counts[taskId] ?? 0) + 1
    }
    return counts
  }, [sessions])

  const issue = openTaskId === null ? null : tasks.find((task) => task.id === openTaskId) ?? null
  const linkedSessions = useMemo(() => {
    if (issue === null) return []
    return sessions
      .filter((session) => (session.task_id ?? '') === issue.id)
      .sort((a, b) => b.last_active_unix - a.last_active_unix)
  }, [issue, sessions])

  const canAct = !isReadOnlyViewer(viewer)

  async function onMoveTask(card: BeadsIssue, target: TaskColumn): Promise<void> {
    const action = actionForDrop(target)
    if (action === null) return
    const updated = await runTaskAction(actions, action, projectId, card)
    store.upsert(projectId, updated)
  }

  async function perform(action: TaskActionName): Promise<void> {
    if (issue === null || actionBusy) return
    setActionBusy(true)
    setActionError(null)
    try {
      const updated = await runTaskAction(actions, action, projectId, issue)
      store.upsert(projectId, updated)
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setActionBusy(false)
    }
  }

  async function comment(text: string): Promise<void> {
    if (issue === null) return
    await actions.comment(projectId, issue.id, text)
    // 评论返回后重取完整快照，流里立刻出现新条目（spec「appears in the stream」）。
    const refreshed = await actions.show(projectId, issue.id)
    store.upsert(projectId, refreshed)
  }

  if (openTaskId !== null) {
    if (issue === null) {
      return (
        <div className={styles.detailLoading} data-testid="task-detail-loading">
          {detailError !== null ? (
            <Alert
              type="error"
              showIcon
              role="alert"
              title={detailError}
              action={<Button size="small" onClick={onCloseTask}>返回看板</Button>}
            />
          ) : (
            <Spin />
          )}
        </div>
      )
    }
    return (
      <TaskDetail
        issue={issue}
        agentIds={viewer.agentIds}
        viewer={viewer}
        sessions={linkedSessions}
        loading={detailLoading}
        actionBusy={actionBusy}
        actionError={actionError}
        onBack={onCloseTask}
        onAction={(action) => void perform(action)}
        onComment={comment}
        onOpenSession={onOpenSession}
        onStartSession={() => onStartSession({ id: issue.id, title: issue.title })}
        canAct={canAct}
      />
    )
  }

  return (
    <TaskBoard
      tasks={tasks}
      loading={loading}
      error={storeError}
      agentIds={viewer.agentIds}
      viewer={viewer}
      sessionCounts={sessionCounts}
      onOpen={onOpenTask}
      onMove={onMoveTask}
      onRefresh={() => void store.refresh(projectId)}
    />
  )
}
