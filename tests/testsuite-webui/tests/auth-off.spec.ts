/**
 * Journey — free access（spec: 免登录直达 [对照]，add-webui-auth-switch）.
 *
 * 功能：鉴权与访问旅程 / 子功能：免登录直达
 *
 * Main-suite form (auth = false sandbox, port 9899). Every other
 * main-config spec implicitly relies on the auth-OFF posture; this file
 * pins it explicitly: `/api/auth/me` reports `enabled:false`, the homepage
 * lands straight on the workbench, and NO gate element (login / setup)
 * ever appears — including past the shell's 5s core-reachability poll that
 * used to flip gated postures to the login gate (webui auth e2e 回归类).
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector } from './helpers/index'

test.describe('鉴权与访问旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('免登录直达', () => {
    test('the workbench is the homepage and no gate ever appears', async ({ page, request }) => {
      // Posture probe: the sandbox must be auth-OFF — otherwise the
      // main-config specs are asserting against the wrong deployment.
      const me = await request.get('/api/auth/me')
      expect(me.status()).toBe(200)
      const info = await me.json()
      expect(info.enabled).toBe(false)

      await page.goto('/')
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-login')).toHaveCount(0)
      await expect(page.locator('sebas-setup')).toHaveCount(0)

      // Outlive two reachability-poll intervals (5s each): auth-off answers
      // every poll with 200, so the workbench must stay put — no gate may
      // spring up.
      await page.waitForTimeout(11_000)
      await expect(page.locator('sebas-dashboard')).toBeVisible()
      await expect(page.locator('sebas-login')).toHaveCount(0)
      await expect(page.locator('sebas-setup')).toHaveCount(0)

      // Reload: still the workbench, no gate re-prompt.
      await page.reload()
      await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-login')).toHaveCount(0)

      // No gated request ever 401s in this posture, so the full console +
      // pageerror clean-slate assertion holds.
      expect(collector.clean()).toEqual([])
    })
  })
})
