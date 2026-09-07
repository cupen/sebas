/**
 * Journey 3.2 — session core round-trip (spec: 首回合往返 + 重载恢复).
 *
 * 功能：agent 对话覆盖 / 子功能：首回合往返与重载恢复
 *
 * Submit a prompt through the workbench composer → the SPA navigates to the
 * session detail → the user prompt quote and fake-claude's "hello world"
 * reply render in order → status converges to Done → after a full page
 * reload the transcript and status come back from the server-side
 * persisted state (nothing lost).
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, resetState, SessionDetailPage, Workbench } from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首回合往返与重载恢复', () => {
    test('composer submit → reply → done → reload restores', async ({ page }) => {
      const workbench = new Workbench(page)
      const detail = new SessionDetailPage(page)

      // The creation-mode composer submit (this journey's first step) needs a
      // focused-free workbench — close everything any earlier journey left.
      await resetState(page.request)

      await page.goto('/')
      await expect(workbench.composer).toBeVisible()

      await workbench.sendPrompt('hello')

      // composer-created navigates the SPA straight to the new session.
      await page.waitForURL(/\/sessions\//)
      await expect(detail.host).toBeVisible()
      await expect(detail.promptQuote).toContainText('hello')

      // fake-claude answers "hello " + "world" as two streamed chunks that
      // land as separate assistant bubbles, in order.
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible({
        timeout: 15_000,
      })
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      const bubbleTexts = await detail.bubbles().allTextContents()
      const helloIdx = bubbleTexts.findIndex((t) => t.includes('hello'))
      const worldIdx = bubbleTexts.findIndex((t) => t.includes('world'))
      expect(helloIdx).toBeGreaterThanOrEqual(0)
      expect(worldIdx).toBeGreaterThanOrEqual(helloIdx)

      // Turn converges to Done (badge slug flips on the status head).
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Reload: transcript + status recover from persisted state.
      await page.reload()
      await expect(detail.host).toBeVisible()
      await expect(detail.promptQuote).toContainText('hello')
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible()
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')

      expect(collector.clean()).toEqual([])
    })
  })
})
