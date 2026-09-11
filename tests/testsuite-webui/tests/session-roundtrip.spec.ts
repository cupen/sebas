/**
 * Journey 3.2 — session core round-trip (spec: 首回合往返 + 重载恢复).
 *
 * 功能：agent 对话覆盖 / 子功能：首回合往返与重载恢复
 *
 * Submit a prompt through the workbench composer → the SPA STAYS on the
 * workbench and the focused session's conversation renders in place — the
 * operator's submission as its own turn bubble, fake-claude's "hello world"
 * reply as one agent bubble → status converges to Done → after a full page
 * reload the conversation and status come back from the server-side
 * persisted state (nothing lost).
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, resetState, FocusedSession, Workbench } from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首回合往返与重载恢复', () => {
    test('composer submit → reply → done → reload restores', async ({ page }) => {
      const workbench = new Workbench(page)
      const detail = new FocusedSession(page)

      // The creation-mode composer submit (this journey's first step) needs a
      // focused-free workbench — close everything any earlier journey left.
      await resetState(page.request)

      await page.goto('/')
      await expect(workbench.composer).toBeVisible()

      await workbench.sendPrompt('hello')

      // Creation no longer navigates (workbench-conversation-view 3.x: the
      // workbench IS the conversation surface) — the SPA stays on `/` and
      // the new session's conversation appears in place, submissions included.
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.userTurn('hello')).toBeVisible()

      // fake-claude answers "hello " + "world": ONE agent turn bubble whose
      // text carries both chunks in arrival order.
      await expect(detail.agentTurn('world').first()).toBeVisible({
        timeout: 15_000,
      })
      const bubbleTexts = await detail.bubbles().allTextContents()
      const helloIdx = bubbleTexts.findIndex((t) => t.includes('hello'))
      const worldIdx = bubbleTexts.findIndex((t) => t.includes('world'))
      expect(helloIdx).toBeGreaterThanOrEqual(0)
      expect(worldIdx).toBeGreaterThanOrEqual(helloIdx)

      // Turn converges to Done (badge slug flips on the session head).
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Reload: focus pointer + conversation + status recover from persisted
      // state — the workbench still renders the same focused session.
      await page.reload()
      await expect(detail.sessionHead).toBeVisible()
      await expect(detail.userTurn('hello')).toBeVisible()
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible()
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')

      expect(collector.clean()).toEqual([])
    })
  })
})
