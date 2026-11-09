/**
 * Journey — fix-webui-qa-round14 的 RBAC 半边（webui-user-management delta：
 * 「viewer 只读视图可读转录」「被拒操作明确呈现」「/sessions 页角色可见性」）。
 *
 * Runs ONLY under playwright.auth.config.ts（TESTSUITE_AUTH=1，端口 9898，
 * admin/admin 预置在沙箱本地 auth.db）。React 入口按路由聚焦会话（不再走
 * POST switch 写），因此 viewer 的零写副作用断言仍保留，member 的「必须有
 * switch」机制断言换成用户可见合同：会话可打开、composer 在位、读面可用。
 *
 *  1. viewer 打开会话：转录渲染、composer 让位（只读视图）、全程零 POST switch。
 *  2. /sessions 总览页：无写入口（新建/重命名/归档）、只读列表照常、副标题
 *     如实「只读总览」。
 *  3. 服务端执法半边（viewer cookie 直调）：POST /api/sessions 与
 *     POST .../switch 403（写档执法），GET detail 200（读面开放）。
 *  4. member 对照：同一会话可打开、composer 在位。
 *
 * 账号自愈：viewer/member 经 admin 的 users API 现场建（幂等，409 = 已就绪
 * ——agents-gate / provider-gate 同款纪律）；探针会话由 admin 在 beforeAll
 * 建（viewer/member 无 sessions.write，建会话必须由 root 档完成）。
 */
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import {
  authLogin,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  Login,
  waitStatus,
} from './helpers/index'
import { ReactRail, ReactSessionsPage, ReactShell, ReactWorkbench } from './helpers/pages/react'

const VIEWER = { username: 'r14-viewer', password: 'r14-viewer-123' }
const MEMBER = { username: 'r14-member', password: 'r14-member-123' }
/** 探针会话的首条 prompt（行 preview 的可辨文本，行定位用）。 */
const PROBE_PROMPT = 'rbac-round14'

async function ensureUser(
  request: APIRequestContext,
  username: string,
  password: string,
  role: 'viewer' | 'member',
): Promise<void> {
  const resp = await request.post('/api/users', { data: { username, password, role } })
  expect([200, 201, 409], `ensureUser ${username}: HTTP ${resp.status()}`).toContain(resp.status())
}

test.describe('viewer 只读口径（fix-webui-qa-round14，webui-user-management）', () => {
  let collector: ErrorCollector
  /** beforeAll 建的探针会话（编码键）与所属项目名。 */
  let probeKey = ''
  let projectName = ''

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test.beforeAll(async ({ request }) => {
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    await ensureUser(request, VIEWER.username, VIEWER.password, 'viewer')
    await ensureUser(request, MEMBER.username, MEMBER.password, 'member')
    projectName = (await ensureSceneProject(request)).name
    probeKey = await createSession(request, { prompt: PROBE_PROMPT })
    await waitStatus(request, probeKey, ['done'])
  })

  function trackSwitchPosts(page: Page): string[] {
    const switchPosts: string[] = []
    page.on('request', (r) => {
      if (r.method() === 'POST' && r.url().includes('/switch')) switchPosts.push(r.url())
    })
    return switchPosts
  }

  async function focusProbe(page: Page): Promise<void> {
    const rail = new ReactRail(page)
    await rail.ensureProjectExpanded(projectName)
    await rail.sessionRow(PROBE_PROMPT).click()
  }

  test('viewer rail 点击 = 纯 GET 只读视图：不发 switch、转录在、composer 让位', async ({
    page,
  }) => {
    const switchPosts = trackSwitchPosts(page)
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.getByTestId('react-shell')).toBeVisible({ timeout: 15_000 })

    await focusProbe(page)

    // 直落该会话的只读视图（无 switch 写、无幻影：URL 即该会话深链）。
    await expect(page).toHaveURL(new RegExp(`/sessions/${probeKey}`), { timeout: 15_000 })
    // 转录照常渲染（GET detail 读面）；composer 整体让位给只读说明。
    const workbench = new ReactWorkbench(page)
    await expect(workbench.transcript).toBeVisible({ timeout: 15_000 })
    await expect(page.getByTestId('composer')).toHaveCount(0)
    // 「打开」零写副作用：全程无 POST switch。
    expect(switchPosts, 'viewer opening a session must not POST switch').toEqual([])
  })

  test('viewer 的 /sessions 总览页：无写入口、只读列表照常、副标题如实', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.getByTestId('react-shell')).toBeVisible({ timeout: 15_000 })

    await page.goto('/sessions')
    const overview = new ReactSessionsPage(page)
    // 新建入口不呈现；副标题如实「只读总览」。
    await expect(overview.pageRoot.getByRole('button', { name: '新建会话' })).toHaveCount(0, {
      timeout: 15_000,
    })
    await expect(overview.pageRoot).toContainText('只读总览')
    // 只读列表照常渲染；写操作按钮（重命名/归档）不呈现。
    const card = overview.pageRoot.locator('.ant-card').first()
    await expect(card).toBeVisible({ timeout: 15_000 })
    await expect(card.getByRole('button', { name: '重命名' })).toHaveCount(0)
    await expect(card.getByRole('button', { name: /归\s*档/ })).toHaveCount(0)
    await expect(card.getByRole('button', { name: /聚\s*焦/ })).toBeVisible()
  })

  test('服务端执法半边：viewer 写 403 / 读 200（cookie 直调）', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.getByTestId('react-shell')).toBeVisible({ timeout: 15_000 })

    // 创建（sessions.write）403；同会话 switch 403；GET detail 200（读面开放
    // ——「viewer 打开会话走纯 GET」的执法前提）。
    const create = await page.request.post('/api/sessions', {
      data: { prompt: 'nope', agent: 'claude' },
    })
    expect(create.status()).toBe(403)
    const switchResp = await page.request.post(`/api/sessions/${probeKey}/switch`)
    expect(switchResp.status()).toBe(403)
    const detail = await page.request.get(`/api/sessions/${probeKey}`)
    expect(detail.status()).toBe(200)
  })

  test('member 对照：rail 点击可打开会话、composer 在位', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(MEMBER.username, MEMBER.password)
    await expect(page.getByTestId('react-shell')).toBeVisible({ timeout: 15_000 })

    await focusProbe(page)
    await expect(page).toHaveURL(new RegExp(`/sessions/${probeKey}`), { timeout: 15_000 })

    // composer 正常呈现（viewer 用例的对照面）。
    await expect(page.getByTestId('composer')).toBeVisible({ timeout: 15_000 })
  })
})
