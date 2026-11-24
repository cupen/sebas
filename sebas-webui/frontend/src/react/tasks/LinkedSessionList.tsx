import { Empty, Tooltip, Typography } from 'antd'
import type { SessionRow } from '../../api/client.js'
import { relativeTime, sessionLabel, statusLabel } from '../app/labels.js'
import styles from './tasks.module.css'

export interface LinkedSessionListProps {
  /** 已按 `last_active_unix` 倒序（最新在前）。 */
  sessions: SessionRow[]
  onOpen: (encodedKey: string) => void
}

/** 关联会话列表（agent-task-center 8.4）：标题 / agent / 相位 / 时间，
 *  点击切到会话视图并聚焦该会话。 */
export function LinkedSessionList({ sessions, onOpen }: LinkedSessionListProps) {
  return (
    <section className={styles.linkedSessions} data-testid="task-linked-sessions">
      <Typography.Title level={5} className={styles.sectionTitle}>
        关联会话
      </Typography.Title>
      {sessions.length === 0 ? (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未有会话推进此任务" />
      ) : (
        <div className={styles.sessionList}>
          {sessions.map((session) => (
            <button
              key={session.encoded_key}
              type="button"
              className={styles.sessionRow}
              data-testid={`task-session-${session.encoded_key}`}
              onClick={() => onOpen(session.encoded_key)}
            >
              <span className={styles.sessionTitle} title={sessionLabel(session)}>
                {sessionLabel(session)}
              </span>
              <span className={styles.sessionMeta}>
                <Tooltip title="执行后端">{session.agent_kind ?? '默认'}</Tooltip>
                <span className={styles.sessionPhase}>{statusLabel(session.status_slug)}</span>
                <Typography.Text type="secondary" className={styles.time}>
                  {relativeTime(session.last_active_unix, Date.now())}
                </Typography.Text>
              </span>
            </button>
          ))}
        </div>
      )}
    </section>
  )
}
