/**
 * React 入口的首启建户旅程（redesign-webui-with-react-antd 5.1/5.2）。
 *
 * 只在 `playwright.react-setup.config.ts`（auth 开、零用户、端口 9891）下运行：
 * 首启门禁、客户端校验、创建 root 后进入工作台并跨刷新保持、以及 root 存在
 * 后再次 setup 被 409 拒绝（登录门禁而非建户页）。
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, ReactLogin, ReactSetup, ReactShell } from './helpers/index'

const ROOT_NAME = 'react-review-root'
const ROOT_PASSWORD = 'react-review-pass'

test.describe('React 首启 root 引导旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首启门禁与校验', () => {
    test('shows the setup card, not the login gate, and validates in place', async ({ page }) => {
      const setup = new ReactSetup(page)
      const login = new ReactLogin(page)

      await page.goto('/')
      await expect(setup.heading).toBeVisible({ timeout: 15_000 })
      await expect(login.heading).toHaveCount(0)
      await expect(page.getByTestId('react-shell')).toHaveCount(0)

      // Client-side validation: no request is made, the reason is inline.
      await setup.username.fill('root-candidate')
      await setup.password.fill('short')
      await setup.confirm.fill('short')
      await setup.submit.click()
      await expect(setup.error).toHaveText('密码至少需要 8 个字符')
      await expect(page.getByTestId('react-shell')).toHaveCount(0)

      await setup.password.fill('long-enough-pass')
      await setup.confirm.fill('different-pass')
      await setup.submit.click()
      await expect(setup.error).toHaveText('两次输入的密码不一致')
      await expect(page.getByTestId('react-shell')).toHaveCount(0)

      expect(collector.pageErrors).toEqual([])
    })
  })

  test.describe('建户与重载', () => {
    test('creating root enters the workbench and the session survives reload', async ({ page }) => {
      const setup = new ReactSetup(page)
      const shell = new ReactShell(page)
      const login = new ReactLogin(page)

      await page.goto('/')
      await expect(setup.heading).toBeVisible({ timeout: 15_000 })
      await setup.createRoot(ROOT_NAME, ROOT_PASSWORD)

      await expect(shell.app).toBeVisible({ timeout: 15_000 })
      await expect(page).toHaveURL(/\/$/)
      await expect(shell.rail).toBeVisible()
      await expect(page.getByRole('button', { name: '用户菜单' })).toContainText('root')

      // Cookie keeps the workbench after reload; the setup gate never returns.
      await page.reload()
      await expect(shell.app).toBeVisible({ timeout: 15_000 })
      await expect(setup.heading).toHaveCount(0)

      // Logout: now the instance has users, so the gate is the login form.
      await shell.logout()
      await expect(login.heading).toBeVisible({ timeout: 15_000 })
      await expect(setup.heading).toHaveCount(0)

      expect(collector.pageErrors).toEqual([])
    })

    test('a second setup POST is refused after root exists (409)', async ({ page, request }) => {
      // Sequential after the create test: the sandbox now has its root user.
      const login = new ReactLogin(page)
      await page.goto('/')
      await expect(login.heading).toBeVisible({ timeout: 15_000 })

      const resp = await request.post('/api/auth/setup', {
        data: { username: 'second-root', password: 'second-root-pass' },
      })
      expect(resp.status()).toBe(409)
      expect(collector.pageErrors).toEqual([])
    })
  })
})
