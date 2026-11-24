import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { BeadsIssue, SessionRow } from '../../api/client.js'
import { TaskDetail } from './TaskDetail.js'
import type { TaskViewer } from './actions.js'

afterEach(cleanup)

const agentIds = new Set(['claude'])
const human: TaskViewer = { username: 'cupen', role: 'member', agentIds }

function issue(patch: Partial<BeadsIssue> = {}): BeadsIssue {
  return {
    id: 'bd-42',
    title: '补齐任务看板',
    description: '## 背景\n\n需要三列看板。',
    status: 'open',
    labels: [],
    metadata: { reviewer: 'claude' },
    created_at: '2026-11-24T10:00:00Z',
    updated_at: '2026-11-24T11:00:00Z',
    comments: [
      {
        id: 'c1',
        issue_id: 'bd-42',
        author: 'claude',
        text: '**开始处理**',
        created_at: '2026-11-24T10:30:00Z',
      },
      {
        id: 'c2',
        issue_id: 'bd-42',
        author: 'cupen',
        text: '等我确认下',
        created_at: '2026-11-24T10:40:00Z',
      },
    ],
    ...patch,
  }
}

const sessionRow = (key: string, active: number): SessionRow => ({
  encoded_key: key,
  thread_id: null,
  session_id: key,
  session_id_short: key,
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  last_active: '',
  last_active_unix: active,
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
  task_id: 'bd-42',
})

function renderDetail(
  detail: BeadsIssue,
  overrides: {
    viewer?: TaskViewer
    sessions?: SessionRow[]
    canAct?: boolean
    onAction?: (action: string) => void
    onComment?: (text: string) => Promise<void>
    onStartSession?: () => void
    onOpenSession?: (key: string) => void
  } = {},
) {
  const handlers = {
    onAction: overrides.onAction ?? vi.fn(),
    onComment: overrides.onComment ?? vi.fn(async () => {}),
    onStartSession: overrides.onStartSession ?? vi.fn(),
    onOpenSession: overrides.onOpenSession ?? vi.fn(),
  }
  render(
    <TaskDetail
      issue={detail}
      agentIds={agentIds}
      viewer={overrides.viewer ?? human}
      sessions={overrides.sessions ?? [sessionRow('k-new', 300), sessionRow('k-old', 200)]}
      loading={false}
      actionBusy={false}
      actionError={null}
      onBack={() => {}}
      onAction={handlers.onAction}
      onComment={handlers.onComment}
      onOpenSession={handlers.onOpenSession}
      onStartSession={handlers.onStartSession}
      canAct={overrides.canAct ?? (overrides.viewer?.role === 'viewer' ? false : true)}
    />
  )
  return handlers
}

describe('TaskDetail', () => {
  it('renders the two-column layout with description and metadata', () => {
    renderDetail(issue())
    expect(screen.getByTestId('task-detail')).toBeTruthy()
    expect(screen.getByTestId('task-description').textContent).toContain('背景')
    expect(screen.getByTestId('task-description').textContent).toContain('需要三列看板')
    const meta = screen.getByTestId('task-metadata')
    expect(meta.textContent).toContain('评审人')
    expect(meta.textContent).toContain('claude')
    expect(meta.textContent).toContain('待认领')
  })

  it('offers claim only for an unassigned task', () => {
    const handlers = renderDetail(issue())
    expect(screen.getByTestId('task-action-claim')).toBeTruthy()
    expect(screen.queryByTestId('task-action-release')).toBeNull()
    expect(screen.queryByTestId('task-action-close')).toBeNull()
    fireEvent.click(screen.getByTestId('task-action-claim'))
    expect(handlers.onAction).toHaveBeenCalledWith('claim')
  })

  it('shows release and enter-review for the assignee of an in-progress task', () => {
    renderDetail(issue({ status: 'in_progress', assignee: 'cupen' }))
    expect(screen.getByTestId('task-action-release')).toBeTruthy()
    expect(screen.getByTestId('task-action-review')).toBeTruthy()
    expect(screen.queryByTestId('task-action-close')).toBeNull()
  })

  it('shows close and cancel to a human on an in-review task', () => {
    renderDetail(issue({ status: 'in_progress', assignee: 'claude', labels: ['in-review'] }))
    expect(screen.getByTestId('task-action-close')).toBeTruthy()
    expect(screen.getByTestId('task-action-cancel')).toBeTruthy()
  })

  it('hides close from an agent that is not the designated reviewer', () => {
    renderDetail(issue({ status: 'in_progress', labels: ['in-review'] }), {
      viewer: { username: 'codex', role: 'member', agentIds: new Set(['claude', 'codex']) },
    })
    expect(screen.queryByTestId('task-action-close')).toBeNull()
    expect(screen.getByTestId('task-action-cancel')).toBeTruthy()
  })

  it('renders no actions for a read-only viewer', () => {
    renderDetail(issue(), { viewer: { username: null, role: 'viewer', agentIds } })
    expect(screen.queryByTestId('task-action-claim')).toBeNull()
    expect(screen.queryByTestId('task-start-session')).toBeNull()
    expect(screen.getByText('只读身份，无操作权限')).toBeTruthy()
  })

  it('opens the existing creation dialog for the task', () => {
    const handlers = renderDetail(issue())
    fireEvent.click(screen.getByTestId('task-start-session'))
    expect(handlers.onStartSession).toHaveBeenCalled()
  })

  it('posts a comment and clears the composer', async () => {
    const handlers = renderDetail(issue())
    const box = screen.getByLabelText('发表评论')
    fireEvent.change(box, { target: { value: '看板列需要计数' } })
    fireEvent.click(screen.getByTestId('task-comment-submit'))
    await waitFor(() => expect(handlers.onComment).toHaveBeenCalledWith('看板列需要计数'))
    await waitFor(() => expect((box as HTMLTextAreaElement).value).toBe(''))
  })

  it('renders comment authors with the human/agent avatar convention', () => {
    renderDetail(issue())
    expect(screen.getByTestId('task-comment-c1').textContent).toContain('agent')
    expect(screen.getByTestId('task-comment-c1').innerHTML).toContain('<strong>开始处理</strong>')
    expect(screen.getByTestId('task-comment-c2').textContent).toContain('cupen')
  })

  it('lists linked sessions newest-first and jumps to the conversation on click', () => {
    const handlers = renderDetail(issue())
    const rows = screen.getAllByTestId(/task-session-/)
    expect(rows[0].getAttribute('data-testid')).toBe('task-session-k-new')
    fireEvent.click(screen.getByTestId('task-session-k-new'))
    expect(handlers.onOpenSession).toHaveBeenCalledWith('k-new')
  })
})
