import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from 'react'
import { Button, Drawer, Dropdown, Modal, Tag } from 'antd'
import { api, type Role, type SessionRow } from '../../api/client.js'
import { sharedWs } from '../../api/shared-ws.js'
import { notify, setFatal, setWsDown } from '../../notify.js'
import { canCreateSessions } from '../../views/role-visibility.js'
import {
  armOpeningSeam,
  readAnchorCount,
} from '../../views/unread-cursor.js'
import { observeTurnFrame, setFocusedSession } from '../../views/turn-notify.js'
import type { AuthStore } from '../state/auth-store.js'
import type { SessionStore } from '../state/session-store.js'
import { useStore } from '../state/use-store.js'
import { useRoute, navigate } from '../boundary/use-route.js'
import { readRailCollapsed, writeRailCollapsed } from '../boundary/preferences.js'
import { loadRailWidth, saveRailWidth } from '../../views/split-persist.js'
import { NoticeHost } from './NoticeHost.js'
import { ProjectRail } from './ProjectRail.js'
import { SessionCreateModal } from './SessionCreateModal.js'
import { UsagePage } from '../pages/UsagePage.js'
import { SessionsPage } from '../pages/SessionsPage.js'
import { WorkbenchHome, type StageTab } from './WorkbenchHome.js'
import { SettingsPage } from '../pages/SettingsPage.js'
import { TaskStore } from '../tasks/task-store.js'
import { tabForProject, withProjectTab, type StageTabState } from '../tasks/stage-tabs.js'
import { statusLabel } from './labels.js'
import styles from './shell.module.css'

export interface ShellProps {
  auth: AuthStore
  sessions: SessionStore
  role: Role | null
  username: string | null
}

const ROLE_LABEL: Record<Role, string> = {
  root: 'root',
  admin: 'admin',
  member: 'member',
  viewer: '只读',
}

/** Blueprint 1440: the rail is adjustable in 240–360px and collapsible. */
const RAIL_MIN_PX = 240
const RAIL_MAX_PX = 360
const RAIL_DEFAULT_PX = 280

function clampRail(px: number): number {
  if (!Number.isFinite(px)) return RAIL_DEFAULT_PX
  return Math.min(RAIL_MAX_PX, Math.max(RAIL_MIN_PX, Math.round(px)))
}

function initialRailWidth(): number {
  const stored = loadRailWidth()
  return stored === null ? RAIL_DEFAULT_PX : clampRail(stored)
}

/**
 * `permission.requested` may carry a ChannelKey JSON text on some backends
 * while the rail keys are the canonical encoded form. Match either shape.
 */
export function resolveSessionKey(raw: string, rows: readonly SessionRow[]): string {
  if (rows.some((row) => row.encoded_key === raw)) return raw
  let reference: string | null = null
  try {
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed === 'object' && parsed !== null) {
      const value = (parsed as { reference?: unknown }).reference
      if (typeof value === 'string') reference = value
    }
  } catch {
    /* already an encoded key */
  }
  const hit = reference
    ? rows.find((row) => row.encoded_key.endsWith(reference!))
    : undefined
  return hit?.encoded_key ?? raw
}

