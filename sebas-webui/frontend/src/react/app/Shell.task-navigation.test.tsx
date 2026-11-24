// @vitest-environment jsdom
/**
 * 任务中心 ↔ 会话视图的**双向跳转**（agent-workbench 三条 requirement 的
 * 集成面，agent-task-center 8.4/8.5/9.1）。
 *
 * 组件级已由 `WorkbenchHome.tasks.test.tsx` / `TaskDetail.test.tsx` 覆盖；本
 * 文件钉的是 Shell 的**接线**（stage tab 状态、openTask 指针、创建回调）：
 *
 * 1. 会话头「Task bd-XX」→ 任务详情；
 * 2. 详情页点关联会话 → 回到会话视图；
 * 3. 详情页「开一轮会话推进此任务」→ 创建后 stage 切回会话视图
 *    （spec「Confirming creates a linked session … the stage switches to
 *    Conversation」）。
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const wsFake = vi.hoisted(() => ({
  subscribe: (_handler: (event: unknown) => void) => () => {},
  request: async (_method: string, _params?: unknown) => ({ ok: true }),
  setAuthGated: (_gated: boolean) => {},
  reconnectNow: () => {},
}))
vi.mock('../../api/shared-ws.js', () => ({ sharedWs: wsFake }))

import {
  api,
  type BeadsIssue,
  type Project,
  type SessionDetail,
  type SessionList,
  type SessionRow,
  type Summary,
} from '../../api/client.js'
import { AuthStore } from '../state/auth-store.js'
import { SessionStore } from '../state/session-store.js'
import { Shell } from './Shell.js'

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  window.history.replaceState({}, '', '/')
})

const projects: Project[] = [{ id: 'p1', name: 'alpha', path: '/work/alpha', added_at: 1 }]

function sessionRow(key: string, taskId: string | null): SessionRow {
  return {
    encoded_key: key,
    thread_id: null,
    session_id: key,
    session_id_short: key,
    status: 'done',
    status_label: 'done',
    status_slug: 'done',
    status_glyph: '',
    last_active: '',
    last_active_unix: 1_700_000_000,
    is_active: false,
    project_id: 'p1',
    prompt_preview: `会话 ${key}`,
    label: null,
    msg_count: 1,
    current_model: null,
    available_models: null,
    agent_kind: 'claude',
    pending_count: 0,
    turn_engaged: false,
    desired_mode: 'ask',
    task_id: taskId,
  }
}

function sessionDetail(key: string, taskId: string | null): SessionDetail {
  return {
    chat_id: 'c1',
    thread_id: null,
    session_id: key,
    status: 'done',
    status_label: 'done',
    status_slug: 'done',
    status_glyph: '',
    entries: [
      { position: 0, kind: 'prompt', element_type: 'markdown', content: '你好', created_at_unix: 0 },
    ],
    msg_id: null,
    last_active: '',
    encoded_key: key,
    msg_count: 1,
    current_model: null,
    available_models: null,
    agent_kind: 'claude',
    desired_mode: 'ask',
    pending: [],
    turn_engaged: false,
    task_id: taskId,
  }
}

const issue = (patch: Partial<BeadsIssue> = {}): BeadsIssue => ({
  id: 'bd-42',
  title: '任务主题',
  description: '任务描述正文',
  status: 'in_progress',
  assignee: 'cupen',
  labels: [],
  metadata: { reviewer: 'claude' },
  comments: [
    {
      id: 'c1',
      issue_id: 'bd-42',
      author: 'claude',
      text: '处理中',
      created_at: '2026-11-24T10:00:00Z',
    },
  ],
  ...patch,
})

/** 可变的会话行：创建流程在「服务端」新增一行后由 refresh 取回。 */
function makeSessions(rows: SessionRow[]): SessionStore {
  const summary: Summary = {
    active_count: 0,
    dormant_count: rows.length,
    spawning_count: 0,
    total_sessions: rows.length,
    uptime: '1h',
    recent_sessions: rows.slice(),
    active_session: null,
    active_session_key: null,
    reachability: { ok: true },
  }
  const list = (): SessionList => ({
    recent_sessions: rows.slice(),
    active_count: 0,
    dormant_count: rows.length,
    spawning_count: 0,
    total_sessions: rows.length,
    active_session_key: null,
  })
  return new SessionStore({
    summary: async () => summary,
    sessions: async () => list(),
    projects: async () => ({ projects }),
  })
}

