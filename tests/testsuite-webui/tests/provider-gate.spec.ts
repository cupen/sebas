/**
 * Journey — fix-webui-qa-round10 3.x：provider/别名变更面的角色执法与写入口
 * 裁剪（provider-management「Provider and alias mutations are role-gated」）。
 *
 * Runs ONLY under playwright.auth.config.ts（TESTSUITE_AUTH=1，端口 9898，
 * admin/admin 预置在沙箱本地 auth.db）。
 *
 * spec 逐场景：
 *  - member/viewer 对 provider 与 model-aliases 的增/改/删一律 403，且**无
 *    store 变化**（被拒的创建不留行、探针资源原样）；读面保持登录门
 *    （GET /api/providers、GET /api/model-aliases 200）；
 *  - member 的设置页 Models/别名分区不呈现任何写控件（新建×2/★/✎/🗑/清默认/
 *    别名新建/行内编辑删除），列表照常可浏览；浏览器 cookie 直调写 API 仍
 *    403（呈现层隐藏不构成防线，agents-gate 同款双半边）；
 *  - admin 全管：建→改→删闭环成功（探针资源的生命周期管理就是正向半边）。
 *
 * 服务端执法单测（api_endpoints_test.rs provider_role_gate）钉的是路由层
 * FakeBackend 半边；本旅程补真实 core + 真实登录会话 + 真渲染的另一半。
 * 账号自愈：member/viewer 经 admin 的 users API 现场建（幂等，409 视为已
 * 就绪）——与 agents-gate.spec 同款纪律。
 */
import { expect, test, type APIRequestContext } from '@playwright/test'
import {
  authLogin,
  ErrorCollector,
  Login,
  SettingsModal,
} from './helpers/index'

const MEMBER = { username: 'pgate-member', password: 'pgate-member-123' }
const VIEWER = { username: 'pgate-viewer', password: 'pgate-viewer-123' }
const PROVIDER = 'pgate-provider'
const ALIAS = 'pgate-alias'

/** admin 经 users API 建低权账号（幂等：重名 409 = 已就绪）。 */
async function ensureUser(
  request: APIRequestContext,
  username: string,
  password: string,
  role: 'viewer' | 'member',
): Promise<void> {
  const resp = await request.post('/api/users', { data: { username, password, role } })
  expect([200, 201, 409], `ensureUser ${username}: HTTP ${resp.status()}`).toContain(resp.status())
}

