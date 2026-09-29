/**
 * Journey — gate-agent-directory-writes：agent 目录写执法（RBAC agents.manage
 * 行，settings.manage 档）与 viewer 写入口的呈现层隐藏。
 *
 * Runs ONLY under playwright.auth.config.ts（TESTSUITE_AUTH=1，端口 9898，
 * admin/admin 预置在沙箱本地 auth.db）。对照 spec「RBAC 角色与权限执法」
 * 逐场景：
 *   - viewer / member 会话对 /api/agents 增、改、删任一写操作 → 403，目录
 *     不变（第三轮 GUI QA 实测的越权写缺口，本旅程钉死不回退）；
 *   - admin（root 同档）写生效（建→删闭环）；
 *   - agent 目录读保持登录门：任何已认证角色 GET 200；
 *   - viewer 登录工作台：「新建会话」入口与 Settings → Agents 的写入口
 *     （New/Edit/Delete）不呈现；浏览器内直调写 API 仍 403（呈现层不是
 *     防线，防线在服务端）。
 *
 *   - 角色/禁用即时生效走**真实端到端**路径（spec「会话绑定用户」）：root 经
 *     users API 降级一个在线 admin，同一会话（cookie 不动、无需重登）的下一个
 *     agents 写即 403，me 实时反映新角色；恢复后同一会话写重新放行（反向
 *     证明拦的是角色不是会话）；降为 viewer 后会话写 403（字面场景）；经 API
 *     禁用后同会话下一请求 401、重登被拒。这是 agents-gate 与 users-admin
 *     （后者只断言面板徽标翻转）之间的组合缺口。
 *
 * 测试账号自愈：viewer/member 经 admin 的 users API 现场建（幂等，409 视为
 * 已就绪）——与 users-admin.spec 的 beforeAll 清场同款纪律。
 */
import { expect, test, type APIRequestContext } from '@playwright/test'
import {
  authLogin,
  authMe,
  ensureSceneProject,
  ErrorCollector,
  Login,
  ProjectRail,
  SettingsModal,
} from './helpers/index'

const VIEWER = { username: 'gate-viewer', password: 'viewer-pass-123' }
const MEMBER = { username: 'gate-member', password: 'member-pass-123' }
const PROBE_ID = 'gate-probe'

/** admin 经 users API 建低权账号（幂等：重名 409 = 已就绪）。 */
async function ensureUser(
  request: import('@playwright/test').APIRequestContext,
  username: string,
  password: string,
  role: 'viewer' | 'member',
): Promise<void> {
  const resp = await request.post('/api/users', { data: { username, password, role } })
  expect([200, 201, 409], `ensureUser ${username}: HTTP ${resp.status()}`).toContain(resp.status())
}

