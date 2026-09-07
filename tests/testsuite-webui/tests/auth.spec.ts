/**
 * Journey 3.8 — auth-on form (spec: 免登录直达 [对照] / 登录闭环).
 *
 * 功能：鉴权与访问旅程 / 子功能：深链重定向、登录与登出
 *
 * Runs ONLY under playwright.auth.config.ts (TESTSUITE_AUTH=1 sandbox on port
 * 9898, admin/admin provisioned in the sandbox-local auth file). Covers:
 * the deep link under auth redirects to the login page, wrong credentials
 * are rejected in place, admin/admin enters the workbench, and logout
 * returns to the unauthenticated state. (The auth-OFF free-access control
 * is implicitly covered by every main-config spec.)
 */
import { expect, test } from '@playwright/test'
import { authLogin, createSession, ErrorCollector, Login } from './helpers/index'

test.describe('鉴权闭环', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('深链重定向', () => {
    test('deep link under auth redirects to the login page', async ({ page, request }) => {
      const login = new Login(page)

      // Seed a session via the API. `request` is a standalone API context —
      // its auth cookie never reaches the browser, which stays logged out.
      expect(await authLogin(request, 'admin', 'admin')).toBe(200)
      const key = await createSession(request, { prompt: 'deep link authed' })

      // Logged-out browser hitting the deep path: login gate, not the session.
      await page.goto(`/sessions/${key}`)
      await expect(login.host).toBeVisible()
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // The logged-out page surfaces WS auth-failed as a console error by
      // design; assert clean of real uncaught exceptions only.
      expect(collector.pageErrors).toEqual([])
    })
  })

  test.describe('登录与登出', () => {
    test('session cookie survives reload until logout, then the gate returns', async ({ page }) => {
      const login = new Login(page)

      await page.goto('/')
      await expect(login.host).toBeVisible()
      await login.login('admin', 'admin')
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

      // Reload: the session cookie keeps the workbench up — no gate re-prompt.
      await page.reload()
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      await expect(login.host).toHaveCount(0)

      // Logout: back to the gate, and it sticks across another reload.
      await page
        .locator('sebas-app .sidebar-footer .settings-btn', { hasText: '退出 (admin)' })
        .click()
      await expect(login.host).toBeVisible({ timeout: 15_000 })
      await page.reload()
      await expect(login.host).toBeVisible()
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      expect(collector.pageErrors).toEqual([])
    })

    test('wrong credentials rejected, admin/admin enters, logout returns', async ({ page }) => {
      const login = new Login(page)

      await page.goto('/')
      await expect(login.host).toBeVisible()

      // Wrong password: rejected in place, no workbench.
      await login.login('admin', 'wrong')
      await expect(login.error).toHaveText('用户名或密码错误')
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // Correct credentials: into the workbench.
      await login.login('admin', 'admin')
      await expect(page.locator('sebas-app nav .brand .name')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-dashboard')).toBeVisible()

      // Logout: back to the unauthenticated gate.
      await page
        .locator('sebas-app .sidebar-footer .settings-btn', { hasText: '退出 (admin)' })
        .click()
      await expect(login.host).toBeVisible({ timeout: 15_000 })

      // And the gate actually gates: workbench content is gone.
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // Auth journey intentionally exercises REJECTION paths (wrong password →
      // 401; unauthenticated WS → auth-failed), which the browser logs as
      // console errors by design. Assert clean of real uncaught exceptions
      // (pageerror) instead of console messages: the rejection signals are the
      // contract under test, not defects.
      expect(collector.pageErrors).toEqual([])
    })
  })
})
