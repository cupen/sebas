import { describe, expect, it } from 'vitest'
import type { BeadsIssue, TaskListResponse } from '../../api/client.js'
import type { WsEvent } from '../../api/ws.js'
import { TaskStore, taskColumn, type TaskApi } from './task-store.js'

function issue(id: string, patch: Partial<BeadsIssue> = {}): BeadsIssue {
  return { id, title: `任务 ${id}`, status: 'open', labels: [], ...patch }
}

function seam(overrides: Partial<TaskApi> = {}) {
  const calls = {
    list: [] as string[],
    refresh: [] as string[],
    watch: [] as string[],
    unwatch: [] as string[],
  }
  const tasks: BeadsIssue[] = []
  const api: TaskApi = {
    list: async (projectId) => {
      calls.list.push(projectId)
      return { tasks: tasks.slice() } satisfies TaskListResponse
    },
    refresh: async (projectId) => {
      calls.refresh.push(projectId)
      return { tasks: tasks.slice() } satisfies TaskListResponse
    },
    watch: async (projectId) => {
      calls.watch.push(projectId)
    },
    unwatch: async (projectId) => {
      calls.unwatch.push(projectId)
    },
    ...overrides,
  }
  return { api, calls, tasks }
}

const updated = (projectId: string, snapshot: BeadsIssue): WsEvent =>
  ({
    type: 'task_updated',
    project_id: projectId,
    seq: 1,
    op: 'update',
    issue_id: snapshot.id,
    issue: snapshot,
  }) as WsEvent

describe('taskColumn', () => {
  it('classifies beads states into board columns', () => {
    expect(taskColumn(issue('a'))).toBe('unassigned')
    expect(taskColumn(issue('b', { status: 'in_progress' }))).toBe('in_progress')
    expect(taskColumn(issue('c', { assignee: 'claude' }))).toBe('in_progress')
    expect(taskColumn(issue('d', { status: 'in_progress', labels: ['in-review'] }))).toBe('in_review')
    expect(taskColumn(issue('e', { status: 'closed' }))).toBe('done')
    expect(taskColumn(issue('f', { status: 'closed', close_reason: 'cancelled' }))).toBe('cancelled')
  })
})

describe('TaskStore', () => {
  it('subscribes and loads the project on first entry, and unwatches on release', async () => {
    const { api, calls, tasks } = seam()
    tasks.push(issue('bd-1'), issue('bd-2', { assignee: 'claude' }))
    const store = new TaskStore(api)
    await store.ensure('p1')
    expect(calls.watch).toEqual(['p1'])
    expect(calls.list).toEqual(['p1'])
    const bucket = store.get('p1')
    expect(bucket.watching).toBe(true)
    expect(bucket.loaded).toBe(true)
    expect(bucket.tasks.map((task) => task.id)).toEqual(['bd-1', 'bd-2'])
    await store.release('p1')
    expect(calls.unwatch).toEqual(['p1'])
    expect(store.get('p1').watching).toBe(false)
  })

  it('replaces local state from a WS snapshot without refetching', async () => {
    const { api, calls, tasks } = seam()
    tasks.push(issue('bd-1'))
    const store = new TaskStore(api)
    await store.ensure('p1')
    store.applyEvent(updated('p1', issue('bd-1', { status: 'in_progress', assignee: 'cupen' })))
    expect(store.get('p1').tasks[0].assignee).toBe('cupen')
    expect(calls.list).toEqual(['p1'])
  })

  it('buffers snapshots that arrive during the initial load and merges them after', async () => {
    let release!: (value: TaskListResponse) => void
    const list = new Promise<TaskListResponse>((resolve) => {
      release = resolve
    })
    const { api } = seam({ list: () => list })
    const store = new TaskStore(api)
    const loading = store.load('p1')
    store.applyEvent(updated('p1', issue('bd-2', { title: '缓冲标题' })))
    release({ tasks: [issue('bd-1'), issue('bd-2', { title: '旧标题' })] })
    await loading
    const merged = store.get('p1').tasks.find((task) => task.id === 'bd-2')
    expect(merged?.title).toBe('缓冲标题')
  })

  it('ignores pushes for projects without an open panel', () => {
    const { api } = seam()
    const store = new TaskStore(api)
    store.applyEvent(updated('p2', issue('bd-9')))
    expect(store.get('p2').tasks).toEqual([])
  })

  it('reloads the full list on task_resync', async () => {
    const { api, calls, tasks } = seam()
    tasks.push(issue('bd-1'))
    const store = new TaskStore(api)
    await store.ensure('p1')
    tasks.push(issue('bd-2'))
    store.applyEvent({ type: 'task_resync', project_id: 'p1' } as WsEvent)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(calls.list).toEqual(['p1', 'p1'])
    expect(store.get('p1').tasks.map((task) => task.id)).toEqual(['bd-1', 'bd-2'])
  })

  it('applies a task created by another client (multi-tab convergence)', async () => {
    // 多 tab/多客户端：本端已载入空表，另一客户端建了任务 → journal 推送
    // 的完整快照必须插进本地列表（不是被丢弃），两端看到同一份状态。
    const { api } = seam()
    const store = new TaskStore(api)
    await store.ensure('p1')
    expect(store.get('p1').tasks).toEqual([])
    store.applyEvent(updated('p1', issue('bd-new', { status: 'open' })))
    expect(store.get('p1').tasks.map((task) => task.id)).toEqual(['bd-new'])
  })

  it('keeps project buckets isolated when two panels are open', async () => {
    // 两个 tab 各看一个项目：一个项目的推送不得污染另一个项目的看板。
    const { api } = seam()
    const store = new TaskStore(api)
    await store.ensure('p1')
    await store.ensure('p2')
    store.applyEvent(updated('p1', issue('bd-1', { title: 'P1 的' })))
    expect(store.get('p1').tasks.map((task) => task.id)).toEqual(['bd-1'])
    expect(store.get('p2').tasks).toEqual([])
    expect(store.get('p2').loaded).toBe(true)
  })

  it('manual refresh re-baselines through the refresh endpoint', async () => {
    const { api, calls, tasks } = seam()
    tasks.push(issue('bd-1'))
    const store = new TaskStore(api)
    await store.ensure('p1')
    await store.refresh('p1')
    expect(calls.refresh).toEqual(['p1'])
    expect(store.get('p1').loaded).toBe(true)
  })
})
