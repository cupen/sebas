/**
 * Journey — fix-webui-qa-round14 的 RBAC 半边（webui-user-management delta：
 * 「viewer 只读视图可读转录」「被拒操作明确呈现」「/sessions 页角色可见性」）。
 *
 * Runs ONLY under playwright.auth.config.ts（TESTSUITE_AUTH=1，端口 9898，
 * admin/admin 预置在沙箱本地 auth.db）。单测已有的组件半边（dashboard.test
 * viewer 只读两用例 / sessions.test 角色裁剪三用例 / reportActionRejection
 * 两用例）之外，这里以真实登录会话 + 真渲染钉住：
 *
 *  1. viewer 打开会话 = 纯 GET 只读视图：rail 点击直落深链，**不发 POST
 *     switch**（请求级监听为证）；转录视图照常渲染；composer 整体让位给
 *     只读说明（composer-viewer-readonly）。
 *  2. /sessions 总览页：新建表单与卡片写按钮（聚焦/关闭）不呈现，只读列表
 *     照常渲染，副标题如实「只读总览」。
 *  3. 服务端执法半边（viewer cookie 直调）：POST /api/sessions 与
 *     POST .../switch 403（写档执法），GET detail 200（读面开放）——「打开
 *     不触发写」的执法前提。403 直调走 context 级 request，不进页面 console
 *     （净空断言不受污染，agents-gate 同款纪律）。
 *  4. member 对照：同一会话 rail 点击走 switch 写（请求在场）、composer 在。
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
  ProjectRail,
  waitStatus,
} from './helpers/index'

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

  test('viewer rail 点击 = 纯 GET 只读视图：不发 switch、转录在、composer 让位', async ({
    page,
  }) => {
    const switchPosts = trackSwitchPosts(page)
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    const rail = new ProjectRail(page)
    await rail.ensureProjectExpanded(projectName)
    await rail.host
      .locator('li.session-item:not(.archived)', { hasText: PROBE_PROMPT })
      .first()
      .click()

    // 直落该会话的只读视图（无 switch 写、无幻影：URL 即该会话深链）。
    await expect(page).toHaveURL(new RegExp(`/sessions/${probeKey}`), { timeout: 15_000 })
    // 转录照常渲染（GET detail 读面）；composer 整体让位给只读说明。
    await expect(page.locator('sebas-transcript-view')).toBeVisible({ timeout: 15_000 })
    await expect(
      page.locator('sebas-dashboard [data-testid="composer-viewer-readonly"]'),
    ).toBeVisible()
    await expect(page.locator('sebas-workbench-composer')).toHaveCount(0)
    // 「打开」零写副作用：全程无 POST switch。
    expect(switchPosts, 'viewer opening a session must not POST switch').toEqual([])
  })

  test('viewer 的 /sessions 总览页：无写入口、只读列表照常、副标题如实', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    await page.goto('/sessions')
    const host = page.locator('sebas-sessions')
    // 新建表单不呈现；副标题如实「只读总览——你的角色没有会话写权限。」
    await expect(host.locator('form.composer')).toHaveCount(0, { timeout: 15_000 })
    await expect(host).toContainText('只读总览')
    // 只读列表照常渲染（卡片 + 标题链接），写操作按钮（聚焦/关闭）不呈现。
    const card = host.locator('article.scard').first()
    await expect(card).toBeVisible({ timeout: 15_000 })
    await expect(card.locator('a.chat')).toBeVisible()
    await expect(card.locator('wa-button')).toHaveCount(0)
  })

  test('服务端执法半边：viewer 写 403 / 读 200（cookie 直调）', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

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

  test('member 对照：rail 点击走 switch 写、composer 在位', async ({ page }) => {
    const switchPosts = trackSwitchPosts(page)
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(MEMBER.username, MEMBER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    const rail = new ProjectRail(page)
    await rail.ensureProjectExpanded(projectName)
    await rail.host
      .locator('li.session-item:not(.archived)', { hasText: PROBE_PROMPT })
      .first()
      .click()
    await expect(page).toHaveURL(new RegExp(`/sessions/${probeKey}`), { timeout: 15_000 })

    // switch 写真实发生（viewer 用例的对照面），composer 正常呈现。
    expect(switchPosts.length).toBeGreaterThanOrEqual(1)
    await expect(page.locator('sebas-workbench-composer')).toBeVisible({ timeout: 15_000 })
    await expect(
      page.locator('sebas-dashboard [data-testid="composer-viewer-readonly"]'),
    ).toHaveCount(0)
  })
})
