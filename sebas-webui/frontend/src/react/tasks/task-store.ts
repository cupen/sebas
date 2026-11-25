/**
 * 任务面板 store（agent-task-center 7.5）：按项目持有 beads 任务快照。
 *
 * 数据面两条线：
 * - REST 全量（首次进入 / 手动 Refresh / `task_resync`）——权威列表；
 * - WS `task_updated`（journal 记录内的**完整 issue 快照**）——直接替换本地
 *   对应任务，不做 `bd show` 回查（spec「no bd show re-fetch」）。
 *
 * 订阅生命周期：面板进入时 `tasks.watch`，离开时 `tasks.unwatch`（后端每项目
 * 一条事件泵，订阅者掉线即退出）。
 */

import { api, type BeadsIssue, type TaskListResponse } from '../../api/client.js'
import { sharedWs } from '../../api/shared-ws.js'
import type { WsEvent } from '../../api/ws.js'
import { Generation, createStore, type Store } from '../state/store.js'

export interface ProjectTasks {
  projectId: string
  /** 首次请求进行中。 */
  loading: boolean
  /** 加载/刷新失败原因（就地呈现，不弹全局通知）。 */
  error: string | null
  tasks: BeadsIssue[]
  /** 是否已成功拿到过一次列表（空表 ≠ 未加载）。 */
  loaded: boolean
  /** 本面板是否持有该项目的 WS 订阅。 */
  watching: boolean
}

export interface TaskState {
  projects: Record<string, ProjectTasks>
}

/** Store 依赖的 API 缝（测试注入，生产走 `defaultTaskApi`）。 */
export interface TaskApi {
  list: (projectId: string) => Promise<TaskListResponse>
  refresh: (projectId: string) => Promise<TaskListResponse>
  watch: (projectId: string) => Promise<unknown>
  unwatch: (projectId: string) => Promise<unknown>
}

export const defaultTaskApi: TaskApi = {
  list: (projectId) => api.tasksList(projectId),
  refresh: (projectId) => api.tasksRefresh(projectId),
  watch: (projectId) => sharedWs.request('tasks.watch', { project_id: projectId }),
  unwatch: (projectId) => sharedWs.request('tasks.unwatch', { project_id: projectId }),
}

const EMPTY: TaskState = { projects: {} }

function bucket(projectId: string): ProjectTasks {
  return {
    projectId,
    loading: false,
    error: null,
    tasks: [],
    loaded: false,
    watching: false,
  }
}

/** 看板列判定用的标签集合兜底（wire 缺省该键 = 无标签，不是空数组）。 */
export function labelsOf(issue: BeadsIssue): string[] {
  return issue.labels ?? []
}

/** 任务所在看板列（spec 三列 + 折叠 Done/Cancelled）。 */
export type TaskColumn = 'unassigned' | 'in_progress' | 'in_review' | 'done' | 'cancelled'

export function taskColumn(issue: BeadsIssue): TaskColumn {
  if (issue.status === 'closed') {
    return issue.close_reason === 'cancelled' ? 'cancelled' : 'done'
  }
  if (labelsOf(issue).includes('in-review')) return 'in_review'
  if (issue.status === 'in_progress' || (issue.assignee ?? '').trim() !== '') {
    return 'in_progress'
  }
  return 'unassigned'
}

export class TaskStore {
  readonly store: Store<TaskState>
  private readonly api: TaskApi
  private readonly generations = new Map<string, Generation>()
  /**
   * REST 加载窗口内到达的 WS 快照：加载完成后覆盖同 id 的 REST 值（WS 永远
   * 更新——列表快照可能早于某条事件）。
   */
  private readonly pending = new Map<string, Map<string, BeadsIssue>>()
  /** 同一项目的并发全量载入只发一次请求（保证 await 到的是同一份结果）。 */
  private readonly inFlight = new Map<string, Promise<void>>()

  constructor(apiSeam: TaskApi = defaultTaskApi) {
    this.api = apiSeam
    this.store = createStore(EMPTY)
  }

  getState(): TaskState {
    return this.store.getState()
  }

  get(projectId: string): ProjectTasks {
    return this.store.getState().projects[projectId] ?? bucket(projectId)
  }

  /** 进入面板：订阅 + 全量载入（幂等；已订阅则不重复请求）。 */
  async ensure(projectId: string): Promise<void> {
    const current = this.store.getState().projects[projectId]
    if (current?.watching === true) {
      if (!current.loaded) await this.load(projectId)
      return
    }
    this.patch(projectId, (p) => ({ ...p, watching: true }))
    try {
      await this.api.watch(projectId)
    } catch (error) {
      // 订阅失败不阻断 REST 面：面板仍可读，只是不实时。
      this.patch(projectId, (p) => ({ ...p, watching: false }))
      console.warn('tasks.watch failed', error)
    }
    await this.load(projectId)
  }

