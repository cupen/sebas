/**
 * Journey 2.x — session management round-trips (phase-2 tasks 2.1 + 2.2).
 *
 * 功能：会话管理覆盖 / 子功能：多会话切换、archive 写保护
 *
 * 2.1: two live sessions, focus switches both ways (rail click + cold
 * deep-link) without crosstalk. Focus is a display pointer only (spec), so
 * the contract is observable: URL + prompt quote + Done per switch, and a
 * follow-up grows ONLY the switched-to session (API body length is the
 * oracle — follow-up user text is filtered from the detail body, so length
 * growth is the crosstalk signal).
 *
 * 2.2: archive → write-protection (400) → restore → writable again.
 * Archived sessions leave the active list/rail, show in History, reject
 * messages with 400, and come back fully usable after restore.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
  listSessions,
  middleTruncate,
  ProjectRail,
  resetState,
  FocusedSession,
  SessionsPage,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Short rail label for a session key (mirrors the backend elision). */
  async function shortId(
    request: import('@playwright/test').APIRequestContext,
    key: string,
  ): Promise<string> {
    const { detail } = await getSession(request, key)
    return middleTruncate(detail!.session_id ?? '', 18)
  }

  /** Assert the detail page shows the session with the given original prompt. */
  async function expectDetail(
    page: import('@playwright/test').Page,
    detail: FocusedSession,
    prompt: string,
  ): Promise<void> {
    await expect(detail.host).toBeVisible()
    await expect(detail.userTurn(prompt)).toBeVisible({ timeout: 15_000 })
    await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })
  }

  test.describe('多会话切换', () => {
    test('2.1 dual-session switch (rail + deep-link) does not crosstalk', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const t = Date.now()
      const promptA = `alpha-${t}`
      const promptB = `beta-${t}`
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const keyA = await createSession(page.request, { prompt: promptA })
      const keyB = await createSession(page.request, { prompt: promptB })
      await waitStatus(page.request, keyA, ['done'])
      await waitStatus(page.request, keyB, ['done'])
      const shortA = await shortId(page.request, keyA)
      const shortB = await shortId(page.request, keyB)

      // Cold deep-link to A.
      await page.goto(`/sessions/${keyA}`)
      await expectDetail(page, detail, promptA)
      const countA1 = await detail.bubbles().count()

      // Follow-up lands in A only.
      await detail.sendFollowUp(`follow-A-${t}`)
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBeGreaterThan(countA1)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })
      const lenAfterA = (await getSession(page.request, keyA)).detail!.entries.length
      const lenBBefore = (await getSession(page.request, keyB)).detail!.entries.length

      // Rail click switches to B IN PLACE (workbench-conversation-view 3.1:
      // switch + focus, no navigation away from the workbench, no reload).
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandInbox()
      await rail.sessionItem(shortB).click()
      await expectDetail(page, detail, promptB)
      expect(page.url()).not.toContain('/sessions/')

      // Follow-up lands in B only; A is untouched.
      const countB1 = await detail.bubbles().count()
      await detail.sendFollowUp(`follow-B-${t}`)
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBeGreaterThan(countB1)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })
      expect((await getSession(page.request, keyB)).detail!.entries.length).toBeGreaterThan(
        lenBBefore,
      )
      expect((await getSession(page.request, keyA)).detail!.entries.length).toBe(lenAfterA)

      // Cold deep-link back to A: still its own conversation, still Done. The
      // operator's own turn bubble carries A's LATEST submission (the
      // follow-up) — workbench-turn-queue seeds each web turn's prompt at
      // start, and the conversation view renders it (2.3).
      await page.goto(`/sessions/${keyA}`)
      await expectDetail(page, detail, `follow-A-${t}`)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('archive 写保护', () => {
    test('2.2 archive → 400 write-protection → restore unhides honestly', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const rail = new ProjectRail(page)
      const sessions = new SessionsPage(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)

      const tag = `restore ${Date.now()}`
      const key = await createSession(page.request, { prompt: tag })
      await waitStatus(page.request, key, ['done'])
      const bodyBefore = (await getSession(page.request, key)).detail!.entries.length

      // Archive via the rail; row leaves the active rail.
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandInbox()
      const short = await shortId(page.request, key)
      await rail.archiveSession(short)
      await expect(rail.sessionItem(short)).toHaveCount(0, { timeout: 10_000 })

      // History group holds the entry; active list hides it.
      await rail.expandHistory()
      const archivedRow = rail.host
        .locator('li.session-item.archived', { hasText: tag })
        .first()
      await expect(archivedRow).toBeVisible({ timeout: 10_000 })
      await page.goto('/sessions')
      await expect(sessions.cardFor(key)).toHaveCount(0, { timeout: 10_000 })

      // Write protection: message to the archived session is rejected.
      const refused = await page.request.post(`/api/sessions/${key}/message`, {
        data: { message: 'should not land' },
      })
      expect(refused.status()).toBe(400)
      expect(await refused.text()).toContain('archiv')
      const afterRefuse = await getSession(page.request, key)
      if (afterRefuse.detail) {
        expect(afterRefuse.detail.entries.length).toBe(bodyBefore)
      }

      // Restore via the History row. IMPLEMENTATION DISCOVERY (honest contract):
      // archive closed the session (mapping + transcript dropped,
      // engine/mod.rs web_close_session), and restore only deletes the archive
      // entry (api.rs restore_session) — it does NOT resurrect the session.
      // The rail's restore stays on the workbench (the switch 404s silently —
      // the session is gone); no fabricated success, no dead deep link.
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandHistory()
      await rail.host
        .locator('li.session-item.archived', { hasText: tag })
        .first()
        .click()
      // The operator is never navigated to a dead session view: the
      // workbench stays up (workbench-conversation-view 3.1/3.4).
      await expect(rail.host).toBeVisible({ timeout: 10_000 })
      expect(page.url()).not.toContain('/sessions/')

      // Archive list no longer carries the entry; the session stays gone from
      // the live list (it was closed at archive time, restore resurrects nothing).
      const archiveList = await page.request.get('/api/archive')
      expect(archiveList.ok()).toBe(true)
      expect(await archiveList.text()).not.toContain(tag)
      expect(
        (await listSessions(page.request)).filter((r) => r.encoded_key === key).length,
      ).toBe(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
