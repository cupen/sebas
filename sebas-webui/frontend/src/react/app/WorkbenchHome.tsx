import { useEffect, useMemo } from 'react'
import { Button, Card, Empty, Spin, Tag, Typography } from 'antd'
import { api } from '../../api/client.js'
import { armOpeningSeam, peekOpeningSeam, readAnchorCount } from '../../views/unread-cursor.js'
import { ConversationStore } from '../state/conversation-store.js'
import type { SessionStore } from '../state/session-store.js'
import { useStore } from '../state/use-store.js'
import { ConversationPane } from '../conversation/ConversationPane.js'
import { sessionLabel, statusLabel } from './labels.js'
import styles from './workbench.module.css'

export interface WorkbenchHomeProps {
  sessionKey: string | null
  sessions: SessionStore
  selectedProjectId: string | null
  canCreate: boolean
  onRequestCreate: () => void
  onFocus: (key: string) => void
}

/** Workbench landing and deep-link header (3.5). Full transcript/composer is phase 4. */
export function WorkbenchHome({
  sessionKey,
  sessions,
  selectedProjectId,
  canCreate,
  onRequestCreate,
  onFocus,
}: WorkbenchHomeProps) {
  const summary = useStore(sessions.store, (state) => state.summary)
  const projects = useStore(sessions.store, (state) => state.projects)
  const conversation = useMemo(
    () => new ConversationStore({ session: (key) => api.session(key) }),
    [],
  )
  const conv = useStore(conversation.store, (state) => state)
  // The rail's rename writes the shared session row; the focused header reads
  // the same row so it follows immediately instead of waiting for a detail
  // refetch (round3 D6: header and rail are one value, no reload).
  const focusedRow = useStore(
    sessions.store,
    (state) => state.list?.recent_sessions.find((row) => row.encoded_key === sessionKey) ?? null,
  )

  useEffect(() => {
    if (sessionKey === null) {
      conversation.invalidate()
      return
    }
    // Deep links bypass the rail's open path; arm the opening boundary here so
    // the transcript still draws the seam from the pre-write anchor instead of
    // reading a cursor that a previous visit may already have advanced.
    if (peekOpeningSeam(sessionKey) === undefined) {
      armOpeningSeam(sessionKey, readAnchorCount(sessionKey))
    }
    void conversation.open(sessionKey)
  }, [sessionKey, conversation])

  const project = projects.find((item) => item.id === selectedProjectId) ?? null
  const focused = summary?.active_session ?? null
  const detail = conv.detail

  if (sessionKey !== null) {
    // Lit parity: the session surface exists only once its read model does.
    // Rendering the header (and an empty transcript) ahead of the snapshot
    // makes "0 turns" a legitimate observation of a session that has turns —
    // the placeholder becomes indistinguishable from real content.
    if (detail === null && conv.error === null) {
      return (
        <div className={styles.page} data-testid="workbench-session-loading">
          <Spin />
        </div>
      )
    }
    return (
      <div className={styles.page} data-testid="workbench-session">
        <header className={styles.head}>
          <div>
            <Typography.Title level={3} className={styles.title}>
              {focusedRow
                ? sessionLabel(focusedRow)
                : detail
                  ? sessionLabel({
                      label: null,
                      prompt_preview:
                        detail.entries.find((entry) => entry.kind === 'prompt')?.content ?? null,
                      session_id_short: detail.session_id,
                      encoded_key: detail.encoded_key,
                    })
                  : '聚焦会话'}
            </Typography.Title>
            <div className={styles.keyRow}>
              <Typography.Text code className={styles.key} title={sessionKey}>
                {sessionKey}
              </Typography.Text>
              {detail ? <Tag>{statusLabel(detail.status_slug)}</Tag> : null}
              {detail?.current_model ? <Tag>{detail.current_model}</Tag> : null}
            </div>
          </div>
        </header>

        <ConversationPane
          sessionKey={sessionKey}
          conversation={conversation}
          reachability={summary?.reachability ?? null}
          canWrite={canCreate}
        />
      </div>
    )
  }

  return (
    <div className={styles.page} data-testid="workbench-home">
      <header className={styles.head}>
        <div>
          <Typography.Title level={3} className={styles.title}>
            {project ? project.name : '工作台'}
          </Typography.Title>
          <Typography.Paragraph type="secondary" className={styles.sub}>
            {project ? project.path : '从左侧选择项目与会话。'}
          </Typography.Paragraph>
        </div>
        {canCreate && project ? (
          <Button type="primary" onClick={onRequestCreate}>
            新建会话
          </Button>
        ) : null}
      </header>

      <div className={styles.stats}>
        <Card size="small" variant="borderless" className={styles.card}>
          <span className={styles.statValue}>{summary?.active_count ?? 0}</span>
          <span className={styles.statLabel}>活跃会话</span>
        </Card>
        <Card size="small" variant="borderless" className={styles.card}>
          <span className={styles.statValue}>{summary?.total_sessions ?? 0}</span>
          <span className={styles.statLabel}>会话总数</span>
        </Card>
        <Card size="small" variant="borderless" className={styles.card}>
          <span className={styles.statValue}>{summary?.uptime ?? '—'}</span>
          <span className={styles.statLabel}>运行时长</span>
        </Card>
      </div>

      <Card variant="borderless" className={styles.card} title="当前焦点">
        {focused ? (
          <div className={styles.focus}>
            <div>
              <Typography.Text strong>{sessionLabel({
                label: null,
                prompt_preview: null,
                session_id_short: focused.session_id,
                encoded_key: focused.encoded_key,
              })}</Typography.Text>
              <div className={styles.focusMeta}>
                {statusLabel(focused.status_slug)} · {focused.current_model ?? '模型未上报'}
              </div>
            </div>
            <Button onClick={() => onFocus(focused.encoded_key)}>打开会话</Button>
          </div>
        ) : (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有聚焦会话" />
        )}
      </Card>
    </div>
  )
}