test.describe('agents 写执法（gate-agent-directory-writes）', () => {
  let collector: ErrorCollector

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
  })

  test('viewer/member 增改删一律 403 且目录不变；读保持登录门', async ({ request }) => {
    for (const user of [VIEWER, MEMBER]) {
      expect(await authLogin(request, user.username, user.password)).toBe(200)
      // 读：认证即可（agent 列表是选 agent 建会话的前置数据，只收写）。
      expect((await request.get('/api/agents')).status()).toBe(200)
      // 写三件套：403（服务端路由层执法）。
      expect(
        (await request.post('/api/agents', {
          data: { id: PROBE_ID, driver: 'claude', path: 'claude' },
        })).status(),
      ).toBe(403)
      expect(
        (await request.put('/api/agents/claude', { data: { display: 'hijacked' } })).status(),
      ).toBe(403)
      expect((await request.delete('/api/agents/claude')).status()).toBe(403)
    }
    // 目录未被改动：探针 id 不存在、既有 claude 条目原样在册。
    const catalog = (await (await request.get('/api/agents')).json()) as {
      agents: Array<{ id: string }>
    }
    expect(catalog.agents.map((a) => a.id)).not.toContain(PROBE_ID)
    expect(catalog.agents.map((a) => a.id)).toContain('claude')
  })

  test('admin 写生效（建→删闭环，settings.manage 同档）', async ({ request }) => {
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    const created = await request.post('/api/agents', {
      data: { id: PROBE_ID, driver: 'claude', path: 'claude' },
    })
    expect(created.status()).toBe(201)
    // 幂等清场 + 语义双断言：删除生效（200），再删得 404。
    expect((await request.delete(`/api/agents/${PROBE_ID}`)).status()).toBe(200)
    expect((await request.delete(`/api/agents/${PROBE_ID}`)).status()).toBe(404)
  })

  test('viewer 登录：New session 与 Agents 写入口不呈现；浏览器直调仍 403', async ({
    page,
    request,
  }) => {
    // admin 先注册 scene 项目（rail 有行，「+」的缺位才可断言）。
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    await ensureSceneProject(request)

    // viewer 走 UI 登录进入工作台。
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(VIEWER.username, VIEWER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    // 侧栏角色展示位（gate-agent-directory-writes 2.2）。
    await expect(
      page.locator('sebas-app .sidebar-footer .settings-btn', {
        hasText: `退出 (${VIEWER.username} · viewer)`,
      }),
    ).toBeVisible()

    // 新建会话入口（项目行「+」）不呈现；只读浏览不受影响（行本身在）。
    const rail = new ProjectRail(page)
    await expect(rail.host).toBeVisible()
    await page.reload()
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
    const rows = rail.host.locator('.row')
    await expect(rows.first()).toBeVisible({ timeout: 10_000 })
    await expect(rail.host.locator('button[aria-label^="New session"]')).toHaveCount(0)

    // Settings → Agents：分区可浏览（读），写入口全部不呈现。
    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agents')
    const modal = page.locator('sebas-settings-modal')
    await expect(modal.locator('wa-button').filter({ hasText: 'New agent' })).toHaveCount(0)
    await expect(modal.locator('[data-testid="agent-row"]').first()).toBeVisible()
    await expect(modal.locator('[data-testid="agent-row"] button[title="Edit"]')).toHaveCount(0)
    await expect(modal.locator('[data-testid="agent-row"] button[title="Delete"]')).toHaveCount(0)
    await settings.closeButton.click()
    await expect(settings.panel).toBeHidden()

    // 直调写 API：viewer 会话（cookie 随行）仍 403（呈现层隐藏不构成防线）。
    // 走 context 级 request 而非 page.evaluate fetch——同一 cookie 罐，且
    // 注定的 403 不进页面 console（ErrorCollector 的净空断言不受污染）。
    const probe = await page.request.post('/api/agents', {
      data: { id: PROBE_ID, driver: 'claude', path: 'claude' },
    })
    expect(probe.status()).toBe(403)
  })

  test('admin 登录：Agents 写入口在（对照面），行内 Edit/Delete 可见', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login('admin', 'admin')
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agents')
    const modal = page.locator('sebas-settings-modal')
    await expect(modal.locator('wa-button').filter({ hasText: 'New agent' })).toBeVisible()
    // config 种子的 claude 行带编辑/删除入口（呈现层与执法同档开放）。
    const row = modal.locator('[data-testid="agent-row"][data-id="claude"]')
    await expect(row).toBeVisible()
    await expect(row.locator('button[title="Edit"]')).toBeVisible()
    await expect(row.locator('button[title="Delete"]')).toBeVisible()
  })
})

/**
 * 角色/禁用即时生效（spec「会话绑定用户」：角色不快照进会话，按用户库实时
 * 解析）——agents 写路径的**真实端到端**闭环。服务端单测
 * `agents_write_resolves_role_and_enabled_live` 在 FakeBackend 路由层已钉同款
 * 语义（库直改），这里补的是单测够不到的两层：
 *   1. 降级走 root 的 users API 全链路（users_set_role 不踢会话——下一个
 *      请求按新角色执法），而非库直改；
 *   2. 被降级用户的会话 cookie 全程不动（「无需重新登录」的字面语义）——
 *      root 动作走独立的 APIRequestContext，被降级用户用测试自带 request。
 *
 * 串行 + 自愈：beforeAll 与 healLive 把探针用户重置为「启用 + 指定角色」，
 * 重试（新 worker 重跑 beforeAll）与中途失败残留都收敛回已知态。
 */
