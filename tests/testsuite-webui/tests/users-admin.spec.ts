/**
 * Journey — Settings → Users 管理闭环（add-webui-multiuser-rbac 5.3/5.4）。
 *
 * Runs ONLY under playwright.auth.config.ts（auth-on，端口 9898，admin/admin
 * 预置在沙箱本地 auth.db）。本旅程是 2026-09-23 浏览器验收发现的最大覆盖
 * 缺口：整套 Users 管理面（建户/重名/弱密码/改角色/停用/删除/root 保护/
 * 角色裁剪）此前没有任何浏览器级用例；auth.db 现由 sebas-db 共享 runtime
 * 承载（extract-sebas-db：连接配方 + ActiveRecord CRUD），本旅程同时充当
 * user_store 改道后的浏览器级回归。
 *
 * 选择器已迁到 React 设置的公共语义：设置分区导航（设置分区 aside）、
 * 对话框标题/字段可访问名、行动作的可访问名与状态文案。AntD 的 Select 用
 * pickSelectOption 走可见选项行，不再 evaluate 组件私有 value。
 */
import { expect, test } from '@playwright/test'
import { authLogin, ErrorCollector, Login } from './helpers/index'
import { pickSelectOption, ReactSettings, ReactShell } from './helpers/pages/react'

const USERNAME = 'tester'
const PASSWORD = 'tester-pass-123'
const WEAK_PASSWORD = 'short'

type Page = import('@playwright/test').Page

/** admin cases 的公共前置：fresh context 无 cookie，先过登录门再进面板。 */
async function loginAsAdmin(page: Page): Promise<ReactSettings> {
  await page.goto('/')
  const login = new Login(page)
  await login.visible()
  await login.login('admin', 'admin')
  await expect(page.getByTestId('react-shell')).toBeVisible()
  const settings = new ReactSettings(page)
  await new ReactShell(page).openSettings()
  await settings.openSection('用户')
  return settings
}

function userRow(settings: ReactSettings, username: string) {
  return settings.region
    .locator('strong', { hasText: new RegExp(`^${username}$`) })
    .locator('xpath=../..')
}

