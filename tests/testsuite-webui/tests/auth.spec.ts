/**
 * Journey 3.8 — auth-on form (spec: 免登录直达 [对照] / 登录闭环).
 *
 * 功能：鉴权与访问旅程 / 子功能：深链重定向、登录与登出
 *
 * Runs ONLY under playwright.auth.config.ts (TESTSUITE_AUTH=1 sandbox on port
 * 9898, unified test account admin/admin provisioned in the sandbox-local
 * auth.db via `sebas auth add`). Covers: the deep link under auth redirects to
 * the login page, wrong credentials are rejected in place, admin/admin
 * enters the workbench, and logout returns to the unauthenticated state.
 * (The auth-OFF free-access control is implicitly covered by every
 * main-config spec.)
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
        .locator('sebas-app .sidebar-footer .settings-btn', { hasText: '退出 (admin · root)' })
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
      await login.login('admin', 'wrong-password')
      await expect(login.error).toHaveText('用户名或密码错误')
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // Correct credentials: into the workbench.
      await login.login('admin', 'admin')
      await expect(page.locator('sebas-app nav .brand .name')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-dashboard')).toBeVisible()

      // Logout: back to the unauthenticated gate.
      await page
        .locator('sebas-app .sidebar-footer .settings-btn', { hasText: '退出 (admin · root)' })
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

  test.describe('登录前 /ws 静默（fix-webui-qa-round3 D11）', () => {
    // 独立计数器：ErrorCollector 有意过滤 127.0.0.1 的 WebSocket 连接失败
    // （主套件 auth-off 无此噪音），而本旅程的合同恰是「未认证 /ws 升级失败
    // 至多一条」——这里裸数 console error，不做该过滤。
    test('pre-login /ws upgrade noise stays at most one per load; login restores the connection', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const wsFailures: string[] = []
      page.on('console', (msg) => {
        if (
          msg.type() === 'error' &&
          /WebSocket connection to 'ws:\/\/127\.0\.0\.1:\d+\/ws'/.test(msg.text())
        ) {
          wsFailures.push(msg.text())
        }
      })
      const login = new Login(page)

      await page.goto('/')
      await expect(login.host).toBeVisible()
      // 撑过旧短退避梯的活跃窗口：修复前梯子按 ~0.5/1/2/4/8s 重试，9s 内
      // 攒 5+ 条 401；修复后模块装载的急连至多 1 条，之后鉴权闸静默。
      await page.waitForTimeout(9_000)
      expect(
        wsFailures.length,
        `pre-login /ws failures: ${wsFailures.length}`,
      ).toBeLessThanOrEqual(1)

      // 再加载一次：同样至多一条（每页装载一次静默尝试）。
      await page.reload()
      await expect(login.host).toBeVisible()
      await page.waitForTimeout(4_000)
      expect(wsFailures.length).toBeLessThanOrEqual(2)

      // 登录成功 = 撤闸：工作台出现（/ws 连上），且登录后不再新增失败——
      // 恢复正常连接姿态，无重试风暴。
      await login.login('admin', 'admin')
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      const atLogin = wsFailures.length
      await page.waitForTimeout(4_000)
      expect(
        wsFailures.length,
        `post-login /ws failures appeared: ${wsFailures.slice(atLogin).join(' | ')}`,
      ).toBe(atLogin)
    })
  })
})