test.describe.serial('角色/禁用即时生效（agents 写，spec「会话绑定用户」）', () => {
  const LIVE = { username: 'gate-live', password: 'live-pass-12345' }
  const LIVE_PROBE = 'gate-live-probe'
  /** root 侧的独立 cookie 罐——降级/禁用/恢复都不碰被测用户的会话。 */
  let admin: APIRequestContext
  let liveId = 0

  /** 把探针用户收敛到「存在 + 启用 + 指定角色」（幂等，重试安全）。 */
  async function healLive(role: 'admin' | 'member' | 'viewer'): Promise<void> {
    expect(await authLogin(admin, 'admin', 'admin')).toBe(200)
    const { users } = (await (await admin.get('/api/users')).json()) as {
      users: Array<{ id: number; username: string }>
    }
    const known = users.find((u) => u.username.toLowerCase() === LIVE.username)
    if (!known) {
      const created = await admin.post('/api/users', {
        data: { username: LIVE.username, password: LIVE.password, role },
      })
      expect([200, 201], `healLive create: ${created.status()}`).toContain(created.status())
      const after = (await (await admin.get('/api/users')).json()) as {
        users: Array<{ id: number; username: string }>
      }
      liveId = after.users.find((u) => u.username.toLowerCase() === LIVE.username)!.id
    } else {
      liveId = known.id
      expect(
        (await admin.post(`/api/users/${liveId}/enabled`, { data: { enabled: true } })).status(),
      ).toBe(200)
      expect(
        (await admin.post(`/api/users/${liveId}/role`, { data: { role } })).status(),
      ).toBe(200)
    }
    expect(liveId, 'healLive 必须拿到探针用户 id').toBeGreaterThan(0)
  }

  test.beforeAll(async ({ playwright }) => {
    admin = await playwright.request.newContext()
    await healLive('admin')
  })

  test.afterAll(async () => {
    // 清场：探针用户连同其全部会话一并删除（users_delete 踢会话）；探针
    // agent 若因中途失败残留也一并摘除。
    if (!admin) return
    expect(await authLogin(admin, 'admin', 'admin')).toBe(200)
    const { users } = (await (await admin.get('/api/users')).json()) as {
      users: Array<{ id: number; username: string }>
    }
    const stale = users.find((u) => u.username.toLowerCase() === LIVE.username)
    if (stale) expect((await admin.delete(`/api/users/${stale.id}`)).status()).toBe(200)
    const catalog = (await (await admin.get('/api/agents')).json()) as {
      agents: Array<{ id: string }>
    }
    if (catalog.agents.some((a) => a.id === LIVE_PROBE)) {
      expect((await admin.delete(`/api/agents/${LIVE_PROBE}`)).status()).toBe(200)
    }
    await admin.dispose()
  })

  test('降级即时生效：admin 写通过 → root 降为 member → 同会话 agents 写 403，恢复后重新放行', async ({
    request,
  }) => {
    // request 罐 = LIVE 自己的会话（此后 root 的动作都在 admin 罐里，
    // 本罐不被触碰——「无需重新登录」的字面语义）。
    expect(await authLogin(request, LIVE.username, LIVE.password)).toBe(200)
    expect((await authMe(request)).role).toBe('admin')

    // 降级前：admin 档写走真实 core 路径生效（建→删闭环）。
    expect(
      (
        await request.post('/api/agents', {
          data: { id: LIVE_PROBE, driver: 'claude', path: 'claude' },
        })
      ).status(),
    ).toBe(201)
    expect((await request.delete(`/api/agents/${LIVE_PROBE}`)).status()).toBe(200)

    // root 经 users API 降级（users_set_role 不踢会话）。
    expect(
      (await admin.post(`/api/users/${liveId}/role`, { data: { role: 'member' } })).status(),
    ).toBe(200)

    // 同一会话、无需重登：me 实时反映新角色（角色不快照进会话）。
    expect((await authMe(request)).role).toBe('member')

    // 下一个 agents 写：三件套全 403（服务端路由层执法）。
    expect(
      (
        await request.post('/api/agents', {
          data: { id: LIVE_PROBE, driver: 'claude', path: 'claude' },
        })
      ).status(),
    ).toBe(403)
    expect((await request.put('/api/agents/claude', { data: { display: 'hijacked' } })).status()).toBe(
      403,
    )
    expect((await request.delete('/api/agents/claude')).status()).toBe(403)

    // 目录未被越权写弄脏：探针不在册、claude 原样在册（member 读仍放行）。
    const catalog = (await (await request.get('/api/agents')).json()) as {
      agents: Array<{ id: string }>
    }
    expect(catalog.agents.map((a) => a.id)).not.toContain(LIVE_PROBE)
    expect(catalog.agents.map((a) => a.id)).toContain('claude')

    // 恢复 admin：同一会话写重新放行（反向证明 403 拦的是角色，不是会话）。
    expect(
      (await admin.post(`/api/users/${liveId}/role`, { data: { role: 'admin' } })).status(),
    ).toBe(200)
    expect(
      (
        await request.post('/api/agents', {
          data: { id: LIVE_PROBE, driver: 'claude', path: 'claude' },
        })
      ).status(),
    ).toBe(201)
    expect((await request.delete(`/api/agents/${LIVE_PROBE}`)).status()).toBe(200)
  })

  test('降为 viewer：下一个会话写 403（spec「角色调整即时生效」字面场景），agents 写同拒', async ({
    request,
  }) => {
    // request 罐是本测试私有的——先以 LIVE 身份过登录门。
    expect(await authLogin(request, LIVE.username, LIVE.password)).toBe(200)
    // 前案已把 LIVE 恢复 admin；这里经 users API 再降到 viewer（字面场景的
    // 终态角色），agents 写与会话写都收口到 403。
    expect(
      (await admin.post(`/api/users/${liveId}/role`, { data: { role: 'viewer' } })).status(),
    ).toBe(200)

    // 读面不受角色影响：agent 目录读保持登录门（viewer 可浏览）。
    expect((await request.get('/api/agents')).status()).toBe(200)

    // 字面场景的「下一个写操作」：会话写 403（无需重新登录）。
    expect((await request.post('/api/sessions', { data: { agent: 'native' } })).status()).toBe(403)
    // agents 写同表（本 change 的执法行）。
    expect(
      (
        await request.post('/api/agents', {
          data: { id: LIVE_PROBE, driver: 'claude', path: 'claude' },
        })
      ).status(),
    ).toBe(403)

    // 收敛回 admin（afterAll 删户前不再依赖角色，但保持已知态）。
    expect(
      (await admin.post(`/api/users/${liveId}/role`, { data: { role: 'admin' } })).status(),
    ).toBe(200)
  })

  test('禁用即刻失效：同会话下一请求 401，重登被拒（agents 写路径）', async ({ request }) => {
    expect(await authLogin(request, LIVE.username, LIVE.password)).toBe(200)

    // root 经 users API 禁用（服务端立即踢掉该用户全部会话）。
    expect(
      (await admin.post(`/api/users/${liveId}/enabled`, { data: { enabled: false } })).status(),
    ).toBe(200)

    // 既有会话的下一个请求 401：agents 写与会话读都拦在登录门。
    expect(
      (
        await request.post('/api/agents', {
          data: { id: LIVE_PROBE, driver: 'claude', path: 'claude' },
        })
      ).status(),
    ).toBe(401)
    expect((await request.get('/api/agents')).status()).toBe(401)

    // 重新登录被拒绝（401，与凭据错误同文案——这里断状态码）。
    expect(await authLogin(request, LIVE.username, LIVE.password)).toBe(401)
  })

  test('浏览器组合旅程：降级→登出→重登，侧栏角色展示位随新角色翻转且登出无残留', async ({
    browser,
  }) => {
    test.setTimeout(60_000)
    // 前案把 LIVE 禁用了；先收敛回「启用 + admin」再走 UI。
    await healLive('admin')

    // 浏览器上下文独立（不揣 admin 会话 cookie）。
    const ctx = await browser.newContext()
    const page = await ctx.newPage()
    const collector = new ErrorCollector(page)

    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(LIVE.username, LIVE.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
    // 侧栏角色展示位（gate-agent-directory-writes 2.2）：与 /api/auth/me 一致。
    // 页脚有两个 .settings-btn（退出 + 设置）——用 aria-label 精确点名退出钮。
    const logoutBtn = page.locator('sebas-app .sidebar-footer button[aria-label="Sign out"]')
    await expect(logoutBtn).toHaveText(/退出 \(gate-live · admin\)/)

    // root 降级为 member（API 侧，浏览器不动）。
    expect(
      (await admin.post(`/api/users/${liveId}/role`, { data: { role: 'member' } })).status(),
    ).toBe(200)

    // 登出 → 登录页（无残留：用户名与角色不留在界面上）。
    await logoutBtn.click()
    await expect(page.locator('sebas-login')).toBeVisible({ timeout: 15_000 })
    await expect(page.locator('sebas-app .sidebar-footer button[aria-label="Sign out"]')).toHaveCount(
      0,
    )

    // 重登：前端重取 me，展示位随新角色翻转（display 与执法同源）。
    await login.login(LIVE.username, LIVE.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
    await expect(page.locator('sebas-app .sidebar-footer button[aria-label="Sign out"]')).toHaveText(
      /退出 \(gate-live · member\)/,
    )

    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
    await ctx.close()

    // 收敛回 admin（留给重试/后续旅程一个已知态）。
    await healLive('admin')
  })
})