function makeAuth(): AuthStore {
  return new AuthStore({
    authMe: async () => ({ enabled: false, authenticated: true }) as never,
    authLogin: async () => ({}),
    authSetup: async () => ({}),
    authLogout: async () => ({}),
  })
}

function stubApi(detailFor: (key: string) => SessionDetail, tasks: BeadsIssue[]): void {
  vi.spyOn(api, 'agents').mockResolvedValue({
    agents: [{ id: 'claude', display: 'Claude', reachable: true }],
  } as never)
  vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [] } as never)
  vi.spyOn(api, 'session').mockImplementation(async (key: string) => detailFor(key))
  vi.spyOn(api, 'sessionApprovals').mockResolvedValue({ approvals: [] } as never)
  vi.spyOn(api, 'tasksList').mockResolvedValue({ tasks: tasks.slice() } as never)
  vi.spyOn(api, 'tasksRefresh').mockResolvedValue({ tasks: tasks.slice() } as never)
  vi.spyOn(api, 'taskShow').mockResolvedValue(issue() as never)
  vi.spyOn(api, 'providers').mockResolvedValue({ providers: [], model_aliases: [] } as never)
  vi.spyOn(api, 'providerDefaults').mockResolvedValue(null as never)
  vi.spyOn(api, 'sessionApprovals').mockResolvedValue({ approvals: [] } as never)
}

beforeEach(() => {
  vi.stubGlobal(
    'matchMedia',
    (query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList,
  )
})

describe('Shell task ↔ session navigation', () => {
  it('round-trips: session back-link → task detail → linked session → conversation', async () => {
    window.history.replaceState({}, '', '/sessions/oc_linked')
    stubApi((key) => sessionDetail(key, 'bd-42'), [issue()])
    const sessions = makeSessions([sessionRow('oc_linked', 'bd-42')])
    await sessions.refresh()

    render(<Shell auth={makeAuth()} sessions={sessions} role="member" username="cupen" />)

    // 会话详情头渲染回链（9.1）。
    const link = await screen.findByTestId('session-task-link')
    expect(link.textContent).toContain('Task bd-42')
    fireEvent.click(link)

    // 回链 → stage 切到 Tasks 并打开详情（9.1）。
    expect(await screen.findByTestId('task-detail')).toBeTruthy()
    expect(screen.getByTestId('stage-tab-tasks').getAttribute('aria-pressed')).toBe('true')

    // 详情页关联会话 → 切回会话视图（8.4）。
    fireEvent.click(screen.getByTestId('task-session-oc_linked'))
    expect(await screen.findByTestId('workbench-session')).toBeTruthy()
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull())
    expect(screen.getByTestId('stage-tab-conversation').getAttribute('aria-pressed')).toBe('true')
  })

  it('switches the stage to Conversation after starting a session from the task', async () => {
    window.history.replaceState({}, '', '/')
    stubApi((key) => sessionDetail(key, null), [issue()])
    const rows = [sessionRow('oc_linked', 'bd-42')]
    const sessions = makeSessions(rows)
    await sessions.refresh()
    const createSession = vi
      .spyOn(api, 'createSession')
      .mockResolvedValue({ key: 'oc_new' } as never)

    render(<Shell auth={makeAuth()} sessions={sessions} role="member" username="cupen" />)

    // 选中项目 → 打开任务详情。
    fireEvent.click(await screen.findByLabelText('alpha 1'))
    fireEvent.click(screen.getByTestId('stage-tab-tasks'))
    fireEvent.click(await screen.findByTestId('task-card-bd-42'))
    await screen.findByTestId('task-detail')

    // 详情页「开一轮会话推进此任务」→ 复用创建对话框（8.5）。
    fireEvent.click(screen.getByTestId('task-start-session'))
    // 服务端新增行：确认后 refresh 取回（focusedProject 解析依赖它）。
    rows.push(sessionRow('oc_new', 'bd-42'))
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Agent' })).toBeTruthy())
    fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Agent' }))
    fireEvent.click(await screen.findByText('Claude'))
    fireEvent.click(screen.getByRole('button', { name: /创\s*建$/ }))

    await waitFor(() => expect(createSession).toHaveBeenCalled())
    expect(createSession.mock.calls[0][0]).toMatchObject({ taskId: 'bd-42' })

    // spec「Confirming creates a linked session … the stage switches to
    // Conversation」：新会话的会话视图必须接管 stage，任务详情退出。
    await waitFor(() =>
      expect(screen.getByTestId('stage-tab-conversation').getAttribute('aria-pressed')).toBe(
        'true',
      ),
    )
    expect(screen.queryByTestId('task-detail')).toBeNull()
  })
})