test.describe('Users 管理闭环', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  // 重试/复用场景的自愈：admin 身份经 API 清掉同名残留，保证 serial 首案
  // 从「只有 admin」的干净名册出发（auth-on 沙箱每次冷启只预置 admin）。
  test.beforeAll(async ({ request }) => {
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    const resp = await request.get('/api/users')
    const { users } = (await resp.json()) as {
      users: Array<{ id: number; username: string }>
    }
    for (const name of [USERNAME, 'shortlived']) {
      const stale = users.find((u) => u.username.toLowerCase() === name)
      if (stale) expect((await request.delete(`/api/users/${stale.id}`)).status()).toBe(200)
    }
  })

  test('root 经对话框创建 member 用户，列表即时可见', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    await settings.region.getByRole('button', { name: '新建用户' }).click()
    const dialog = page.getByRole('dialog', { name: '新建用户' })
    await expect(dialog.getByRole('button', { name: /创\s*建/ })).toBeVisible()

    await dialog.getByLabel('新用户名').fill(USERNAME)
    await dialog.getByLabel('新用户密码').fill(PASSWORD)
    // 角色留默认 member（对话框打开即 member，无需再选）。
    await dialog.getByRole('button', { name: /创\s*建/ }).click()

    await expect(dialog).toBeHidden()
    await expect(settings.region.getByRole('alert')).toHaveText('用户已创建')
    const row = userRow(settings, USERNAME)
    await expect(row).toBeVisible()
    await expect(row.getByText('member', { exact: true }).first()).toBeVisible()
    await expect(row).toContainText('已启用')
  })

  test('重名创建被 409 拒绝并就地展示（大小写不敏感）', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    await settings.region.getByRole('button', { name: '新建用户' }).click()
    const dialog = page.getByRole('dialog', { name: '新建用户' })
    await expect(dialog.getByRole('button', { name: /创\s*建/ })).toBeVisible()

    // 大写变体：唯一性按 NOCASE 判定（user_store 的 COLLATE NOCASE 契约）。
    await dialog.getByLabel('新用户名').fill(USERNAME.toUpperCase())
    await dialog.getByLabel('新用户密码').fill(PASSWORD)
    await dialog.getByRole('button', { name: /创\s*建/ }).click()

    // React 把创建失败放在设置区的 action alert（对话框仍在）；断言错误
    // 语义（409/用户名已存在）而不是把错误安置位置写成实现细节。
    await expect(settings.region.getByRole('alert')).toContainText('用户名已存在')
    await expect(dialog).toBeVisible()
    await dialog.getByRole('button', { name: /取\s*消/ }).click()
    await expect(dialog).toBeHidden()
  })

  test('角色 member→viewer 即时生效且重开面板后仍在', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    const row = userRow(settings, USERNAME)
    await pickSelectOption(page, row.getByRole('combobox', { name: `${USERNAME} 的角色` }), 'viewer')

    await expect(settings.region.getByRole('alert')).toHaveText(`${USERNAME} 角色更新`)
    await expect(row.getByText('viewer', { exact: true }).first()).toBeVisible()

    // 关掉重开：角色来自 auth.db 读回，不是前端残留状态。
    await settings.close()
    await new ReactShell(page).openSettings()
    await settings.openSection('用户')
    await expect(userRow(settings, USERNAME).getByText('viewer', { exact: true }).first()).toBeVisible()
  })

  test('member 登录后 Users 管理入口被裁剪，API 越权 403', async ({ browser }) => {
    // 浏览器上下文必须独立——默认 context 里还揣着 admin 的会话 cookie。
    const ctx = await browser.newContext()
    const page = await ctx.newPage()
    const login = new Login(page)
    await page.goto('/')
    await login.visible()
    await login.login(USERNAME, PASSWORD)
    await expect(page.getByTestId('react-shell')).toBeVisible()

    const settings = new ReactSettings(page)
    await new ReactShell(page).openSettings()
    // 角色驱动入口裁剪（5.4）：非 root 不渲染 Users 分区。
    await expect(settings.section('用户')).toHaveCount(0)
    // API 侧同一把尺：同一成员会话直呼 /api/users 被点名拒绝。
    expect((await ctx.request.get('/api/users')).status()).toBe(403)
    await ctx.close()
  })

  test('停用用户后行内状态翻转，可再启用', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    const row = userRow(settings, USERNAME)
    await row.getByRole('button', { name: /停\s*用/ }).click()
    await expect(settings.region.getByRole('alert')).toHaveText('已停用用户')
    await expect(row).toContainText('已禁用')

    await row.getByRole('button', { name: /启\s*用/ }).click()
    await expect(row).toContainText('已启用')
  })

  test('删除用户后从名册消失且无法再登录', async ({ page, playwright }) => {
    const settings = await loginAsAdmin(page)

    const row = userRow(settings, USERNAME)
    await row.getByRole('button', { name: /删\s*除/ }).click()
    const dialog = page.getByRole('dialog', { name: '删除用户' })
    await expect(dialog).toContainText(USERNAME)
    await dialog.getByRole('button', { name: /删\s*除/ }).click()

    await expect(settings.region.getByRole('alert')).toHaveText('已删除用户')
    await expect(row).toHaveCount(0)

    // 删除即时生效：旧凭据登录被就地拒绝（会话已随删除失效）。用无 cookie
    // 的独立 API context——request fixture 里揣着 admin 会话，不能代表路人。
    const dead = await playwright.request.newContext()
    expect(await authLogin(dead, USERNAME, PASSWORD)).toBe(401)
    await dead.dispose()
  })
  test('弱密码被客户端拦截，不发起请求', async ({ page }) => {
    const settings = await loginAsAdmin(page)
    const creates: string[] = []
    page.on('request', (request) => {
      if (request.method() === 'POST' && request.url().endsWith('/api/users')) {
        creates.push(request.url())
      }
    })

    await settings.region.getByRole('button', { name: '新建用户' }).click()
    const dialog = page.getByRole('dialog', { name: '新建用户' })
    await expect(dialog.getByRole('button', { name: /创\s*建/ })).toBeVisible()

    await dialog.getByLabel('新用户名').fill('shortlived')
    await dialog.getByLabel('新用户密码').fill(WEAK_PASSWORD)
    await dialog.getByRole('button', { name: /创\s*建/ }).click()

    await expect(settings.region.getByRole('alert')).toContainText('密码至少需要 8 个字符')
    await expect(dialog).toBeVisible()
    // 客户端拦截＝请求根本没有发出（不是服务端 400 的另一种说法）。
    expect(creates).toEqual([])
    await dialog.getByRole('button', { name: /取\s*消/ }).click()
    await expect(userRow(settings, 'shortlived')).toHaveCount(0)
  })

  test('最后一个启用的 root 受保护：自己那一行的停用/删除被客户端自锁', async ({ page }) => {
    // 契约（fix-webui-qa-round8 验收期）：自己那一行的停用/删除禁用且
    // title 点名原因，角色下拉同锁；重置密码不受自锁约束。
    const settings = await loginAsAdmin(page)

    const adminRow = userRow(settings, 'admin')
    const selfGuardTitle = '不能对自己行执行该操作（自锁防护）'
    // 行内动作序（enabled 用户）：停用 / 重置密码 / 删除（+ 角色下拉）。
    const stop = adminRow.getByRole('button', { name: /停\s*用/ })
    const reset = adminRow.getByRole('button', { name: '重置密码' })
    const remove = adminRow.getByRole('button', { name: /删\s*除/ })
    await expect(stop).toBeDisabled()
    await expect(stop).toHaveAttribute('title', selfGuardTitle)
    await expect(reset).toBeEnabled()
    await expect(remove).toBeDisabled()
    await expect(remove).toHaveAttribute('title', selfGuardTitle)
    await expect(adminRow.getByRole('combobox', { name: 'admin 的角色' })).toBeDisabled()

    await expect(adminRow).toContainText('已启用')
  })

})
