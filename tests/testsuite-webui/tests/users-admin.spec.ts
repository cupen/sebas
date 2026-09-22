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
 * 交互约定与既有套件一致（helpers/pages/components.ts）：
 *  - wa-input 内层 input 逐字键入（pressSequentially）——`.fill` 会绕过
 *    wa-input 的 input 事件，@input 驱动的状态不更新（addProjectByPath 同款）；
 *  - wa-select 用 evaluate 设 value + 派发 change（pickDialogAgent 同款）。
 */
import { expect, test } from '@playwright/test'
import { authLogin, ErrorCollector, Login, SettingsModal } from './helpers/index'

const USERNAME = 'tester'
const PASSWORD = 'tester-pass-123'
const WEAK_PASSWORD = 'short'

/** admin cases 的公共前置：fresh context 无 cookie，先过登录门再进面板。 */
async function loginAsAdmin(page: import('@playwright/test').Page): Promise<SettingsModal> {
  await page.goto('/')
  const login = new Login(page)
  await login.visible()
  await login.login('admin', 'admin')
  await expect(page.locator('sebas-login')).toBeHidden()
  const settings = new SettingsModal(page)
  await settings.openViaSidebar()
  await settings.openSection('Users')
  return settings
}

test.describe.serial('Users 管理闭环', () => {
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
    const stale = users.find((u) => u.username.toLowerCase() === USERNAME)
    if (stale) expect((await request.delete(`/api/users/${stale.id}`)).status()).toBe(200)
  })

  test('root 经对话框创建 member 用户，列表即时可见', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    await page
      .locator('sebas-settings-modal wa-button')
      .filter({ hasText: 'New user' })
      .click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="New user"]')
    // wa-dialog 宿主在 top layer 读作 hidden——可见性断言落在渲染出的内部
    // 元素上（settings.spec 同款纪律）。
    await expect(dialog.locator('wa-button').filter({ hasText: 'Create' })).toBeVisible()

    const nameInput = dialog.locator('wa-input[label="Username"] input')
    await nameInput.click()
    await nameInput.pressSequentially(USERNAME)
    const passInput = dialog.locator('wa-input[label="Password (min 8 characters)"] input')
    await passInput.click()
    await passInput.pressSequentially(PASSWORD)
    // 角色留默认 member（对话框打开即 member，无需再选）。
    await dialog.locator('wa-button').filter({ hasText: 'Create' }).click()

    await expect(dialog).toBeHidden()
    await expect(
      page.locator('sebas-settings-modal [data-testid="user-action"]'),
    ).toHaveText(`用户 ${USERNAME} 已创建（角色 member）`)
    const row = page.locator(`sebas-settings-modal [data-testid="user-row"][data-username="${USERNAME}"]`)
    await expect(row).toBeVisible()
    await expect(row.locator('.provider-badge')).toHaveText('member')
    await expect(row.locator('.provider-key')).toHaveText('enabled')
  })

  test('重名创建被 409 拒绝并就地展示（大小写不敏感）', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    await page
      .locator('sebas-settings-modal wa-button')
      .filter({ hasText: 'New user' })
      .click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="New user"]')
    await expect(dialog.locator('wa-button').filter({ hasText: 'Create' })).toBeVisible()

    const nameInput = dialog.locator('wa-input[label="Username"] input')
    await nameInput.click()
    // 大写变体：唯一性按 NOCASE 判定（user_store 的 COLLATE NOCASE 契约）。
    await nameInput.pressSequentially(USERNAME.toUpperCase())
    const passInput = dialog.locator('wa-input[label="Password (min 8 characters)"] input')
    await passInput.click()
    await passInput.pressSequentially(PASSWORD)
    await dialog.locator('wa-button').filter({ hasText: 'Create' }).click()

    await expect(dialog.locator('[data-testid="user-create-error"]')).toHaveText(
      '用户名已存在（大小写不敏感）',
    )
    // 对话框保持打开、失败不发通知条。
    await expect(dialog.locator('[data-testid="user-create-error"]')).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()
    await expect(dialog).toBeHidden()
  })

  test('弱密码被客户端拦截，不发起请求', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    await page
      .locator('sebas-settings-modal wa-button')
      .filter({ hasText: 'New user' })
      .click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="New user"]')
    await expect(dialog.locator('wa-button').filter({ hasText: 'Create' })).toBeVisible()

    const nameInput = dialog.locator('wa-input[label="Username"] input')
    await nameInput.click()
    await nameInput.pressSequentially('shortlived')
    const passInput = dialog.locator('wa-input[label="Password (min 8 characters)"] input')
    await passInput.click()
    await passInput.pressSequentially(WEAK_PASSWORD)
    await dialog.locator('wa-button').filter({ hasText: 'Create' }).click()

    await expect(dialog.locator('[data-testid="user-create-error"]')).toHaveText(
      '密码至少需要 8 个字符',
    )
    await expect(dialog.locator('[data-testid="user-create-error"]')).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="user-row"][data-username="shortlived"]'),
    ).toHaveCount(0)
  })

  test('角色 member→viewer 即时生效且重开面板后仍在', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    const row = page.locator(`sebas-settings-modal [data-testid="user-row"][data-username="${USERNAME}"]`)
    await row.locator('wa-select.user-role').evaluate((el) => {
      ;(el as unknown as { value: string }).value = 'viewer'
      el.dispatchEvent(new Event('change', { bubbles: true }))
    })

    await expect(
      page.locator('sebas-settings-modal [data-testid="user-action"]'),
    ).toHaveText(`用户 ${USERNAME} 的角色已改为 viewer`)
    await expect(row.locator('.provider-badge')).toHaveText('viewer')

    // 关掉重开：角色来自 auth.db 读回，不是前端残留状态。
    await settings.close()
    await settings.openViaSidebar()
    await settings.openSection('Users')
    await expect(
      page
        .locator(`sebas-settings-modal [data-testid="user-row"][data-username="${USERNAME}"]`)
        .locator('.provider-badge'),
    ).toHaveText('viewer')
  })

  test('member 登录后 Users 管理入口被裁剪，API 越权 403', async ({ browser }) => {
    // 浏览器上下文必须独立——默认 context 里还揣着 admin 的会话 cookie。
    const ctx = await browser.newContext()
    const page = await ctx.newPage()
    const login = new Login(page)
    await page.goto('/')
    await login.visible()
    await login.login(USERNAME, PASSWORD)
    await expect(page.locator('sebas-login')).toBeHidden()

    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    // 角色驱动入口裁剪（5.4）：非 root 不渲染 Users 分区。
    await expect(settings.panel.locator('.nav-item', { hasText: 'Users' })).toHaveCount(0)
    // API 侧同一把尺：同一成员会话直呼 /api/users 被点名拒绝。
    expect((await ctx.request.get('/api/users')).status()).toBe(403)
    await ctx.close()
  })

  test('停用用户后行内状态翻转，可再启用', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    const row = page.locator(`sebas-settings-modal [data-testid="user-row"][data-username="${USERNAME}"]`)
    await row.locator('button[title="Disable user"]').click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="user-action"]'),
    ).toHaveText(`用户 ${USERNAME} 已禁用`)
    await expect(row.locator('.provider-key')).toHaveText('disabled')

    await row.locator('button[title="Enable user"]').click()
    await expect(row.locator('.provider-key')).toHaveText('enabled')
  })

  test('最后一个启用的 root 受保护：禁用与删除都被拒', async ({ page }) => {
    const settings = await loginAsAdmin(page)

    const adminRow = page.locator('sebas-settings-modal [data-testid="user-row"][data-username="admin"]')
    await adminRow.locator('button[title="Disable user"]').click()
    await expect(page.locator('sebas-settings-modal [data-testid="user-action"]')).toHaveText(
      '不能删除、禁用或降级最后一个启用的 root',
    )
    await expect(adminRow.locator('.provider-key')).toHaveText('enabled')

    await adminRow.locator('button[title="Delete user"]').click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="Delete user"]')
    await expect(dialog.locator('.dialog-text')).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: 'Delete' }).click()
    // 删除先撞上自删守卫（当前登录的正是 admin）；LastRoot 守卫由上面的
    // 禁用尝试断言。
    await expect(dialog.locator('[data-testid="user-delete-error"]')).toHaveText(
      '不能删除当前登录的用户自己',
    )
    await expect(dialog.locator('[data-testid="user-delete-error"]')).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()
    await expect(adminRow).toBeVisible()
  })

  test('删除用户后从名册消失且无法再登录', async ({ page, playwright }) => {
    const settings = await loginAsAdmin(page)

    const row = page.locator(`sebas-settings-modal [data-testid="user-row"][data-username="${USERNAME}"]`)
    await row.locator('button[title="Delete user"]').click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="Delete user"]')
    await expect(dialog.locator('.dialog-text')).toBeVisible()
    await expect(dialog.locator('.dialog-text')).toContainText(USERNAME)
    await dialog.locator('wa-button').filter({ hasText: 'Delete' }).click()

    await expect(
      page.locator('sebas-settings-modal [data-testid="user-action"]'),
    ).toHaveText(`用户 ${USERNAME} 已删除`)
    await expect(row).toHaveCount(0)

    // 删除即时生效：旧凭据登录被就地拒绝（会话已随删除失效）。用无 cookie
    // 的独立 API context——request fixture 里揣着 admin 会话，不能代表路人。
    const dead = await playwright.request.newContext()
    expect(await authLogin(dead, USERNAME, PASSWORD)).toBe(401)
    await dead.dispose()
  })
})
