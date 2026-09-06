/**
 * Journey 3.5 — error semantics (spec: 错误呈现).
 *
 * "refuse": the stub ends the turn with a refusal — a NON-terminal error.
 * The session must survive it honestly: no fake Done, the row stays, and
 * the next message still completes. "crash": the stub emits one last text
 * frame and the child process dies — the mapping is torn down and the UI
 * must present that death (session gone / not-found), never a fabricated
 * success. Both trigger through the follow-up composer on an already-open
 * detail page (live WS), the realistic operator flow.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
  listSessions,
  SessionDetailPage,
  waitStatus,
} from './helpers/index'

test.describe('error journeys', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Open an idle session's detail page (live WS) and return the key. */
  async function openIdle(page: import('@playwright/test').Page) {
    const detail = new SessionDetailPage(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    return { key, detail }
  }

  test('refuse — non-terminal: session survives, next message works', async ({ page }) => {
    const { key, detail } = await openIdle(page)

    // On an already-DONE session a refusal is a NON-terminal error: it is
    // surfaced on the feishu card path (not the webui turn_log), so the
    // honest webui-observable contract is that the session is NOT torn down
    // and the next message still completes a full round-trip. We must not
    // fabricate a failure text that the webui transcript never carries.
    await detail.sendFollowUp('refuse')

    // The session mapping survives the refusal (not removed / not failed).
    const afterRefuse = await getSession(page.request, key)
    expect(afterRefuse.detail).not.toBeNull()
    expect(afterRefuse.detail!.status_slug).not.toBe('failed')

    // The next message still goes through a full round-trip.
    await detail.sendFollowUp('hello')
    await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible({
      timeout: 20_000,
    })
    await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
    await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

    expect(collector.clean()).toEqual([])
  })

  test('crash — honest death: not-found presentation, no fake success', async ({ page }) => {
    const { key, detail } = await openIdle(page)

    await detail.sendFollowUp('boom crash')

    // The child dies mid-turn; the session mapping is torn down. The detail
    // view refetches over the live WS connection and surfaces the 404
    // honestly instead of showing a stale or successful session.
    await expect(detail.errorCallout).toContainText('Session not found', { timeout: 20_000 })
    await expect(detail.backToWorkbench).toBeVisible()

    // Row gone from the live list; the API agrees.
    await expect
      .poll(
        async () => (await listSessions(page.request)).some((r) => r.encoded_key === key),
        { timeout: 10_000, intervals: [200] },
      )
      .toBe(false)

    expect(collector.clean()).toEqual([])
  })
})
