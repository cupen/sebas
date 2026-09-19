/**
 * Journey — 图标本地化（fix-webui-approval-restore-and-session-identity 5.4，
 * tasks.md 7.1）。
 *
 * 功能：工作台首屏 / 子功能：图标本地化
 *
 * The defect this pins: the dashboard pulled Font Awesome icons from
 * ka-f.fontawesome.com — an offline/CDN-blocked browser rendered broken
 * glyphs (403s). The contract now: the used icon subset ships in dist
 * (`public/icons`, Web Awesome `setIconPath('/icons')`), zero requests touch
 * the CDN host, and icons still render (an <svg> lands inside a wa-icon).
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, ProjectRail } from './helpers/index'

const CDN_HOST = 'ka-f.fontawesome.com'

test.describe('工作台首屏', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('图标本地化', () => {
    test('blocking the icon CDN changes nothing: zero CDN requests, local /icons assets, icons still render', async ({
      page,
    }) => {
      const cdnRequests: string[] = []
      const localIconRequests: string[] = []
      // Simulate a hosts-file block: any CDN request fails hard (like a 403).
      await page.route(`**://${CDN_HOST}/**`, (route) => {
        cdnRequests.push(route.request().url())
        return route.abort()
      })
      page.on('request', (req) => {
        if (new URL(req.url()).pathname.startsWith('/icons/')) localIconRequests.push(req.url())
      })

      const rail = new ProjectRail(page)
      await page.goto('/')
      await expect(rail.host).toBeVisible({ timeout: 15_000 })

      // The icon font/CDN is never contacted…
      expect(cdnRequests).toEqual([])
      // …and the inline-SVG icons still render (the rail's add button).
      await expect(rail.addButton.locator('svg')).toHaveCount(1)

      // The wa-icon assets resolve from same-origin: opening the add-project
      // dialog renders <wa-icon name="folder">, whose SVG is fetched under
      // /icons/ (the localized subset), never from the CDN.
      await rail.openAddDialog()
      await expect(localIconRequests.length).toBeGreaterThan(0)
      for (const url of localIconRequests) {
        expect(url.startsWith(`http://127.0.0.1:`), `local icon fetch: ${url}`).toBe(true)
      }
      await rail.addDialog().locator('wa-icon[name="folder"]').first().waitFor({
        state: 'attached',
        timeout: 5_000,
      })

      expect(cdnRequests).toEqual([])
      expect(collector.clean()).toEqual([])
    })
  })
})
