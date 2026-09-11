/**
 * Journey 3.7 — session management surface (spec: close 与 archive / 深链
 * 与退役路径 / 模型面诚实缺省).
 *
 * 功能：会话管理覆盖 / 子功能：close 与 archive、深链与退役路径、模型面诚实缺省
 *
 * close: the confirmation dialog is real — closing removes the row from
 * the active list. archive: the rail's Inbox group hides the session and
 * the History group shows the archived entry; the sessions list no longer
 * lists it. Deep links: a /sessions/:key URL loaded cold (SPA fallback)
 * renders the session. Retired path: /settings canonically redirects to /.
 * Model honest absence: a fake-claude session exposes NO model selector
 * anywhere, and an API-level switch attempt leaves the model absent.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
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

  test.describe('close 与 archive', () => {
    test('close removes the session from the active list', async ({ page }) => {
      const sessions = new SessionsPage(page)

      // Deterministic base so the sessions list shows a predictable set.
      await resetState(page.request)

      const key = await createSession(page.request, { prompt: 'close me' })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/sessions')
      await expect(sessions.pageTitle).toHaveText('Sessions')
      const card = sessions.cardFor(key)
      await expect(card).toBeVisible()

      await sessions.closeCard(key)

      // Row leaves the active list (refetch after close).
      await expect(sessions.cardFor(key)).toHaveCount(0, { timeout: 10_000 })

      expect(collector.clean()).toEqual([])
    })

    test('archive hides from list, shows in History', async ({ page }) => {
      const rail = new ProjectRail(page)
      const sessions = new SessionsPage(page)

      await resetState(page.request)

      // Unique prompt so the archived entry's label is unambiguous (the
      // archive stores session_key with a raw NUL byte and labels by prompt;
      // earlier "archive me" runs leave many same-label entries behind).
      const tag = `archive ${Date.now()}`
      const key = await createSession(page.request, { prompt: tag })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandInbox()
      // The rail labels the active row by `session_id_short` (no chat_id).
      const { detail } = await getSession(page.request, key)
      const shortId = middleTruncate(detail!.session_id ?? '', 18)
      await rail.archiveSession(shortId)

      // The row leaves the active rail...
      await expect(rail.sessionItem(shortId)).toHaveCount(0, { timeout: 10_000 })
      // ...and the History group holds the archived entry (label = prompt).
      await rail.expandHistory()
      await expect(
        rail.host.locator('li.session-item.archived', { hasText: tag }).first(),
      ).toBeVisible({ timeout: 10_000 })

      // Active list no longer shows it.
      await page.goto('/sessions')
      await expect(sessions.cardFor(key)).toHaveCount(0, { timeout: 10_000 })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('深链与退役路径', () => {
    test('deep link renders the session via SPA fallback; /settings redirects to /', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)

      const key = await createSession(page.request, { prompt: 'deep link' })
      await waitStatus(page.request, key, ['done'])

      // Cold navigation straight to the deep path: the WORKBENCH renders with
      // the session focused (workbench-conversation-view 3.4 — no separate
      // detail page) and its conversation (fake-claude answers "hello " +
      // "world" as ONE agent turn bubble).
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible({
        timeout: 15_000,
      })
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Retired /settings route canonically lands on the dashboard.
      await page.goto('/settings')
      await expect(page).toHaveURL(/\/$/)
      await expect(page.locator('sebas-dashboard')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('模型面诚实缺省', () => {
    test('model honest absence — no selector, switch attempt keeps model absent', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)

      const key = await createSession(page.request, { prompt: 'no models' })
      await waitStatus(page.request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      // D4: the head has no model picker; the follow-mode composer has no
      // model dropdown; and nothing errored while rendering.
      await expect(detail.modelPick).toHaveCount(0)
      await expect(page.locator('sebas-workbench-composer wa-select[aria-label="Model"]')).toHaveCount(
        0,
      )

      // The API accepts the request but the truth doesn't change: no model
      // materialises on a session whose agent exposes none.
      await expect
        .poll(
          async () => (await getSession(page.request, key)).detail?.current_model,
          { timeout: 5_000, intervals: [200] },
        )
        .toBeNull()
      const after = await getSession(page.request, key)
      expect(after.detail).not.toBeNull()
      expect(after.detail!.current_model).toBeNull()
      expect(after.detail!.available_models ?? []).toEqual([])

      expect(collector.clean()).toEqual([])
    })
  })
})