  /** 离开面板：取消订阅（数据保留，下次进入立即可见并重新拉取）。 */
  async release(projectId: string): Promise<void> {
    if (this.store.getState().projects[projectId]?.watching !== true) return
    this.patch(projectId, (p) => ({ ...p, watching: false }))
    try {
      await this.api.unwatch(projectId)
    } catch {
      /* 连接已断：后端泵随接收者消失自行退出 */
    }
  }

  /** 全量载入（首次 / resync / 首次访问）。并发调用共用同一次请求。 */
  async load(projectId: string): Promise<void> {
    const inflight = this.inFlight.get(projectId)
    if (inflight !== undefined) return inflight
    const promise = this.reload(projectId, () => this.api.list(projectId))
    this.inFlight.set(projectId, promise)
    try {
      await promise
    } finally {
      this.inFlight.delete(projectId)
    }
  }

  /** 手动 Refresh：全量重载 + 事件游标重新基线（后端一并完成）。 */
  async refresh(projectId: string): Promise<void> {
    return this.reload(projectId, () => this.api.refresh(projectId))
  }

  private async reload(
    projectId: string,
    fetchList: () => Promise<TaskListResponse>,
  ): Promise<void> {
    const token = this.generationFor(projectId).next()
    this.patch(projectId, (p) => ({ ...p, loading: true, error: null }))
    try {
      const response = await fetchList()
      if (!this.generationFor(projectId).isCurrent(token)) return
      const tasks = this.mergePending(projectId, response.tasks)
      this.patch(projectId, (p) => ({
        ...p,
        loading: false,
        error: null,
        tasks,
        loaded: true,
      }))
    } catch (error) {
      if (!this.generationFor(projectId).isCurrent(token)) return
      this.patch(projectId, (p) => ({
        ...p,
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      }))
    }
  }

  /** 本地乐观替换（HTTP 动作成功后调用；随后的 WS 帧会再覆盖一次）。 */
  upsert(projectId: string, issue: BeadsIssue): void {
    this.patch(projectId, (p) => ({ ...p, tasks: replaceIssue(p.tasks, issue) }))
  }

  /**
   * WS 事件入口（shell 的单一订阅分发给这里）：
   * - `task_updated`：用 journal 的完整快照替换本地任务（issue 缺省 = 保持）；
   * - `task_resync`：全量重取（丢帧/落后收敛）。
   */
  applyEvent(event: WsEvent): void {
    if (event.type === 'task_updated') {
      const issue = event.issue ?? null
      if (issue === null) return
      const project = this.store.getState().projects[event.project_id]
      if (project === undefined) {
        // 未打开的任务面板不缓存事件；打开时以 REST 全量为准。
        return
      }
      if (!project.loaded) {
        this.pendingFor(event.project_id).set(issue.id, issue)
        return
      }
      this.upsert(event.project_id, issue)
      return
    }
    if (event.type === 'task_resync') {
      void this.load(event.project_id)
    }
  }

  private generationFor(projectId: string): Generation {
    let generation = this.generations.get(projectId)
    if (generation === undefined) {
      generation = new Generation()
      this.generations.set(projectId, generation)
    }
    return generation
  }

  private pendingFor(projectId: string): Map<string, BeadsIssue> {
    let pending = this.pending.get(projectId)
    if (pending === undefined) {
      pending = new Map()
      this.pending.set(projectId, pending)
    }
    return pending
  }

  private mergePending(projectId: string, tasks: BeadsIssue[]): BeadsIssue[] {
    const pending = this.pending.get(projectId)
    if (pending === undefined || pending.size === 0) return tasks
    let merged = tasks
    for (const issue of pending.values()) merged = replaceIssue(merged, issue)
    pending.clear()
    return merged
  }

  private patch(projectId: string, update: (prev: ProjectTasks) => ProjectTasks): void {
    this.store.setState((prev) => ({
      projects: {
        ...prev.projects,
        [projectId]: update(prev.projects[projectId] ?? bucket(projectId)),
      },
    }))
  }
}

function replaceIssue(tasks: BeadsIssue[], issue: BeadsIssue): BeadsIssue[] {
  const index = tasks.findIndex((item) => item.id === issue.id)
  if (index === -1) return [...tasks, issue]
  const next = tasks.slice()
  next[index] = issue
  return next
}
