import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { BeadsIssue } from '../../api/client.js'
import { TaskStage } from './TaskStage.js'
import { TaskStore, type TaskApi } from './task-store.js'
import { defaultTaskActions, type TaskActionApi, type TaskViewer } from './actions.js'

afterEach(cleanup)

const viewer: TaskViewer = { username: 'cupen', role: 'member', agentIds: new Set(['claude']) }

function issue(patch: Partial<BeadsIssue> = {}): BeadsIssue {
  return { id: 'bd-42', title: '任务主题', status: 'open', labels: [], ...patch }
}

function storeSeam(tasks: BeadsIssue[]): TaskApi {
  return {
    list: async () => ({ tasks: tasks.slice() }),
    refresh: async () => ({ tasks: tasks.slice() }),
    watch: async () => {},
    unwatch: async () => {},
  }
}

function actionSeam(overrides: Partial<TaskActionApi> = {}): TaskActionApi {
  return { ...defaultTaskActions, ...overrides }
}

function renderStage(
  tasks: BeadsIssue[],
  overrides: {
    openTaskId?: string | null
    actions?: TaskActionApi
    onStartSession?: (task: { id: string; title: string }) => void
    onOpenTask?: (id: string) => void
    onOpenSession?: (key: string) => void
  } = {},
) {
  const store = new TaskStore(storeSeam(tasks))
  const handlers = {
    onStartSession: overrides.onStartSession ?? vi.fn(),
    onOpenTask: overrides.onOpenTask ?? vi.fn(),
    onOpenSession: overrides.onOpenSession ?? vi.fn(),
  }
  render(
    <TaskStage
      projectId="p1"
      store={store}
      viewer={viewer}
      sessions={[]}
      openTaskId={overrides.openTaskId ?? null}
      onOpenTask={handlers.onOpenTask}
      onCloseTask={() => {}}
      onOpenSession={handlers.onOpenSession}
      onStartSession={handlers.onStartSession}
      actions={overrides.actions ?? actionSeam()}
    />,
  )
  return { store, handlers }
}

describe('TaskStage', () => {
  it('loads the board on entry and opens a task detail with comments', async () => {
    const show = vi.fn(async () =>
      issue({
        title: '详情标题',
        description: '描述正文',
        comments: [
          { id: 'c1', issue_id: 'bd-42', author: 'claude', text: '处理中', created_at: '2026-11-24T10:00:00Z' },
        ],
      }),
    )
    renderStage([issue()], { openTaskId: 'bd-42', actions: actionSeam({ show }) })
    const detail = await screen.findByTestId('task-detail')
    expect(show).toHaveBeenCalledWith('p1', 'bd-42')
    await waitFor(() => expect(detail.textContent).toContain('详情标题'))
    expect(await screen.findByTestId('task-comment-c1')).toBeTruthy()
  })

  it('claims the task when a card is dragged into in progress', async () => {
    const claim = vi.fn(async () => issue({ status: 'in_progress', assignee: 'cupen' }))
    renderStage([issue()], { actions: actionSeam({ claim }) })
    await waitFor(() => expect(screen.getByTestId('task-card-bd-42')).toBeTruthy())
    const card = screen.getByTestId('task-card-bd-42')
    fireEvent.dragStart(card, {
      dataTransfer: { effectAllowed: 'move', setData: () => {}, getData: () => 'bd-42' },
    })
    fireEvent.drop(screen.getByTestId('task-column-in_progress'), { dataTransfer: {} })
    await waitFor(() => expect(claim).toHaveBeenCalledWith('p1', 'bd-42'))
    await waitFor(() =>
      expect(screen.getByTestId('task-column-in_progress').textContent).toContain('任务主题'),
    )
  })

  it('runs the close action from the detail sidebar and updates the store', async () => {
    const close = vi.fn(async () => issue({ status: 'closed' }))
    renderStage([issue({ status: 'in_progress', labels: ['in-review'] })], {
      openTaskId: 'bd-42',
      actions: actionSeam({ close }),
    })
    await screen.findByTestId('task-detail')
    fireEvent.click(screen.getByTestId('task-action-close'))
    await waitFor(() => expect(close).toHaveBeenCalledWith('p1', 'bd-42'))
    await waitFor(() =>
      expect(screen.getByTestId('task-metadata').textContent).toContain('已完成'),
    )
  })

  it('starts a session for the task with its id and title', async () => {
    const onStartSession = vi.fn()
    renderStage([issue({ title: '任务标题' })], { openTaskId: 'bd-42', onStartSession })
    await screen.findByTestId('task-detail')
    fireEvent.click(screen.getByTestId('task-start-session'))
    expect(onStartSession).toHaveBeenCalledWith({ id: 'bd-42', title: '任务标题' })
  })
})
