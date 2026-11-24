import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { BeadsIssue } from '../../api/client.js'
import { TaskBoard } from './TaskBoard.js'
import { TaskCard } from './TaskCard.js'
import type { TaskViewer } from './actions.js'
import type { TaskColumn } from './task-store.js'

afterEach(cleanup)

const updatedAt = '2026-11-24T12:00:00Z'

const unassigned: BeadsIssue = {
  id: 'bd-1',
  title: '补齐会话终态联动',
  status: 'open',
  labels: [],
  comment_count: 2,
  updated_at: updatedAt,
}

const inProgress: BeadsIssue = {
  id: 'bd-2',
  title: '一个很长很长很长很长很长很长很长很长很长很长的任务标题用于折行截断',
  status: 'in_progress',
  assignee: 'claude',
  labels: [],
  comment_count: 0,
  updated_at: updatedAt,
}

const inReview: BeadsIssue = {
  id: 'bd-3',
  title: '评审中的任务',
  status: 'in_progress',
  assignee: 'cupen',
  labels: ['in-review'],
  comment_count: 1,
  updated_at: updatedAt,
}

const done: BeadsIssue = {
  id: 'bd-4',
  title: '已完成的任务',
  status: 'closed',
  labels: ['in-review'],
  updated_at: updatedAt,
}

const human: TaskViewer = { username: 'cupen', role: 'member', agentIds: new Set(['claude']) }

function renderBoard(
  tasks: BeadsIssue[],
  overrides: {
    viewer?: TaskViewer
    onMove?: (issue: BeadsIssue, target: TaskColumn) => Promise<void>
    sessionCounts?: Record<string, number>
  } = {},
) {
  const onOpen = vi.fn()
  const onMove = overrides.onMove ?? vi.fn(async () => {})
  const result = render(
    <TaskBoard
      tasks={tasks}
      loading={false}
      error={null}
      agentIds={new Set(['claude'])}
      viewer={overrides.viewer ?? human}
      sessionCounts={overrides.sessionCounts ?? { 'bd-2': 2 }}
      onOpen={onOpen}
      onMove={onMove}
      onRefresh={() => {}}
    />,
  )
  return { ...result, onOpen, onMove }
}

function drag(card: HTMLElement, column: HTMLElement): void {
  const data: Record<string, string> = {}
  const dataTransfer = {
    effectAllowed: 'move',
    setData: (type: string, value: string) => {
      data[type] = value
    },
    getData: (type: string) => data[type] ?? '',
  }
  fireEvent.dragStart(card, { dataTransfer })
  fireEvent.drop(column, { dataTransfer })
}

describe('TaskBoard', () => {
  it('renders three columns with accurate counts and a collapsed done region', () => {
    renderBoard([unassigned, inProgress, inReview, done])
    expect(screen.getByTestId('task-count-unassigned').textContent).toBe('1')
    expect(screen.getByTestId('task-count-in_progress').textContent).toBe('1')
    expect(screen.getByTestId('task-count-in_review').textContent).toBe('1')
    expect(screen.queryByTestId('task-card-bd-4')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /已完成 \/ 已取消/ }))
    expect(screen.getByTestId('task-card-bd-4')).toBeTruthy()
  })

  it('shows card fields and opens the task on click', () => {
    const { onOpen } = renderBoard([inProgress])
    const card = screen.getByTestId('task-card-bd-2')
    expect(card.textContent).toContain('一个很长')
    expect(screen.getByTestId('task-comments-bd-2').textContent).toContain('0')
    expect(screen.getByTestId('task-sessions-bd-2').textContent).toContain('2')
    expect(screen.getByTestId('task-avatar-agent-bd-2')).toBeTruthy()
    expect(card.textContent).toContain('agent')
    fireEvent.click(card)
    expect(onOpen).toHaveBeenCalledWith('bd-2')
  })

  it('renders a dashed unassigned avatar without an agent chip', () => {
    render(
      <TaskCard issue={unassigned} agentIds={new Set(['claude'])} sessionCount={0} onOpen={() => {}} draggable now={Date.parse(updatedAt)} />,
    )
    expect(screen.getByLabelText('未认领')).toBeTruthy()
    expect(screen.getByTestId('task-title-bd-1').textContent).toBe(unassigned.title)
    expect(screen.queryByText('agent')).toBeNull()
  })

  it('dragging to in_progress claims the task', async () => {
    const claim = vi.fn(async () => {})
    const { onMove } = renderBoard([inProgress, unassigned], { onMove: claim })
    // bd-1 is unassigned; move it into in_progress.
    drag(screen.getByTestId('task-card-bd-1'), screen.getByTestId('task-column-in_progress'))
    await waitFor(() => expect(onMove).toHaveBeenCalledWith(unassigned, 'in_progress'))
  })

  it('dragging in_progress to in_review enters review', async () => {
    const move = vi.fn(async () => {})
    renderBoard([inProgress], { onMove: move })
    drag(screen.getByTestId('task-card-bd-2'), screen.getByTestId('task-column-in_review'))
    await waitFor(() => expect(move).toHaveBeenCalledWith(inProgress, 'in_review'))
  })

  it('refuses drag-to-done for an agent that is not the reviewer', async () => {
    const move = vi.fn(async () => {})
    const agentNotReviewer: TaskViewer = {
      username: 'codex',
      role: 'member',
      agentIds: new Set(['claude', 'codex']),
    }
    renderBoard([inReview], { onMove: move, viewer: agentNotReviewer })
    drag(screen.getByTestId('task-card-bd-3'), screen.getByTestId('task-column-done'))
    expect(await screen.findByRole('alert')).toBeTruthy()
    expect(screen.getByRole('alert').textContent).toContain('没有关闭该任务的权限')
    expect(move).not.toHaveBeenCalled()
  })

  it('allows a human to drag in_review to done', async () => {
    const move = vi.fn(async () => {})
    renderBoard([inReview], { onMove: move })
    drag(screen.getByTestId('task-card-bd-3'), screen.getByTestId('task-column-done'))
    await waitFor(() => expect(move).toHaveBeenCalledWith(inReview, 'done'))
  })

  it('allows the designated reviewer agent to drag in_review to done', async () => {
    // reviewer 闸门的正例：issue metadata 点名的 agent 与人同权（design D10
    // 的「agent 必须是该任务的 reviewer」在前端呈现侧的对照）。
    const move = vi.fn(async () => {})
    const reviewerAgent: TaskViewer = {
      username: 'claude',
      role: 'member',
      agentIds: new Set(['claude', 'codex']),
    }
    renderBoard([{ ...inReview, metadata: { reviewer: 'claude' } }], {
      onMove: move,
      viewer: reviewerAgent,
    })
    drag(screen.getByTestId('task-card-bd-3'), screen.getByTestId('task-column-done'))
    await waitFor(() =>
      expect(move).toHaveBeenCalledWith(expect.objectContaining({ id: 'bd-3' }), 'done'),
    )
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('surfaces the backend error when a move is rejected', async () => {
    const move = vi.fn(async () => {
      throw new Error('HTTP 409: 已被 claude 认领')
    })
    renderBoard([unassigned], { onMove: move })
    drag(screen.getByTestId('task-card-bd-1'), screen.getByTestId('task-column-in_progress'))
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toContain('已被 claude 认领'),
    )
  })
})
