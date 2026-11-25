/**
 * 任务中心 GUI 验收（agent-task-center 需求 6/8/10/11 + agent-workbench 三条）。
 *
 * 真实浏览器跑 React 生产入口（`playwright.react.config.ts`：Vite 5290 代理到
 * 沙箱 9893），后端是**真的 beads 仓库**：每个用例在沙箱场景目录下新建自己
 * 的项目子目录，首访任务面板时由 WebUI 惰性 `bd init`，所有流转都走真实的
 * `bd` 子进程与 journal 事件泵。
 *
 * 判定锚点（都是产品自己的语义面，不是实现细节）：
 * - 看板/详情：`data-testid`（task-board / task-column-* / task-card-* /
 *   task-detail / task-start-session / session-task-link）；
 * - 实时性：WS `task_updated` 帧（`page.on('websocket')` 原始帧）+ 看板自动
 *   更新，全程不刷新页面；
 * - 身份：`BEADS_ACTOR` 钉死 `gui-test`，认领后的 assignee 可精确断言。
 *
 * 4 个用例：拖拽认领 / WS 亚秒刷新 / 从任务建房并切回会话 / 会话回链跳任务。
 */
import fs from 'node:fs'
import path from 'node:path'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import {
  addProject,
  ErrorCollector,
  getSession,
  listProjects,
  pickSelectOption,
  ReactRail,
  sceneDir,
} from './helpers/index'

/** 沙箱把 `BEADS_ACTOR` 钉成 gui-test（invoke 参数/环境）；缺省同值。 */
const ACTOR = (process.env.BEADS_ACTOR ?? '').trim() || 'gui-test'

interface TaskProject {
  id: string
  name: string
  dir: string
}

interface Issue {
  id: string
  title: string
  status: string
  assignee?: string | null
  labels?: string[]
}

/**
 * 每个用例一个专属项目目录（在沙箱场景根内 → 注册边界与 `.beads` 都在沙箱
 * 里，不会向上发现操作员真实的 `.beads`）。目录名即项目名（后端取 basename）。
 */
async function registerTaskProject(
  request: APIRequestContext,
  tag: string,
): Promise<TaskProject> {
  const dir = path.join(sceneDir(), `task-gui-${tag}-${Date.now()}`)
  fs.mkdirSync(dir, { recursive: true })
  await addProject(request, dir)
  const real = fs.realpathSync(dir)
  const project = (await listProjects(request)).find(
    (item) => item.path === dir || item.path === real,
  )
  if (project === undefined) throw new Error(`project not registered: ${dir}`)
  return { id: project.id, name: path.basename(dir), dir }
}

function taskPath(projectId: string, taskId: string): string {
  return `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}`
}

async function createTask(
  request: APIRequestContext,
  projectId: string,
  title: string,
): Promise<Issue> {
  const response = await request.post(`/api/projects/${encodeURIComponent(projectId)}/tasks`, {
    data: { title },
  })
  if (!response.ok()) {
    throw new Error(`createTask failed: HTTP ${response.status()} ${await response.text()}`)
  }
  return (await response.json()) as Issue
}

async function claimTask(
  request: APIRequestContext,
  projectId: string,
  taskId: string,
): Promise<Issue> {
  const response = await request.post(`${taskPath(projectId, taskId)}/claim`)
  if (!response.ok()) {
    throw new Error(`claimTask failed: HTTP ${response.status()} ${await response.text()}`)
  }
  return (await response.json()) as Issue
}

async function showTask(
  request: APIRequestContext,
  projectId: string,
  taskId: string,
): Promise<Issue> {
  const response = await request.get(taskPath(projectId, taskId))
  if (!response.ok()) {
    throw new Error(`showTask failed: HTTP ${response.status()} ${await response.text()}`)
  }
  return (await response.json()) as Issue
}

/**
 * UI 路径进入看板：选中项目（rail 行点击既选中又展开）→ 切到「任务」tab →
 * 等看板落下。首访时后端在这里惰性 `bd init`，故给足超时。
 */
async function openBoard(page: Page, projectName: string): Promise<ReactRail> {
  const rail = new ReactRail(page)
  await page.goto('/')
  const row = rail.projectRow(projectName)
  await expect(row).toBeVisible({ timeout: 15_000 })
  await row.click()
  await expect(page.getByTestId('stage-tabs')).toBeVisible({ timeout: 15_000 })
  await page.getByTestId('stage-tab-tasks').click()
  await expect(page.getByTestId('stage-tab-tasks')).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('task-board')).toBeVisible({ timeout: 20_000 })
  return rail
}

/** 会话详情（含 `task_id`，helpers 的类型面尚未收录，按 wire 事实断言）。 */
async function sessionTaskId(
  request: APIRequestContext,
  encodedKey: string,
): Promise<string | null | undefined> {
  const { status, detail } = await getSession(request, encodedKey)
  if (detail === null) throw new Error(`GET session failed: HTTP ${status}`)
  return (detail as unknown as { task_id?: string | null }).task_id
}

