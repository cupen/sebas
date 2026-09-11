/**
 * Journey 4.x — agent dialog (phase-2 tasks 4.1 + 4.2).
 *
 * 功能：agent 对话覆盖 / 子功能：多轮连续、输入守卫
 *
 * 4.1: two consecutive rounds in the SAME session append assistant bubbles
 * in order (the detail API filters `kind == "prompt"` entries, so follow-up
 * user text is NOT rendered — the honest observable contract is bubble-count
 * growth + per-round Done convergence) and survive a full page reload.
 *
 * 4.2: composer input guards (empty / whitespace-only submits create no turn)
 * and a special-character long-text round-trip (Chinese + emoji + code fence +
 * HTML-ish markup, ~2KB) that must not break the send path, the turn, or the
 * console. The stub never echoes input, so the payload's survival is proven
 * by a completed round, not by finding the text on screen.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
  FocusedSession,
  waitStatus,
} from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Open an idle session's detail page (live WS) and return key + page object. */
  async function openIdle(page: import('@playwright/test').Page, prompt: string) {
    const detail = new FocusedSession(page)
    const key = await createSession(page.request, { prompt })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    return { key, detail }
  }

  /** Wait for one full stub round (hello + world bubbles, status Done). */
  async function waitRound(
    detail: FocusedSession,
    opts: { bubblesTimeout?: number; doneTimeout?: number } = {},
  ) {
    await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible({
      timeout: opts.bubblesTimeout ?? 20_000,
    })
    await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
    await expect(detail.statusBadge).toHaveAttribute('slug', 'done', {
      timeout: opts.doneTimeout ?? 15_000,
    })
  }

  test.describe('多轮连续', () => {
    test('4.1 two consecutive rounds append in order and survive reload', async ({ page }) => {
      test.setTimeout(60_000)
      const t = Date.now()
      const q1 = `first-q-${t}`
      const q2 = `second-q-${t}`
      const detail = new FocusedSession(page)

      // Round 1 via API (deterministic, no navigation race), then open live detail.
      const key = await createSession(page.request, { prompt: q1 })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await waitRound(detail)
      const round1Count = await detail.bubbles().count()

      // Round 2 only after round 1 converged — never send into a running turn.
      // (No waitRound here: hello/world are already visible from round 1, so
      // only bubble-count growth followed by Done proves round 2 ran.)
      await detail.sendFollowUp(q2)
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBeGreaterThan(round1Count)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Order (workbench-conversation-view 2.1/2.3): each round's hello and
      // world live in the SAME agent turn bubble; the operator's q2 turn
      // sits BETWEEN the two agent turns — user, agent, user, agent.
      const blocks = page.locator('sebas-dashboard sebas-transcript-view .turn-block')
      await expect(blocks).toHaveCount(4)
      const classes: string[] = []
      const texts: string[] = []
      for (let i = 0; i < 4; i++) {
        classes.push((await blocks.nth(i).getAttribute('class')) ?? '')
        texts.push((await blocks.nth(i).textContent()) ?? '')
      }
      expect(classes[0]).toContain('is-user')
      expect(classes[1]).toContain('is-assistant')
      expect(classes[2]).toContain('is-user')
      expect(classes[3]).toContain('is-assistant')
      expect(texts[0]).toContain(q1)
      expect(texts[1]).toContain('hello')
      expect(texts[1]).toContain('world')
      expect(texts[2]).toContain(q2)
      expect(texts[3]).toContain('hello')
      expect(texts[3]).toContain('world')

      // Reload: both rounds recover from persisted state. The card quote
      // shows the LATEST turn's prompt — since workbench-turn-queue 2.1/2.2
      // the web path seeds each turn's prompt at turn start (same as feishu).
      await page.reload()
      await expect(detail.host).toBeVisible()
      await expect(detail.userTurn(q2)).toBeVisible()
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 10_000, intervals: [250] })
        .toBeGreaterThan(round1Count)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('输入守卫', () => {
    test('4.2 empty and blank input creates no turn, session stays usable', async ({
      page,
    }) => {
      const t = Date.now()
      const { detail } = await openIdle(page, `guard-${t}`)
      const before = await detail.bubbles().count()

      // The guard swallows silently client-side (the send button is NOT disabled
      // for empty input) — assert the no-op, not a disabled state.
      await detail.sendFollowUp('')
      await detail.sendFollowUp('   \n\t  ')
      await detail.composerTextarea.fill('   ')
      await detail.sendButton.click()

      // No new turn block appeared, status never left Done, no error surfaced.
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 5_000, intervals: [250] })
        .toBe(before)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')
      await expect(detail.unavailableNote).toHaveCount(0)

      // The session is still usable afterwards (count growth + Done prove the
      // probe round ran — probe text itself is not rendered, see file header).
      const probe = `guard-probe-${t}`
      await detail.sendFollowUp(probe)
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBeGreaterThan(before)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })

    test('4.2 special-char long text round-trips without loss or console errors', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const t = Date.now()
      const { key, detail } = await openIdle(page, `special-${t}`)
      const before = await detail.bubbles().count()

      const codeLine = `print("你好 ${t} 🎉")  # <b>tag</b> & "quotes"`
      const payload = [
        `中文问候🎉🔧 ${t}`,
        '',
        '```python',
        ...Array.from({ length: 40 }, () => codeLine),
        '```',
        '',
        `尾巴 <b>加粗</b> & 转义 "引号" ✅`,
      ].join('\n')
      expect(payload.length).toBeGreaterThan(1500)

      // The special payload must survive the send path (JSON/WS) and complete
      // a round: bubble growth + Done. The payload text itself is not rendered
      // anywhere in the webui (prompt entries are filtered from the detail
      // body), so survival == a healthy round, not visible text.
      await detail.sendFollowUp(payload)
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBeGreaterThan(before)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // API snapshot agrees the transcript grew; assistant side still the fixture.
      const { detail: snap } = await getSession(page.request, key)
      expect(snap).not.toBeNull()
      expect(snap!.entries.length).toBeGreaterThan(0)

      // Reload: transcript + done persist; the card quote carries the latest
      // turn's prompt (the special payload itself — workbench-turn-queue 2.2).
      await page.reload()
      await expect(detail.host).toBeVisible()
      await expect(detail.userTurn(`中文问候🎉🔧 ${t}`)).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')

      expect(collector.clean()).toEqual([])
    })
  })
})
