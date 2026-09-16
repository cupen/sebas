/**
 * Journey — first-run root setup（spec: 首启 root 引导 / 鉴权与访问旅程）.
 *
 * 功能：鉴权与访问旅程 / 子功能：首启门禁、校验与建户
 *
 * Runs ONLY under playwright.auth-setup.config.ts (TESTSUITE_AUTH_SETUP=1
 * sandbox on port 9896: auth switch ON, ZERO users — no admin/admin
 * provisioning; zero-user is the journey's premise).
 *
 * Cases are ORDER-SENSITIVE within the file (workers: 1): the gate/flip
 * guard and the in-place validation must run while the user store is still
 * empty; root creation then flips the posture for the 409 probe that closes
 * the file.
 *
 * Regression pin (webui auth e2e): the shell's core-reachability poll
 * (`/api/summary`, 5s interval) 401s in this zero-user posture, and its
 * global unauthorized handler used to flip the setup card to the login
 * gate ~5s in — a dead end, since with zero users NO credentials can ever
 * pass the gate. The suite asserts the setup card STAYS PUT past two poll
 * intervals.
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, Login, Setup } from './helpers/index'

test.describe('鉴权与访问旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首启门禁', () => {
    test('homepage shows the setup card and never flips to the login gate', async ({
      page,
      request,
    }) => {
      // Posture probe: the sandbox must be in the zero-user setup shape —
      // otherwise this file is asserting against the wrong deployment.
      const me = await request.get('/api/auth/me')
      expect(me.status()).toBe(200)
      const info = await me.json()
      expect(info.enabled).toBe(true)
      expect(info.needs_setup).toBe(true)

      const setup = new Setup(page)
      await page.goto('/')
      await setup.visible()
      await expect(page.locator('sebas-login')).toHaveCount(0)
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // The flip regression: outlive two reachability-poll intervals (5s
      // each) — the card must still be the setup card, not the login gate.
      // (The reachability poll is suspended outside the workbench, so no
      // /api/summary 401 fires here; the shared WS still retries eagerly by
      // design (auth.spec discipline) and logs auth-failed console errors —
      // assert clean of real uncaught exceptions only.)
      await page.waitForTimeout(11_000)
      await setup.visible()
      await expect(page.locator('sebas-login')).toHaveCount(0)
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)
      expect(collector.pageErrors).toEqual([])
    })
  })

  test.describe('校验与建户', () => {
    test('in-place validation rejects short password and mismatch without submitting', async ({
      page,
    }) => {
      const setup = new Setup(page)
      await page.goto('/')
      await setup.visible()

      // Short password: the 8-char floor, same standard as the server 400.
      await setup.setup('admin', 'short')
      await expect(setup.error).toHaveText('密码至少需要 8 个字符')
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)

      // Mismatched confirm: same inline rejection, still no workbench.
      await setup.username.fill('admin')
      await setup.password.fill('password8')
      await setup.confirm.fill('password9')
      await setup.submit.click()
      await expect(setup.error).toHaveText('两次输入的密码不一致')
      await expect(page.locator('sebas-dashboard')).toHaveCount(0)
      expect(collector.pageErrors).toEqual([])
    })

    test('creating root enters the workbench and the session survives reload', async ({
      page,
      request,
    }) => {
      const setup = new Setup(page)
      const me = await request.get('/api/auth/me')
      const needsSetup = (await me.json()).needs_setup === true

      if (needsSetup) {
        // Zero-user posture: provision root through the setup card.
        await page.goto('/')
        await setup.visible()
        await setup.setup('admin', 'password8')
      } else {
        // A prior attempt already created root but crashed afterwards (its
        // retry premise is poisoned) — the same account must simply log in.
        await page.goto('/')
        const login = new Login(page)
        await login.visible()
        await login.login('admin', 'password8')
      }
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-setup')).toHaveCount(0)

      // The setup response sets the session cookie: the sidebar shows the
      // signed-in account and a reload keeps the workbench up.
      await expect(
        page.locator('sebas-app .sidebar-footer .settings-btn', { hasText: '退出 (admin)' }),
      ).toBeVisible()
      await page.reload()
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-login')).toHaveCount(0)
      await expect(page.locator('sebas-setup')).toHaveCount(0)
      expect(collector.pageErrors).toEqual([])
    })

    test('a second setup POST is refused after root exists (409)', async ({ request }) => {
      // Root was created by the previous case; a fresh (cookie-less) request
      // context replaying the setup call must be refused — first-visitor
      // root preemption is the exact scenario this endpoint exists to
      // prevent.
      const res = await request.post('/api/auth/setup', {
        data: { username: 'root', password: 'password8' },
      })
      expect(res.status()).toBe(409)
    })
  })
})