// 失败证据（截图 + trace）留在本文件自己的产物里，不改装配的全局 `use`。
test.use({ screenshot: 'only-on-failure', trace: 'retain-on-failure' })

test.describe('task center GUI', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('用例 1：把卡片从「未认领」拖到「进行中」触发 claim，卡片落到进行中并由当前操作员持有', async ({
    page,
  }) => {
    const project = await registerTaskProject(page.request, 'drag')

    // 首访任务面板：bd init 自动完成，空看板三列为 0。
    await openBoard(page, project.name)
    await expect(page.getByTestId('task-count-unassigned')).toHaveText('0', { timeout: 20_000 })
    await expect(page.getByTestId('task-count-in_progress')).toHaveText('0')

    const task = await createTask(page.request, project.id, `拖拽认领 ${Date.now()}`)
    const card = page.getByTestId(`task-card-${task.id}`)
    await expect(card).toBeVisible({ timeout: 20_000 })
    await expect(page.getByTestId('task-column-unassigned').getByTestId(`task-card-${task.id}`)).toBeVisible()
    await expect(card).toContainText('未认领')

    // 认领请求在拖动落地时发出：先挂监听，再拖。
    const claimUrl = `${taskPath(project.id, task.id)}/claim`
    const claimRequest = page.waitForRequest(
      (request) => request.method() === 'POST' && request.url().endsWith(claimUrl),
      { timeout: 20_000 },
    )
    await card.dragTo(page.getByTestId('task-column-in_progress'))
    const claim = await claimRequest
    expect(claim.method()).toBe('POST')
    expect(claim.url()).toContain(`/${task.id}/claim`)

    // 卡片移列 + 计数跟随 + 认领人显示当前操作员。
    await expect(page.getByTestId('task-column-in_progress').getByTestId(`task-card-${task.id}`)).toBeVisible({
      timeout: 20_000,
    })
    await expect(page.getByTestId('task-column-unassigned').getByTestId(`task-card-${task.id}`)).toHaveCount(0)
    await expect(page.getByTestId('task-count-unassigned')).toHaveText('0')
    await expect(page.getByTestId('task-count-in_progress')).toHaveText('1')
    await expect(page.getByTestId(`task-card-${task.id}`)).toContainText(ACTOR)
    await expect(page.getByTestId(`task-card-${task.id}`)).not.toContainText('未认领')

    // 服务端真相：bd 侧 status/assignee 与看板一致（不是纯前端乐观态）。
    const stored = await showTask(page.request, project.id, task.id)
    expect(stored.status).toBe('in_progress')
    expect(stored.assignee).toBe(ACTOR)

    expect(collector.clean()).toEqual([])
  })

  test('用例 2：另一个页签的看板经 WS task_updated 在 1 秒内自动更新（无需刷新）', async ({
    page,
    context,
  }) => {
    test.setTimeout(90_000)
    const project = await registerTaskProject(page.request, 'live')
    const seed = await createTask(page.request, project.id, `实时种子 ${Date.now()}`)

    // 页签 B 先开，先收集 WS 原始帧（证明更新来自推送而非轮询/刷新）。
    const pageB = await context.newPage()
    const frames: string[] = []
    const socketUrls: string[] = []
    pageB.on('websocket', (socket) => {
      socketUrls.push(socket.url())
      socket.on('framereceived', (frame) => {
        if (typeof frame.payload === 'string') frames.push(frame.payload)
      })
    })
    const boardB = pageB.getByTestId(`task-card-${seed.id}`)
    await openBoard(pageB, project.name)
    await expect(boardB).toBeVisible({ timeout: 20_000 })
    await expect(boardB).toContainText('未认领')

    // 页签 A 也进入同一项目看板（两个订阅者同时在线）。
    await openBoard(page, project.name)
    await expect(page.getByTestId(`task-card-${seed.id}`)).toBeVisible({ timeout: 20_000 })

    // 页签 A 认领 → 页签 B 不刷新、不点刷新按钮，自行更新。
    const beforePost = Date.now()
    const claimed = await claimTask(page.request, project.id, seed.id)
    expect(claimed.assignee).toBe(ACTOR)
    const pushedAt = Date.now()

    await expect(boardB).toContainText(ACTOR, { timeout: 5_000 })
    const pushLatency = Date.now() - pushedAt
    const totalLatency = Date.now() - beforePost
    test.info().annotations.push({
      type: 'ws-latency-ms',
      description: `push=${pushLatency} total=${totalLatency}`,
    })
    console.log(`[task-center] WS push latency ${pushLatency}ms (round-trip+commit ${totalLatency}ms)`)

    expect(pushLatency, `WS 推送延迟 ${pushLatency}ms 应 ≤ 1000ms`).toBeLessThanOrEqual(1000)

    // 列迁移同样实时跟随。
    await expect(pageB.getByTestId('task-column-in_progress').getByTestId(`task-card-${seed.id}`)).toBeVisible({
      timeout: 5_000,
    })
    await expect(pageB.getByTestId('task-count-in_progress')).toHaveText('1')

    // 帧证据：确实有 task_updated 通知携带该 issue 快照；B 页签从未 reload。
    expect(socketUrls.some((url) => url.includes('/ws')), `ws urls: ${socketUrls.join(',')}`).toBe(
      true,
    )
    expect(
      frames.some((frame) => frame.includes('"task_updated"') && frame.includes(seed.id)),
      '应收到携带该 issue 的 task_updated 帧',
    ).toBe(true)
    expect(pageB.url()).toContain('127.0.0.1:5290')

    await pageB.close()
  })

  test('用例 3：任务详情「开一轮会话推进此任务」创建绑定 task_id 的会话并切回会话视图', async ({
    page,
  }) => {
    test.setTimeout(90_000)
    const project = await registerTaskProject(page.request, 'start')
    const task = await createTask(page.request, project.id, `推进任务 ${Date.now()}`)
    await claimTask(page.request, project.id, task.id)

    const rail = await openBoard(page, project.name)
    await page.getByTestId(`task-card-${task.id}`).click()
    const detail = page.getByTestId('task-detail')
    await expect(detail).toBeVisible({ timeout: 20_000 })
    await expect(detail.locator('code').first()).toHaveText(task.id)

    await page.getByTestId('task-start-session').click()
    // 复用既有创建对话框，标题带任务上下文（spec「Dialog opens with the task context」）。
    const dialog = page.getByRole('dialog', { name: `推进任务 ${task.id}：${task.title}` })
    await expect(dialog).toBeVisible({ timeout: 15_000 })
    await pickSelectOption(page, dialog.getByRole('combobox', { name: 'Agent' }), 'claude')
    const prompt = `推进 ${task.id}`
    await dialog.getByRole('textbox', { name: '任务描述' }).fill(prompt)
    await dialog.getByRole('button', { name: /创\s*建/ }).click()

    // 确认后 stage 切回会话视图（详情退出），URL 指向新会话。
    await expect(page).toHaveURL(/\/sessions\/[^/]+$/, { timeout: 30_000 })
    await expect(page.getByTestId('stage-tab-conversation')).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('workbench-session')).toBeVisible({ timeout: 30_000 })
    await expect(page.getByTestId('task-detail')).toHaveCount(0)

    const encodedKey = page.url().split('/sessions/')[1]
    expect(encodedKey.length).toBeGreaterThan(0)
    // session_map 行的 task_id 正确（服务端真相，不是前端记忆）。
    expect(await sessionTaskId(page.request, encodedKey)).toBe(task.id)

    // 新会话出现在 rail 对应项目下（项目行需先展开）。
    await rail.ensureProjectExpanded(project.name)
    await expect(rail.sessionRow(prompt)).toBeVisible({ timeout: 20_000 })

    expect(collector.clean()).toEqual([])
  })

  test('用例 4：会话头「Task <id>」回链切到任务看板并打开该任务详情', async ({ page }) => {
    test.setTimeout(90_000)
    const project = await registerTaskProject(page.request, 'backlink')
    const task = await createTask(page.request, project.id, `回链任务 ${Date.now()}`)
    await claimTask(page.request, project.id, task.id)

    // 准备「已绑 task_id 的会话」（用例 3 之后的同一状态，这里经 API 直达）。
    const prompt = `推进 ${task.id}`
    const created = await page.request.post('/api/sessions', {
      data: { prompt, project_id: project.id, agent: 'claude', task_id: task.id },
    })
    expect(created.ok(), `create session: HTTP ${created.status()}`).toBe(true)
    const { key } = (await created.json()) as { key: string }

    // 从 rail 进入会话（选中项目 + 聚焦该会话，与操作员路径一致）。
    const rail = new ReactRail(page)
    await page.goto('/')
    await expect(rail.projectRow(project.name)).toBeVisible({ timeout: 15_000 })
    await rail.projectRow(project.name).click()
    await rail.ensureProjectExpanded(project.name)
    await rail.sessionRow(prompt).click()
    await expect(page.getByTestId('workbench-session')).toBeVisible({ timeout: 30_000 })

    // 会话头出现回链（rail 行本体不显示任务链接）。
    const link = page.getByTestId('session-task-link')
    await expect(link).toBeVisible({ timeout: 30_000 })
    await expect(link).toContainText(`Task ${task.id}`)
    expect(await sessionTaskId(page.request, key)).toBe(task.id)

    await link.click()
    // stage 切到 Tasks tab 并落到该任务详情。
    await expect(page.getByTestId('stage-tab-tasks')).toHaveAttribute('aria-pressed', 'true')
    const detail = page.getByTestId('task-detail')
    await expect(detail).toBeVisible({ timeout: 20_000 })
    await expect(detail.locator('code').first()).toHaveText(task.id)
    await expect(detail.getByRole('heading', { level: 3 })).toHaveText(task.title)

    // 回链是双向的：详情页的关联会话列表里能点回该会话。
    await expect(detail.getByTestId('task-linked-sessions')).toContainText(/推进 /)

    expect(collector.clean()).toEqual([])
  })
})