export function Shell({ auth, sessions, role, username }: ShellProps) {
  const { pathname, route } = useRoute()
  const summary = useStore(sessions.store, (state) => state.summary)
  const list = useStore(sessions.store, (state) => state.list)
  // 横幅聚焦标签与 rail 行同源（recent_sessions 的 WS 实时 status_slug）：
  // summary 投影刷新滞后，曾出现「横幅=运行中 / 转录=完成」长窗口失同步
  //（视觉验收 DEF-1）。summary 仅在会话不在 recent 窗口时回退。
  const focusedRow =
    route.kind === 'session'
      ? (list?.recent_sessions ?? []).find((row) => row.encoded_key === route.key)
      : null
  const focusStatusSlug =
    focusedRow?.status_slug ?? summary?.active_session?.status_slug ?? null
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null)
  const [drawerOpen, setDrawerOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [narrow, setNarrow] = useState(false)
  const [compactRail, setCompactRail] = useState(false)
  const [railWidth, setRailWidth] = useState(initialRailWidth)
  const [railCollapsed, setRailCollapsed] = useState(readRailCollapsed)
  const [resizingRail, setResizingRail] = useState(false)
  const railWidthRef = useRef(railWidth)
  const dragOrigin = useRef<{ x: number; width: number } | null>(null)
  const settingsReturnFocus = useRef<HTMLElement | null>(null)
  const [connected, setConnected] = useState<boolean | null>(null)
  const [create, setCreate] = useState<{
    open: boolean
    projectId: string | null
    /** 任务详情页发起的创建：标题改写 + 请求携带 task_id。 */
    task: { id: string; title: string } | null
  }>({ open: false, projectId: null, task: null })
  // 任务中心（agent-task-center 7.1/9.1）：每项目各记一份 stage 视图；
  // 详情打开态记在 Shell，切 tab 不改变聚焦会话指针。
  const taskStore = useMemo(() => new TaskStore(), [])
  const [tabByProject, setTabByProject] = useState<StageTabState>({})
  const [openTask, setOpenTask] = useState<{ projectId: string; taskId: string } | null>(null)
  const [agentIds, setAgentIds] = useState<ReadonlySet<string>>(() => new Set())
  const canCreate = canCreateSessions(role)
  useEffect(() => {
    let cancelled = false
    api
      .agents()
      .then((response) => {
        if (!cancelled) setAgentIds(new Set(response.agents.map((agent) => agent.id)))
      })
      .catch(() => {
        /* agent 目录不可用时按「无 agent kind」渲染，任务面不受影响 */
      })
    return () => {
      cancelled = true
    }
  }, [])
  // Focus projection for turn-end notices: only the session actually open in
  // the conversation pane suppresses its own completion toast.
  useEffect(() => {
    setFocusedSession(route.kind === 'session' ? route.key : null)
    return () => setFocusedSession(null)
  }, [route])
  const projects = useStore(sessions.store, (state) => state.projects)
  const focusedProjectId =
    route.kind === 'session'
      ? ((list?.recent_sessions ?? []).find((row) => row.encoded_key === route.key)?.project_id ??
        null)
      : null
  const project =
    (route.kind === 'session'
      ? projects.find((item) => item.id === focusedProjectId)
      : undefined) ?? projects.find((item) => item.id === selectedProjectId)

  const openSession = useCallback(
    (key: string) => {
      if (!key) return
      setDrawerOpen(false)
      // Opening a session arms the unseen boundary with the anchor read BEFORE
      // any focus write (Lit `openSession` parity). Re-focus re-arms with the
      // CURRENT anchor: a stale arm would redraw a seam for content already
      // read. The transcript consumes it to draw the seam; the cursor itself
      // only advances when content is actually seen.
      armOpeningSeam(key, readAnchorCount(key))
      const projectId =
        (list?.recent_sessions ?? []).find((row) => row.encoded_key === key)?.project_id ?? null
      if (projectId !== null) {
        setSelectedProjectId(projectId)
        // 打开会话总是回到会话视图（回链/关联会话跳转的语义）。
        setTabByProject((current) => withProjectTab(current, projectId, 'conversation'))
      }
      navigate(`/sessions/${key}`)
    },
    [list],
  )

  const setStageTab = useCallback(
    (projectId: string | null) => (tab: StageTab) => {
      setTabByProject((current) => withProjectTab(current, projectId, tab))
    },
    [],
  )

  const openTaskDetail = useCallback((projectId: string | null, taskId: string) => {
    if (projectId === null) return
    setOpenTask({ projectId, taskId })
    setTabByProject((current) => withProjectTab(current, projectId, 'tasks'))
  }, [])

  const requestCreateForTask = useCallback(
    (projectId: string | null, task: { id: string; title: string }) => {
      setCreate({ open: true, projectId, task })
    },
    [],
  )

  const selectProject = useCallback(
    (id: string | null) => {
      setSelectedProjectId(id)
      // Displayed-project reconciliation: a focused session whose project no
      // longer matches the selected row must not linger by inertia.
      if (route.kind === 'session' && focusedProjectId !== id) {
        navigate('/')
      }
    },
    [route.kind, focusedProjectId],
  )

  const closeDrawer = useCallback(() => {
    setDrawerOpen(false)
    const delay =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
        ? 0
        : 350
    window.setTimeout(() => document.getElementById('nav-menu-toggle')?.focus(), delay)
  }, [])

  useEffect(() => {
    railWidthRef.current = railWidth
  }, [railWidth])

  const openSettings = useCallback(() => {
    const active = document.activeElement
    settingsReturnFocus.current = active instanceof HTMLElement ? active : null
    setDrawerOpen(false)
    setSettingsOpen(true)
  }, [])

  const closeSettings = useCallback(() => {
    setSettingsOpen(false)
  }, [])

  // Focus returns after the modal has finished unmounting; restoring earlier
  // makes the closing dialog steal the focus straight back. The narrow drawer
  // unmounts its own footer entry, so fall back to its visible menu trigger.
  const restoreSettingsFocus = useCallback(() => {
    const saved = settingsReturnFocus.current
    settingsReturnFocus.current = null
    const visible =
      saved !== null && saved.isConnected && saved.getClientRects().length > 0
        ? saved
        : document.getElementById('nav-menu-toggle')
    visible?.focus()
  }, [])

  const toggleRail = useCallback(() => {
    setRailCollapsed((current) => {
      writeRailCollapsed(!current)
      return !current
    })
  }, [])

  function beginRailResize(event: ReactPointerEvent<HTMLDivElement>): void {
    if (compactRail || railCollapsed) return
    event.preventDefault()
    dragOrigin.current = { x: event.clientX, width: railWidthRef.current }
    event.currentTarget.setPointerCapture(event.pointerId)
    setResizingRail(true)
  }

  function moveRailResize(event: ReactPointerEvent<HTMLDivElement>): void {
    const origin = dragOrigin.current
    if (origin === null) return
    const next = clampRail(origin.width + event.clientX - origin.x)
    railWidthRef.current = next
    setRailWidth(next)
  }

  function endRailResize(event: ReactPointerEvent<HTMLDivElement>): void {
    if (dragOrigin.current === null) return
    dragOrigin.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    setResizingRail(false)
    saveRailWidth(railWidthRef.current)
  }

  function resizeRailBy(delta: number): void {
    setRailWidth((current) => {
      const next = clampRail(current + delta)
      railWidthRef.current = next
      saveRailWidth(next)
      return next
    })
  }

  // Fatal vs. recovery bookkeeping lives outside the store: the notice layer
  // only holds the current slot, so the transition fact has to be observed
  // here (Lit app-shell applies reachability through the same two-slot model:
  // enter fatal → lock screen, leave fatal → clear + 「核心已恢复」info).
  const fatalWasActive = useRef(false)
  const applyFatal = useCallback((next: { kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'; cause?: string } | null) => {
    if (next === null) {
      if (fatalWasActive.current) {
        fatalWasActive.current = false
        notify({ level: 'info', message: '核心已恢复' })
      }
      setFatal(null)
      return
    }
    fatalWasActive.current = true
    setFatal(next)
  }, [])

  const probeReachability = useCallback(async () => {
    try {
      const payload = (await sharedWs.request('core.reachability.get')) as {
        ok: boolean
        kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'
        cause?: string
      }
      applyFatal(payload.ok ? null : { kind: payload.kind, cause: payload.cause })
    } catch {
      /* not connected yet; the ws-state listener covers the transition */
    }
  }, [applyFatal])

  useEffect(() => {
    const narrowQuery = window.matchMedia('(max-width: 600px)')
    const compactQuery = window.matchMedia('(max-width: 768px)')
    const updateNarrow = () => setNarrow(narrowQuery.matches)
    const updateCompact = () => setCompactRail(compactQuery.matches)
    updateNarrow()
    updateCompact()
    narrowQuery.addEventListener('change', updateNarrow)
    compactQuery.addEventListener('change', updateCompact)
    return () => {
      narrowQuery.removeEventListener('change', updateNarrow)
      compactQuery.removeEventListener('change', updateCompact)
    }
  }, [])

  useEffect(() => {
    sharedWs.setAuthGated(false)
    const off = sharedWs.subscribe((event) => {
      switch (event.type) {
        case 'session.created':
        case 'session.updated':
          sessions.applyPhaseFrame(event)
          // Non-focused turn endpoints surface through the notice layer; a
          // focused session settles in its transcript instead (round11 3.1).
          observeTurnFrame(event)
          break
        case 'session.removed':
          sessions.applyRemoved(event.session_id)
          if (window.location.pathname === `/sessions/${event.session_id}`) navigate('/')
          break
        case 'permission.requested':
          sessions.markWaiting(resolveSessionKey(event.session_id, list?.recent_sessions ?? []))
          break
        case 'session.pending_dropped': {
          const texts = event.dropped.map((item) => item.text).join('、')
          notify({
            level: 'warn',
            message: `以下待执行消息未被执行（会话已结束）：${texts}`,
          })
          break
        }
        case 'config.updated':
          void sessions.refresh()
          break
        case 'task_updated':
        case 'task_resync':
          // 任务面板的实时面（7.5）：事件载荷是完整 issue 快照，直接替换本地
          // 状态，不回查 `bd show`（store 负责缓冲未加载窗口）。
          taskStore.applyEvent(event)
          break
        case 'core.reachability':
          applyFatal(
            event.ok
              ? null
              : { kind: event.kind, cause: event.cause },
          )
          break
        default:
          break
      }
    })
    const onWsState = (event: Event) => {
      const next = (event as CustomEvent<{ connected: boolean }>).detail.connected
      setConnected(next)
      setWsDown(!next)
      if (next) void probeReachability()
    }
    const onRefetch = () => {
      void sessions.refresh()
    }
    window.addEventListener('sebas:ws-state', onWsState)
    window.addEventListener('sebas:refetch', onRefetch)
    sharedWs.reconnectNow()
    void sessions.refresh()
    return () => {
      off()
      window.removeEventListener('sebas:ws-state', onWsState)
      window.removeEventListener('sebas:refetch', onRefetch)
    }
  }, [sessions, taskStore, probeReachability, applyFatal])

  useEffect(() => {
    document.title = project ? `${project.name} · sebas` : 'sebas 工作台'
  }, [project])

  async function logout(): Promise<void> {
    await auth.logout()
    sharedWs.setAuthGated(true)
    navigate('/')
  }

  const railBody = (
    <div
      className={styles.railInner}
      data-rail-compact={!narrow && compactRail ? 'true' : 'false'}
    >
      <div className={styles.railHead}>
        <button type="button" className={styles.brand} onClick={() => navigate('/')}>
          <span className={styles.mark} aria-hidden="true">
            &gt;
          </span>
          <span className={styles.brandText}>
            <span className={styles.brandName}>sebas</span>
            <span className={styles.brandSub}>agent console</span>
          </span>
        </button>
        <button
          type="button"
          className={styles.railToggle}
          aria-label={railCollapsed ? '展开导航' : '收起导航'}
          aria-pressed={railCollapsed}
          onClick={toggleRail}
        >
          {railCollapsed ? '»' : '«'}
        </button>
      </div>
      <nav className={styles.nav} aria-label="主导航">
        <button
          type="button"
          className={styles.navItem}
          aria-label="工作台"
          aria-current={pathname === '/' ? 'page' : undefined}
          onClick={() => {
            setDrawerOpen(false)
            navigate('/')
          }}
        >
          <span className={styles.navIcon} data-icon="◈" aria-hidden="true" />
          <span className={styles.navLabel}>工作台</span>
        </button>
        <button
          type="button"
          className={styles.navItem}
          aria-label="会话总览"
          aria-current={pathname.startsWith('/sessions') ? 'page' : undefined}
          onClick={() => {
            setDrawerOpen(false)
            navigate('/sessions')
          }}
        >
          <span className={styles.navIcon} data-icon="≡" aria-hidden="true" />
          <span className={styles.navLabel}>会话总览</span>
        </button>
        <button
          type="button"
          className={styles.navItem}
          aria-label="用量"
          aria-current={pathname.startsWith('/usage') ? 'page' : undefined}
          onClick={() => {
            setDrawerOpen(false)
            navigate('/usage')
          }}
        >
          <span className={styles.navIcon} data-icon="∿" aria-hidden="true" />
          <span className={styles.navLabel}>用量</span>
        </button>
      </nav>
      <ProjectRail
        sessions={sessions}
        selectedProjectId={selectedProjectId}
        onSelectProject={selectProject}
        canCreate={canCreate}
        focusedKey={route.kind === 'session' ? route.key : null}
        onFocus={openSession}
        onRequestCreate={(projectId) => setCreate({ open: true, projectId, task: null })}
        disconnected={connected === false}
        onRetry={() => {
          sharedWs.reconnectNow()
          void sessions.refresh()
        }}
      />
      <div className={styles.railFoot}>
        <Button
          className={styles.navItem}
          aria-label="设置"
          aria-current={settingsOpen ? 'page' : undefined}
          onClick={openSettings}
        >
          <span className={styles.navIcon} data-icon="⚙" aria-hidden="true" />
          <span className={styles.navLabel}>设置</span>
        </Button>
        <Dropdown
          trigger={['click']}
          menu={{
            items: [{ key: 'logout', label: '退出登录', danger: true }],
            onClick: ({ key }) => {
              if (key === 'logout') void logout()
            },
          }}
        >
          <Button className={styles.user} aria-label="用户菜单">
            <span className={styles.navIcon} data-icon="◍" aria-hidden="true" />
            <span className={styles.userName}>{username ?? '未登录'}</span>
            <span className={styles.role}>{role ? ROLE_LABEL[role] : '免登录'}</span>
          </Button>
        </Dropdown>
      </div>
    </div>
  )

  let content: ReactNode
  if (route.kind === 'usage') {
    content = <UsagePage />
  } else if (route.kind === 'sessions') {
    content = (
      <SessionsPage
        sessions={sessions}
        canCreate={canCreate}
        onFocus={openSession}
        onRequestCreate={() => setCreate({ open: true, projectId: selectedProjectId, task: null })}
      />
    )
  } else {
    const stageProjectId = focusedProjectId ?? selectedProjectId
    content = (
      <WorkbenchHome
        sessionKey={route.kind === 'session' ? route.key : null}
        sessions={sessions}
        selectedProjectId={stageProjectId}
        canCreate={canCreate}
        onRequestCreate={() =>
          setCreate({ open: true, projectId: stageProjectId, task: null })
        }
        onFocus={openSession}
        tab={tabForProject(tabByProject, stageProjectId)}
        onTabChange={setStageTab(stageProjectId)}
        tasks={taskStore}
        openTaskId={openTask !== null && openTask.projectId === stageProjectId ? openTask.taskId : null}
        onOpenTask={(taskId) => openTaskDetail(stageProjectId, taskId)}
        onCloseTask={() => setOpenTask(null)}
        onStartTaskSession={(task) => requestCreateForTask(stageProjectId, task)}
        username={username}
        role={role}
        agentIds={agentIds}
      />
    )
  }

  return (
    <div className={styles.app} data-testid="react-shell">
      {!narrow ? (
        <aside
          className={`${styles.rail} ${railCollapsed ? styles.railCollapsed : ''} ${resizingRail ? styles.railResizing : ''}`}
          aria-label="项目导航"
          style={{ '--rail-width': `${railWidth}px` } as CSSProperties}
        >
          {railBody}
          <div
            className={styles.railResize}
            role="separator"
            aria-label="调整导航宽度"
            aria-orientation="vertical"
            aria-valuemin={RAIL_MIN_PX}
            aria-valuemax={RAIL_MAX_PX}
            aria-valuenow={railWidth}
            tabIndex={0}
            onPointerDown={beginRailResize}
            onPointerMove={moveRailResize}
            onPointerUp={endRailResize}
            onPointerCancel={endRailResize}
            onKeyDown={(event) => {
              if (event.key === 'ArrowLeft') {
                event.preventDefault()
                resizeRailBy(-16)
              } else if (event.key === 'ArrowRight') {
                event.preventDefault()
                resizeRailBy(16)
              } else if (event.key === 'Home') {
                event.preventDefault()
                resizeRailBy(RAIL_MIN_PX - railWidth)
              } else if (event.key === 'End') {
                event.preventDefault()
                resizeRailBy(RAIL_MAX_PX - railWidth)
              }
            }}
          />
        </aside>
      ) : (
        <Drawer
          open={drawerOpen}
          placement="left"
          size={300}
          onClose={closeDrawer}
          styles={{ body: { padding: 0 } }}
          title="导航"
        >
          {railBody}
        </Drawer>
      )}

      <div className={styles.main}>
        <header className={styles.topbar}>
          {narrow ? (
            <Button id="nav-menu-toggle" size="small" onClick={() => setDrawerOpen(true)}>
              菜单
            </Button>
          ) : null}
          <span className={styles.projectName}>{project?.name ?? '工作台'}</span>
          {project ? <span className={styles.projectPath}>{project.path}</span> : null}
          <span className={styles.spacer} />
          {focusStatusSlug !== null ? (
            <Tag className={styles.focusTag} title="聚焦会话">
              {statusLabel(focusStatusSlug)}
            </Tag>
          ) : null}
          <span
            className={`${styles.conn} ${connected === true ? styles.connOk : connected === false ? styles.connBad : ''}`}
            role="status"
          >
            {connected === null ? '连接中…' : connected ? '已连接' : '连接已断开'}
          </span>
        </header>
        <main className={styles.content}>{content}</main>
      </div>

      {/* 设置是浮于工作台之上的独立窗口（无独立路由）：工作台保持在下面，
       * 关闭后回到原会话与草稿。定位、宽度与窄屏分区横排在
       * settings.module.css 里约束。 */}
      <Modal
        open={settingsOpen}
        onCancel={closeSettings}
        afterClose={restoreSettingsFocus}
        footer={null}
        title="设置"
        closable={{ 'aria-label': '关闭设置' }}
        width={980}
        style={{ top: 40 }}
        styles={{
          // Full-bleed window: the settings surface owns the frame's insets so
          // its section rail can reach the window edge instead of floating
          // inside AntD's default 24px content padding.
          container: { padding: 0 },
          header: { padding: '14px 16px', marginBottom: 0 },
          body: { padding: 0 },
        }}
        // A large window that scales from the centre reads as a "pop" and
        // shifts measured geometry mid-animation: open it with a quiet fade
        // (opacity only) instead of AntD's default zoom.
        transitionName="sebas-settings-fade"
        destroyOnHidden
        focusable={{ focusTriggerAfterClose: false }}
      >
        <SettingsPage role={role} username={username} />
      </Modal>

      <NoticeHost
        onRetry={() => {
          sharedWs.reconnectNow()
          void sessions.refresh()
          notify({ level: 'info', message: '正在重新连接核心…' })
        }}
      />

      <SessionCreateModal
        open={create.open}
        projectId={create.projectId}
        projects={projects}
        task={create.task}
        onClose={() => setCreate({ open: false, projectId: null, task: null })}
        onCreated={(key) => {
          // 反查项目不可靠：新行尚未进列表（refresh 未 await）——直接用创建
          // 请求的项目把 stage 切回会话视图。
          const projectId = create.projectId
          setCreate({ open: false, projectId: null, task: null })
          if (projectId !== null) {
            setTabByProject((c) => withProjectTab(c, projectId, 'conversation'))
          }
          void sessions.refresh()
          openSession(key)
        }}
      />
    </div>
  )
}