test.describe('provider 变更面角色执法（fix-webui-qa-round10 3.x）', () => {
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
    await ensureUser(request, MEMBER.username, MEMBER.password, 'member')
    await ensureUser(request, VIEWER.username, VIEWER.password, 'viewer')
    // 幂等清场：探针 provider/别名删掉重建，收敛到已知态（admin 全管的正向半边）。
    await request.delete(`/api/model-aliases/${ALIAS}`)
    await request.delete(`/api/providers/${PROVIDER}`)
    const created = await request.post('/api/providers', {
      data: { name: PROVIDER, api_key: 'sk-pgate-dummy' },
    })
    expect([200, 201], `admin creates probe provider: ${created.status()}`).toContain(
      created.status(),
    )
    const alias = await request.post('/api/model-aliases', {
      data: { alias: ALIAS, provider: PROVIDER },
    })
    expect([200, 201], `admin creates probe alias: ${alias.status()}`).toContain(alias.status())
  })

  test.afterAll(async ({ request }) => {
    // 清场：探针资源摘除（失败残留也收敛）。
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    await request.delete(`/api/model-aliases/${ALIAS}`)
    await request.delete(`/api/providers/${PROVIDER}`)
  })

  test('member/viewer 变更一律 403 且无 store 变化；读面保持登录门', async ({ request }) => {
    for (const user of [MEMBER, VIEWER]) {
      expect(await authLogin(request, user.username, user.password)).toBe(200)
      // 读面：认证即可（spec「Read access … remains available to signed-in
      // roles」）。别名清单随 providers 读面一并下发（routes.rs：别名分区
      // 不另造读 API，写面才有 /api/model-aliases）。
      expect((await request.get('/api/providers')).status()).toBe(200)
      // provider 三件套：403（服务端路由层执法）。
      expect(
        (
          await request.post('/api/providers', {
            data: { name: `made-by-${user.username}`, api_key: 'sk-x' },
          })
        ).status(),
      ).toBe(403)
      expect(
        (await request.put(`/api/providers/${PROVIDER}`, { data: { api_key: 'sk-hijack' } })).status(),
      ).toBe(403)
      expect((await request.delete(`/api/providers/${PROVIDER}`)).status()).toBe(403)
      // 别名三件套：同档 403。
      expect(
        (
          await request.post('/api/model-aliases', {
            data: { alias: `made-by-${user.username}`, provider: PROVIDER },
          })
        ).status(),
      ).toBe(403)
      expect(
        (
          await request.put(`/api/model-aliases/${ALIAS}`, {
            data: { alias: ALIAS, provider: PROVIDER },
          })
        ).status(),
      ).toBe(403)
      expect((await request.delete(`/api/model-aliases/${ALIAS}`)).status()).toBe(403)
    }

    // 无 store 变化：被拒的创建不留行，探针 provider/别名原样在册（providers
    // 是条目数组、别名是 model_aliases 名表；无独立别名读路由）。
    const providers = (await (await request.get('/api/providers')).json()) as {
      providers: Array<{ name: string }>
      model_aliases?: Record<string, unknown>
    }
    const names = providers.providers.map((p) => p.name)
    expect(names).toContain(PROVIDER)
    expect(names.some((n) => n.startsWith('made-by-'))).toBe(false)
    expect(Object.keys(providers.model_aliases ?? {})).toContain(ALIAS)
  })

  test('member 登录：Models/别名分区无写控件、列表可浏览；浏览器直调仍 403', async ({
    page,
  }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login(MEMBER.username, MEMBER.password)
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('模型')
    const modal = page.locator('sebas-settings-modal')
    // 读面保留：provider 列表照常呈现（探针行在册；providers 读面异步加载）。
    await expect(modal.locator('.provider-row', { hasText: PROVIDER })).toBeVisible({
      timeout: 10_000,
    })
    // 写控件全部不呈现（呈现层与执法同档）。
    await expect(modal.locator('wa-button').filter({ hasText: '新建（预设）' })).toHaveCount(0)
    await expect(modal.locator('wa-button').filter({ hasText: '新建（自定义）' })).toHaveCount(0)
    await expect(modal.locator('button[title="设为新建会话的默认"]')).toHaveCount(0)
    await expect(modal.locator('button[title="清除新建会话的默认值"]')).toHaveCount(0)
    await expect(modal.locator('button[title="编辑"]')).toHaveCount(0)
    await expect(modal.locator('button[title="删除"]')).toHaveCount(0)

    // 别名分区：写入口不呈现、列表保留（探针别名可浏览）。
    await settings.openSection('别名')
    const aliases = modal.locator('sebas-model-aliases')
    await expect(aliases.locator('[data-testid="alias-create"]')).toHaveCount(0)
    await expect(aliases.locator('[data-testid="alias-list"]')).toContainText(ALIAS, {
      timeout: 10_000,
    })
    await expect(aliases.locator('button[title="编辑"]')).toHaveCount(0)
    await expect(aliases.locator('button[title="删除"]')).toHaveCount(0)

    await settings.closeButton.click()
    await expect(settings.panel).toBeHidden()

    // 直调写 API：member 会话（cookie 随行）仍 403——呈现层不是防线。
    // 走 context 级 request，注定的 403 不进页面 console（净空断言不受污染）。
    const probe = await page.request.post('/api/providers', {
      data: { name: 'made-in-browser', api_key: 'sk-x' },
    })
    expect(probe.status()).toBe(403)
  })

  test('admin 对照面：写控件在（新建/★/✎/🗑），改名→删除→重建闭环成功', async ({ page }) => {
    await page.goto('/')
    const login = new Login(page)
    await login.visible()
    await login.login('admin', 'admin')
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('模型')
    const modal = page.locator('sebas-settings-modal')
    await expect(modal.locator('wa-button').filter({ hasText: '新建（预设）' }).first()).toBeVisible()
    await expect(modal.locator('wa-button').filter({ hasText: '新建（自定义）' }).first()).toBeVisible()
    const row = modal.locator('.provider-row', { hasText: PROVIDER })
    await expect(row.locator('button[title="设为新建会话的默认"]')).toBeVisible({
      timeout: 10_000,
    })
    await expect(row.locator('button[title="编辑"]')).toBeVisible()
    await expect(row.locator('button[title="删除"]')).toBeVisible()
    await settings.closeButton.click()
    await expect(settings.panel).toBeHidden()

    // API 全管闭环（正向半边；beforeAll 已覆盖建，这里覆盖改→删→再建）。
    const request = page.request
    expect(
      (await request.put(`/api/providers/${PROVIDER}`, { data: { api_key: 'sk-rotated' } })).status(),
    ).toBe(200)
    expect((await request.delete(`/api/providers/${PROVIDER}`)).status()).toBe(200)
    expect(
      (
        await request.post('/api/providers', {
          data: { name: PROVIDER, api_key: 'sk-pgate-dummy' },
        })
      ).status(),
    ).toBe(201)
    // 别名不随 provider 删除级联（状态库按名独立成行）——显式删掉再重建，
    // 闭环覆盖别名写面的 admin 半边。
    expect((await request.delete(`/api/model-aliases/${ALIAS}`)).status()).toBe(200)
    expect(
      (
        await request.post('/api/model-aliases', {
          data: { alias: ALIAS, provider: PROVIDER },
        })
      ).status(),
    ).toBe(201)
  })
})
