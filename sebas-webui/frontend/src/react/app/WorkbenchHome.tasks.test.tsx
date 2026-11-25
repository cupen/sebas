import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  api,
  type BeadsIssue,
  type Project,
  type SessionDetail,
  type SessionList,
  type SessionRow,
  type Summary,
} from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { TaskStore, type TaskApi } from '../tasks/task-store.js'
import { WorkbenchHome } from './WorkbenchHome.js'

afterEach(cleanup)

const row: SessionRow = {
  encoded_key: 'oc_1',
  thread_id: null,
  session_id: 's1',
  session_id_short: 's1',
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  last_active: '',
  last_active_unix: 1_700_000_000,
  is_active: true,
  project_id: 'p1',
  prompt_preview: '首个任务',
  label: null,
  msg_count: 1,
  current_model: null,
  available_models: null,
  agent_kind: 'claude',
  pending_count: 0,
  turn_engaged: false,
  desired_mode: 'ask',
  task_id: 'bd-42',
}

const projects: Project[] = [{ id: 'p1', name: 'alpha', path: '/work/alpha', added_at: 1 }]

async function sessionStore(): Promise<SessionStore> {
  const summary: Summary = {
    active_count: 1,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 1,
    uptime: '1h',
    recent_sessions: [row],
    active_session: null,
    active_session_key: null,
    reachability: { ok: true },
  }
  const list: SessionList = {
    recent_sessions: [row],
    active_count: 1,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 1,
    active_session_key: null,
  }
  const sessions = new SessionStore({
    summary: async () => summary,
    sessions: async () => list,
    projects: async () => ({ projects }),
  })
  await sessions.refresh()
  return sessions
}

function taskStore(tasks: BeadsIssue[]): TaskStore {
  const seam: TaskApi = {
    list: async () => ({ tasks: tasks.slice() }),
    refresh: async () => ({ tasks: tasks.slice() }),
    watch: async () => {},
    unwatch: async () => {},
  }
  return new TaskStore(seam)
}

const issue: BeadsIssue = { id: 'bd-42', title: '任务主题', status: 'open', labels: [] }

const detail = (taskId: string | null): SessionDetail => ({
  chat_id: 'c1',
  thread_id: null,
  session_id: 's1',
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  entries: [{ position: 0, kind: 'prompt', element_type: 'markdown', content: '你好', created_at_unix: 0 }],
  msg_id: null,
  last_active: '',
  encoded_key: 'oc_1',
  msg_count: 1,
  current_model: null,
  available_models: null,
  agent_kind: 'claude',
  desired_mode: 'ask',
  pending: [],
  turn_engaged: false,
  task_id: taskId,
})

describe('WorkbenchHome stage tabs', () => {
  it('shows the stage tabs for a selected project and switches views', async () => {
    const sessions = await sessionStore()
    const onTabChange = vi.fn()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId="p1"
        canCreate
        onRequestCreate={() => {}}
        onFocus={() => {}}
        tab="conversation"
        onTabChange={onTabChange}
        tasks={taskStore([issue])}
      />,
    )
    expect(screen.getByTestId('stage-tab-conversation').getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(screen.getByTestId('stage-tab-tasks'))
    expect(onTabChange).toHaveBeenCalledWith('tasks')
  })

  it('renders the board on the tasks tab while keeping the conversation mounted', async () => {
    const sessions = await sessionStore()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId="p1"
        canCreate
        onRequestCreate={() => {}}
        onFocus={() => {}}
        tab="tasks"
        onTabChange={() => {}}
        tasks={taskStore([issue])}
      />,
    )
    expect(await screen.findByTestId('task-board')).toBeTruthy()
    expect(screen.getByTestId('stage-tab-tasks').getAttribute('aria-pressed')).toBe('true')
    // 会话区仍在 DOM 里（隐藏），切换回来时内容原样保留。
    expect(screen.getByTestId('workbench-home').parentElement?.hasAttribute('hidden')).toBe(true)
  })

  it('hides the tabs when no project is selected', async () => {
    const sessions = await sessionStore()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId={null}
        canCreate
        onRequestCreate={() => {}}
        onFocus={() => {}}
        tab="conversation"
        onTabChange={() => {}}
        tasks={taskStore([])}
      />,
    )
    expect(screen.queryByTestId('stage-tabs')).toBeNull()
  })
})

describe('session task back-link', () => {
  it('renders a Task link for a linked session and opens the task detail', async () => {
    vi.spyOn(api, 'session').mockResolvedValue(detail('bd-42'))
    const sessions = await sessionStore()
    const onOpenTask = vi.fn()
    render(
      <WorkbenchHome
        sessionKey="oc_1"
        sessions={sessions}
        selectedProjectId="p1"
        canCreate
        onRequestCreate={() => {}}
        onFocus={() => {}}
        tab="conversation"
        onTabChange={() => {}}
        tasks={taskStore([issue])}
        onOpenTask={onOpenTask}
      />,
    )
    await waitFor(() => expect(screen.getByTestId('session-task-link')).toBeTruthy())
    expect(screen.getByTestId('session-task-link').textContent).toContain('Task bd-42')
    fireEvent.click(screen.getByTestId('session-task-link'))
    expect(onOpenTask).toHaveBeenCalledWith('bd-42')
    vi.restoreAllMocks()
  })

  it('renders no link for a session without a task', async () => {
    vi.spyOn(api, 'session').mockResolvedValue(detail(null))
    const sessions = await sessionStore()
    render(
      <WorkbenchHome
        sessionKey="oc_1"
        sessions={sessions}
        selectedProjectId="p1"
        canCreate
        onRequestCreate={() => {}}
        onFocus={() => {}}
        onOpenTask={() => {}}
      />,
    )
    await screen.findByTestId('workbench-session')
    expect(screen.queryByTestId('session-task-link')).toBeNull()
    vi.restoreAllMocks()
  })
})
