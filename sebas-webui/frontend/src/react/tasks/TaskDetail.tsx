import { Button, Tag, Typography } from 'antd'
import type { BeadsIssue, SessionRow } from '../../api/client.js'
import { renderMarkdown } from '../../components/markdown.js'
import { relativeFromIso, reviewerOf } from './principals.js'
import { ACTION_LABEL, availableActions, isAgentCaller, type TaskActionName, type TaskViewer } from './actions.js'
import { taskColumn } from './task-store.js'
import { CommentStream } from './CommentStream.js'
import { LinkedSessionList } from './LinkedSessionList.js'
import styles from './tasks.module.css'

/** 看板列 → 详情页状态文案。 */
export function taskStatusText(issue: BeadsIssue): string {
  switch (taskColumn(issue)) {
    case 'unassigned':
      return '待认领'
    case 'in_progress':
      return '进行中'
    case 'in_review':
      return '评审中'
    case 'done':
      return '已完成'
    case 'cancelled':
      return '已取消'
  }
}

export interface TaskDetailProps {
  issue: BeadsIssue
  agentIds: ReadonlySet<string>
  viewer: TaskViewer
  /** 该任务关联的会话（最新在前）。 */
  sessions: SessionRow[]
  loading: boolean
  actionBusy: boolean
  actionError: string | null
  onBack: () => void
  onAction: (action: TaskActionName) => void
  onComment: (text: string) => Promise<void>
  onOpenSession: (encodedKey: string) => void
  onStartSession: () => void
  /** 只读身份（viewer 角色）：不发起会话、不发表评论。 */
  canAct: boolean
}

/** 任务详情页（agent-task-center 8.1/8.2/8.5）：GitHub issue 式双栏。 */
export function TaskDetail({
  issue,
  agentIds,
  viewer,
  sessions,
  loading,
  actionBusy,
  actionError,
  onBack,
  onAction,
  onComment,
  onOpenSession,
  onStartSession,
  canAct,
}: TaskDetailProps) {
  const actions = availableActions(issue, viewer)
  const assignee = (issue.assignee ?? '').trim()
  const reviewer = reviewerOf(issue)
  const description = (issue.description ?? '').trim()

  return (
    <div className={styles.detail} data-testid="task-detail">
      <header className={styles.detailHead}>
        <Button size="small" onClick={onBack} data-testid="task-detail-back">
          ← 看板
        </Button>
        <Typography.Text code className={styles.taskId} title={issue.id}>
          {issue.id}
        </Typography.Text>
        <Tag
          color={taskColumn(issue) === 'in_review' ? 'gold' : taskColumn(issue) === 'done' ? 'green' : undefined}
        >
          {taskStatusText(issue)}
        </Tag>
        {loading ? <Typography.Text type="secondary">加载中…</Typography.Text> : null}
      </header>

      {actionError !== null ? (
        <div className={styles.detailError} role="alert">
          {actionError}
        </div>
      ) : null}

      <div className={styles.detailGrid}>
        <main className={styles.detailMain}>
          <Typography.Title level={3} className={styles.detailTitle}>
            {issue.title}
          </Typography.Title>
          {description !== '' ? (
            <div
              className={styles.markdown}
              data-testid="task-description"
              dangerouslySetInnerHTML={{ __html: renderMarkdown(description) }}
            />
          ) : (
            <Typography.Text type="secondary">（无描述）</Typography.Text>
          )}
          <CommentStream
            comments={issue.comments ?? []}
            agentIds={agentIds}
            canPost={canAct}
            onSubmit={onComment}
          />
        </main>

        <aside className={styles.detailSide} aria-label="任务元信息与操作">
          <section className={styles.sideBlock} data-testid="task-metadata">
            <Typography.Title level={5} className={styles.sectionTitle}>
              元信息
            </Typography.Title>
            <dl className={styles.metaList}>
              <dt>状态</dt>
              <dd>{taskStatusText(issue)}</dd>
              <dt>认领人</dt>
              <dd>
                {assignee === '' ? (
                  '未认领'
                ) : (
                  <>
                    {assignee}
                    {isAgentCaller({ ...viewer, username: assignee }) ? <Tag className={styles.chip}>agent</Tag> : null}
                  </>
                )}
              </dd>
              <dt>评审人</dt>
              <dd>{reviewer ?? '未指定'}</dd>
              <dt>创建</dt>
              <dd>{relativeFromIso(issue.created_at)}</dd>
              <dt>更新</dt>
              <dd>{relativeFromIso(issue.updated_at)}</dd>
              <dt>标签</dt>
              <dd>{issue.labels.length === 0 ? '无' : issue.labels.join('、')}</dd>
            </dl>
          </section>

          <section className={styles.sideBlock} data-testid="task-actions" aria-label="任务操作">
            {actions.length === 0 ? (
              <Typography.Text type="secondary">
                {canAct ? '当前状态无可用操作' : '只读身份，无操作权限'}
              </Typography.Text>
            ) : (
              <div className={styles.actionRow}>
                {actions.map((action) => (
                  <Button
                    key={action}
                    size="small"
                    loading={actionBusy}
                    danger={action === 'cancel'}
                    type={action === 'close' ? 'primary' : 'default'}
                    data-testid={`task-action-${action}`}
                    onClick={() => onAction(action)}
                  >
                    {ACTION_LABEL[action]}
                  </Button>
                ))}
              </div>
            )}
            {canAct ? (
              <Button
                block
                size="small"
                className={styles.startSession}
                data-testid="task-start-session"
                onClick={onStartSession}
              >
                开一轮会话推进此任务
              </Button>
            ) : null}
          </section>

          <LinkedSessionList sessions={sessions} onOpen={onOpenSession} />
        </aside>
      </div>
    </div>
  )
}
