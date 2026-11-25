import { useMemo } from 'react'
import { Avatar, Tag, Tooltip, Typography } from 'antd'
import type { BeadsIssue } from '../../api/client.js'
import { initialOf, isAgentPrincipal, relativeFromIso } from './principals.js'
import styles from './tasks.module.css'

export interface TaskCardProps {
  issue: BeadsIssue
  agentIds: ReadonlySet<string>
  /** 经 `session_map.task_id` 关联的会话数。 */
  sessionCount: number
  onOpen: (taskId: string) => void
  /** Done/Cancelled 区不可拖动（spec）。 */
  draggable: boolean
  onDragStart?: (taskId: string) => void
  onDragEnd?: () => void
  now?: number
}

/** 看板卡片（agent-task-center 7.3）：标题两行截断 + tooltip、协作方头像、
 *  `agent` chip、评论数、会话数、相对更新时间。 */
export function TaskCard({
  issue,
  agentIds,
  sessionCount,
  onOpen,
  draggable,
  onDragStart,
  onDragEnd,
  now,
}: TaskCardProps) {
  const assignee = (issue.assignee ?? '').trim()
  const agentAssignee = isAgentPrincipal(assignee, agentIds)
  const commentCount = issue.comment_count ?? issue.comments?.length ?? 0
  const updated = useMemo(() => relativeFromIso(issue.updated_at, now ?? Date.now()), [issue.updated_at, now])

  const avatar = agentAssignee ? (
    <Tooltip title={`agent ${assignee}`}>
      <Avatar size={22} className={styles.avatarAgent} data-testid={`task-avatar-agent-${issue.id}`}>
        ⚙
      </Avatar>
    </Tooltip>
  ) : assignee !== '' ? (
    <Tooltip title={assignee}>
      <Avatar size={22} className={styles.avatarHuman}>
        {initialOf(assignee)}
      </Avatar>
    </Tooltip>
  ) : (
    <Tooltip title="未认领">
      <Avatar size={22} className={styles.avatarNone} aria-label="未认领" />
    </Tooltip>
  )

  return (
    <div
      className={styles.card}
      draggable={draggable}
      data-testid={`task-card-${issue.id}`}
      role="button"
      tabIndex={0}
      onDragStart={(event) => {
        if (!draggable) {
          event.preventDefault()
          return
        }
        event.dataTransfer.setData('text/task-id', issue.id)
        event.dataTransfer.effectAllowed = 'move'
        onDragStart?.(issue.id)
      }}
      onDragEnd={() => onDragEnd?.()}
      onClick={() => onOpen(issue.id)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onOpen(issue.id)
        }
      }}
    >
      <Tooltip title={issue.title} mouseEnterDelay={0.4}>
        <div className={styles.cardTitle} data-testid={`task-title-${issue.id}`}>
          {issue.title}
        </div>
      </Tooltip>
      <div className={styles.cardMeta}>
        {avatar}
        {assignee !== '' ? <span className={styles.assignee}>{assignee}</span> : <span className={styles.assignee}>未认领</span>}
        {agentAssignee ? <Tag className={styles.chip}>agent</Tag> : null}
        <span className={styles.spacer} />
        <Tooltip title="评论数">
          <span className={styles.count} data-testid={`task-comments-${issue.id}`}>
            💬 {commentCount}
          </span>
        </Tooltip>
        <Tooltip title="关联会话数">
          <span className={styles.count} data-testid={`task-sessions-${issue.id}`}>
            ▣ {sessionCount}
          </span>
        </Tooltip>
        <Typography.Text type="secondary" className={styles.time}>
          {updated}
        </Typography.Text>
      </div>
    </div>
  )
}
