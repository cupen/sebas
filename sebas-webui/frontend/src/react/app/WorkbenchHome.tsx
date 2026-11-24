import { useEffect, useMemo, type ReactNode } from 'react'
import { Button, Card, Empty, Spin, Tag, Typography } from 'antd'
import { api, type Role } from '../../api/client.js'
import { armOpeningSeam, peekOpeningSeam, readAnchorCount } from '../../views/unread-cursor.js'
import { ConversationStore } from '../state/conversation-store.js'
import type { SessionStore } from '../state/session-store.js'
import { useStore } from '../state/use-store.js'
import { ConversationPane } from '../conversation/ConversationPane.js'
import { TaskStage } from '../tasks/TaskStage.js'
import type { TaskStore } from '../tasks/task-store.js'
import type { StageTab } from '../tasks/stage-tabs.js'
import { sessionLabel, statusLabel } from './labels.js'
import styles from './workbench.module.css'

export type { StageTab }

const NO_AGENTS: ReadonlySet<string> = new Set()

export interface WorkbenchHomeProps {
  sessionKey: string | null
  sessions: SessionStore
  selectedProjectId: string | null
  canCreate: boolean
  onRequestCreate: () => void
  onFocus: (key: string) => void
  /** 当前项目的 stage 视图（缺省 = 会话，向后兼容旧调用面）。 */
  tab?: StageTab
  onTabChange?: (tab: StageTab) => void
  /** 任务面板 store（缺省 = 不渲染任务视图）。 */
  tasks?: TaskStore | null
  openTaskId?: string | null
  onOpenTask?: (taskId: string) => void
  onCloseTask?: () => void
  onStartTaskSession?: (task: { id: string; title: string }) => void
  username?: string | null
  role?: Role | null
  agentIds?: ReadonlySet<string>
}

/** Workbench landing, deep-link header and stage tabs (3.5 / agent-task-center 7.1). */
export function WorkbenchHome({
  sessionKey,
  sessions,
  selectedProjectId,
  canCreate,
  onRequestCreate,
  onFocus,
  tab = 'conversation',
  onTabChange,
  tasks = null,
  openTaskId = null,
  onOpenTask,
  onCloseTask,
  onStartTaskSession,
  username = null,
  role = null,
  agentIds = NO_AGENTS,
}: WorkbenchHomeProps) {
  const summary = useStore(sessions.store, (state) => state.summary)
  const projects = useStore(sessions.store, (state) => state.projects)
  const sessionRows = useStore(sessions.store, (state) => state.list?.recent_sessions ?? [])
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
  const taskTabActive = tab === 'tasks' && project !== null && tasks !== null
  const taskProjectId = sessionKey !== null ? project?.id ?? null : selectedProjectId

  const tabs = project !== null && onTabChange !== undefined ? (
    <nav className={styles.tabs} aria-label="视图切换" data-testid="stage-tabs">
      <button
        type="button"
        className={`${styles.tab} ${tab === 'conversation' ? styles.tabActive : ''}`}
        aria-pressed={tab === 'conversation'}
        data-testid="stage-tab-conversation"
        onClick={() => onTabChange('conversation')}
      >
        会话
      </button>
      <button
        type="button"
        className={`${styles.tab} ${tab === 'tasks' ? styles.tabActive : ''}`}
        aria-pressed={tab === 'tasks'}
        data-testid="stage-tab-tasks"
        onClick={() => onTabChange('tasks')}
      >
        任务
      </button>
    </nav>
  ) : null

  let conversationBody: ReactNode
  if (sessionKey !== null) {
    // Lit parity: the session surface exists only once its read model does.
    // Rendering the header (and an empty transcript) ahead of the snapshot
    // makes "0 turns" a legitimate observation of a session that has turns —
    // the placeholder becomes indistinguishable from real content.
    if (detail === null && conv.error === null) {
      conversationBody = (
        <div className={styles.page} data-testid="workbench-session-loading">
          <Spin />
        </div>
      )
    } else {
      const taskId = (detail?.task_id ?? '').trim()
      conversationBody = (
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
                {/* 回链只在会话详情头出现；rail 行不显示（density，9.1/9.2）。 */}
                {taskId !== '' && onOpenTask !== undefined ? (
                  <Button
                    type="link"
                    size="small"
                    className={styles.taskLink}
                    data-testid="session-task-link"
                    onClick={() => onOpenTask(taskId)}
                  >
                    Task {taskId}
                  </Button>
                ) : null}
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
  } else {
    conversationBody = (
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

  return (
    <div className={styles.stage} data-testid="workbench-stage">
      {tabs}
      {/*
        切到任务视图时会话面板保持挂载（hidden 只影响渲染，不断事件流）——
        spec「switching tabs does not affect sessions」：流式回合继续、焦点不变。
      */}
      <div hidden={taskTabActive}>{conversationBody}</div>
      {taskTabActive && tasks !== null && taskProjectId !== null ? (
        <TaskStage
          projectId={taskProjectId}
          store={tasks}
          viewer={{ username, role, agentIds }}
          sessions={sessionRows}
          openTaskId={openTaskId}
          onOpenTask={(taskId) => onOpenTask?.(taskId)}
          onCloseTask={() => onCloseTask?.()}
          onOpenSession={(key) => onFocus(key)}
          onStartSession={(task) => onStartTaskSession?.(task)}
        />
      ) : null}
    </div>
  )
}
