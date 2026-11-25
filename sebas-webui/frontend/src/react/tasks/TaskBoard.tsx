import { useMemo, useState } from 'react'
import { Alert, Button, Empty, Spin } from 'antd'
import { errorText, type BeadsIssue } from '../../api/client.js'
import { canDrop, type TaskViewer } from './actions.js'
import { taskColumn, type TaskColumn } from './task-store.js'
import { TaskCard } from './TaskCard.js'
import styles from './tasks.module.css'

const ACTIVE_COLUMNS: Array<{ key: Extract<TaskColumn, 'unassigned' | 'in_progress' | 'in_review'>; label: string; hint: string }> = [
  { key: 'unassigned', label: '未认领', hint: '待认领的公共队列' },
  { key: 'in_progress', label: '进行中', hint: '已认领，正在推进' },
  { key: 'in_review', label: '评审中', hint: '等待评审与关闭' },
]

export interface TaskBoardProps {
  tasks: BeadsIssue[]
  loading: boolean
  error: string | null
  agentIds: ReadonlySet<string>
  viewer: TaskViewer
  /** beads issue id → 关联会话数。 */
  sessionCounts: Record<string, number>
  onOpen: (taskId: string) => void
  /** 拖动落列：由父层执行 HTTP 动作并更新 store。 */
  onMove: (issue: BeadsIssue, target: TaskColumn) => Promise<void>
  onRefresh: () => void
}

/** 三列看板 + 折叠 Done/Cancelled（agent-task-center 7.2/7.4）。 */
export function TaskBoard({
  tasks,
  loading,
  error,
  agentIds,
  viewer,
  sessionCounts,
  onOpen,
  onMove,
  onRefresh,
}: TaskBoardProps) {
  const [dragging, setDragging] = useState<string | null>(null)
  const [dropError, setDropError] = useState<string | null>(null)
  const [doneOpen, setDoneOpen] = useState(false)
  const [hoverColumn, setHoverColumn] = useState<TaskColumn | null>(null)

  const grouped = useMemo(() => {
    const buckets: Record<TaskColumn, BeadsIssue[]> = {
      unassigned: [],
      in_progress: [],
      in_review: [],
      done: [],
      cancelled: [],
    }
    for (const issue of tasks) buckets[taskColumn(issue)].push(issue)
    const byUpdated = (a: BeadsIssue, b: BeadsIssue) =>
      (b.updated_at ?? '').localeCompare(a.updated_at ?? '')
    for (const key of Object.keys(buckets) as TaskColumn[]) buckets[key].sort(byUpdated)
    return buckets
  }, [tasks])

  async function drop(target: TaskColumn): Promise<void> {
    setHoverColumn(null)
    const id = dragging
    setDragging(null)
    if (id === null) return
    const issue = tasks.find((task) => task.id === id)
    if (issue === undefined) return
    if (taskColumn(issue) === target) return
    if (!canDrop(issue, target, viewer)) {
      setDropError(target === 'done' ? '没有关闭该任务的权限（需人身份或指定 reviewer）' : '当前身份不能执行该操作')
      return
    }
    setDropError(null)
    try {
      await onMove(issue, target)
    } catch (failure) {
      setDropError(errorText(failure))
    }
  }

  const dragIssue = dragging === null ? null : tasks.find((task) => task.id === dragging) ?? null

  return (
    <div className={styles.board} data-testid="task-board">
      <div className={styles.boardHead}>
        <span className={styles.boardHeadText}>任务看板</span>
        <span className={styles.spacer} />
        {loading ? <Spin size="small" /> : null}
        <Button size="small" onClick={() => onRefresh()} data-testid="task-refresh">
          刷新
        </Button>
      </div>
      {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
      {dropError !== null ? (
        <Alert
          type="warning"
          showIcon
          title={dropError}
          role="alert"
          closable={{ onClose: () => setDropError(null) }}
        />
      ) : null}

      <div className={styles.columns}>
        {ACTIVE_COLUMNS.map((column) => (
          <section
            key={column.key}
            className={`${styles.column} ${hoverColumn === column.key ? styles.columnHot : ''}`}
            data-testid={`task-column-${column.key}`}
            aria-label={column.label}
            onDragOver={(event) => {
              if (dragging === null) return
              event.preventDefault()
              setHoverColumn(column.key)
            }}
            onDragLeave={() => setHoverColumn((current) => (current === column.key ? null : current))}
            onDrop={(event) => {
              event.preventDefault()
              void drop(column.key)
            }}
          >
            <header className={styles.columnHead}>
              <span className={styles.columnTitle}>{column.label}</span>
              <span className={styles.columnCount} data-testid={`task-count-${column.key}`}>
                {grouped[column.key].length}
              </span>
            </header>
            <p className={styles.columnHint}>{column.hint}</p>
            <div className={styles.columnBody}>
              {grouped[column.key].length === 0 ? (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无任务" />
              ) : (
                grouped[column.key].map((issue) => (
                  <TaskCard
                    key={issue.id}
                    issue={issue}
                    agentIds={agentIds}
                    sessionCount={sessionCounts[issue.id] ?? 0}
                    onOpen={onOpen}
                    draggable
                    onDragStart={setDragging}
                    onDragEnd={() => {
                      setDragging(null)
                      setHoverColumn(null)
                    }}
                  />
                ))
              )}
            </div>
          </section>
        ))}
      </div>

      <section
        className={`${styles.collapsed} ${hoverColumn === 'done' ? styles.columnHot : ''}`}
        data-testid="task-column-done"
        onDragOver={(event) => {
          if (dragIssue === null) return
          event.preventDefault()
          setHoverColumn('done')
        }}
        onDragLeave={() => setHoverColumn((current) => (current === 'done' ? null : current))}
        onDrop={(event) => {
          event.preventDefault()
          void drop('done')
        }}
      >
        <button
          type="button"
          className={styles.collapsedHead}
          aria-expanded={doneOpen}
          onClick={() => setDoneOpen((open) => !open)}
        >
          <span aria-hidden="true">{doneOpen ? '▾' : '▸'}</span>
          <span>已完成 / 已取消</span>
          <span className={styles.columnCount}>
            {grouped.done.length + grouped.cancelled.length}
          </span>
          {dragIssue !== null ? <span className={styles.dropHint}>拖到此处标记完成</span> : null}
        </button>
        {doneOpen ? (
          <div className={styles.collapsedBody}>
            {[...grouped.done, ...grouped.cancelled].map((issue) => (
              <TaskCard
                key={issue.id}
                issue={issue}
                agentIds={agentIds}
                sessionCount={sessionCounts[issue.id] ?? 0}
                onOpen={onOpen}
                draggable={false}
              />
            ))}
          </div>
        ) : null}
      </section>
    </div>
  )
}
