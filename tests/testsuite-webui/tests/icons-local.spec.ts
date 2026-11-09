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
 * the CDN host, and icons still render (an <svg> lands inside a span).
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

      // The icon font/CDN is never contacted, and the controls render with
      // local affordances (the React stack has no Web Awesome icon host).
      expect(cdnRequests).toEqual([])
      await expect(rail.addButton).toBeVisible()

      // The add-project dialog stays fully usable without any external icon
      // asset and still never reaches the CDN.
      await rail.openAddDialog()
      await expect(rail.addDialog().getByRole('textbox', { name: '项目路径' })).toBeVisible()

      expect(cdnRequests).toEqual([])
      expect(localIconRequests.every((url) => url.startsWith('http://127.0.0.1:'))).toBe(true)
      expect(collector.clean()).toEqual([])
    })
  })
})
