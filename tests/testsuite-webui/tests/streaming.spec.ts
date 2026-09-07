/**
 * Journey 3.3 — streaming turn (spec: 流式分批渲染).
 *
 * The "stream" trigger makes fake-claude emit 5 text chunks, pause 800ms,
 * then finish. We spawn with the trigger and open the page immediately,
 * then poll (no fixed sleeps) the server's transcript for the deterministic
 * contract: the 5 chunks arrive as SEPARATE bubbles (batched streaming, not
 * one blob) and the status converges to Done.
 *
 * The transient `working` state is timing-dependent (a scheduled CI box can
 * observe the whole turn inside one poll gap), so we record it as an
 * annotation rather than a hard assert — the D4 discipline against
 * order/timing-dependent flakes.
 */
import { expect, test } from '@playwright/test'
import { createSession, ErrorCollector, getSession, SessionDetailPage } from './helpers/index'

test.describe('streaming turn', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('chunks arrive in batches and the turn converges to done', async ({ page }) => {
    const detail = new SessionDetailPage(page)

    // Spawn with the stream trigger and open the page at once.
    const key = await createSession(page.request, { prompt: 'stream' })
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()

    // Sample the server transcript at 100ms (no fixed sleep) until all 5
    // chunks are present. Zero fixed sleeps anywhere.
    let sawTransientRunning = false
    await expect
      .poll(async () => {
        const { detail: api } = await getSession(page.request, key)
        if (!api) return -1
        if (api.status_slug === 'working') sawTransientRunning = true
        return api.body.filter((b) => /^chunk\d/.test(b.content)).length
      }, { timeout: 20_000, intervals: [100] })
      .toBe(5)
    if (sawTransientRunning) {
      test.info().annotations.push({
        type: 'spuriousInfo',
        description: 'observed transient working state during the stream turn',
      })
    }

    // The five chunks render as separate bubbles (batched arrival, not one blob).
    for (let i = 0; i < 5; i++) {
      await expect(detail.turnWith(`chunk${i}`).first()).toBeVisible({ timeout: 15_000 })
    }

    // Final convergence: Done, honestly.
    await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

    expect(collector.clean()).toEqual([])
  })
})
