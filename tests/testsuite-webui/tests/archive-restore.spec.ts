/**
 * Journey — 归档恢复重建会话（fix-webui-qa-defects 2.4，tasks.md）。
 *
 * 功能：会话管理覆盖 / 子功能：归档恢复重建
 *
 * Spec anchors: project-session-actions「restore archived session」+
 * 「restore preserves the transcript」（fix-webui-qa-defects delta）。
 *
 * The data-loss defect this pins: restore used to only delete the archive
 * entry while the session stayed gone (rail / list / archive all empty —
 * data reachable nowhere). The contract now: restore REBUILDS the session
 * row (Dormant under the original key + project) and replays the archived
 * transcript, and the archive entry is consumed only after the rebuild
 * succeeded. Observably: after confirming the restore dialog the rail row
 * is back under its original project, the workbench renders the full
 * conversation, the History group no longer lists the entry, the session is
 * writable again (follow-up completes), and the API truth agrees
 * (list + detail carry all N entries; /api/archive drops the tag).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  listSessions,
  ProjectRail,
  resetState,
  FocusedSession,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /**
   * Expand a project row ONLY when collapsed — the row itself is the
   * expand/collapse toggle, so a blind click on an already-expanded row
   * would hide the sessions this journey needs to see.
   */
  async function ensureProjectExpanded(rail: ProjectRail, name: string): Promise<void> {
    const row = rail.projectRow(name)
    if ((await row.getAttribute('aria-expanded')) === 'false') {
      await row.click()
    }
  }

  test.describe('归档恢复重建', () => {
    test('archive → restore: rail row back, transcript intact, History cleared, writable again', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)

      const tag = `rebuild ${Date.now()}`
      const { id: projectId, name: projectName } = await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: tag, projectId })
      await waitStatus(page.request, key, ['done'])
      const entriesBefore = (await getSession(page.request, key)).detail!.entries.length
      expect(entriesBefore).toBeGreaterThan(0)

      // ── archive via the rail row's … menu ─────────────────────────────
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await ensureProjectExpanded(rail, projectName)
      await rail.archiveSession(tag)
      await expect(rail.sessionItem(tag)).toHaveCount(0, { timeout: 10_000 })

      // History group holds the archived entry.
      await rail.expandHistory()
      const archivedRow = rail.host
        .locator('li.session-item.archived', { hasText: tag })
        .first()
      await expect(archivedRow).toBeVisible({ timeout: 10_000 })

      // ── restore: History row → read-only view → confirm dialog ────────
      await archivedRow.click()
      const archivedView = page.locator('sebas-dashboard [data-testid="archived-view"]')
      await expect(archivedView).toBeVisible({ timeout: 10_000 })
      await archivedView.locator('[data-testid="archived-restore"]').click()
      // The confirm dialog names the NEW semantics upfront (task 2.3): the
      // rebuild note promises the session row will be rebuilt WITH its
      // transcript, not merely that the History entry disappears. (The
      // wa-dialog host reads hidden in the top layer — assert the rendered
      // contents, same as sessions.spec / pending-stack.spec.)
      await expect(
        page.locator('sebas-dashboard [data-testid="restore-rebuild-note"]'),
      ).toContainText('将重建会话并保留对话记录', { timeout: 10_000 })
      await page.locator('sebas-dashboard [data-testid="restore-confirm"]').click()

      // The read-only view exits (archive-view-close)…
      await expect(archivedView).toHaveCount(0, { timeout: 10_000 })
      // …and the workbench renders the REBUILT session's conversation — the
      // restore handler switched focus onto the restored key (switch now
      // succeeds: the Dormant mapping exists again, vs the old 404 silence).
      await expect(detail.userTurn(tag)).toBeVisible({ timeout: 20_000 })
      expect(page.url()).not.toContain('/sessions/')

      // Rail 可见：the active (non-archived) row is back under its original
      // project; the History group no longer lists the entry. round4 3.1
      // (design M1) migrates the naming sources across the archive round-trip,
      // so the restored row's rail label is the archived prompt preview
      // (= tag here) — the restored mapping is NOT label-less: the pre-round4
      // session_id_short fallback assertion is obsolete by design.
      await ensureProjectExpanded(rail, projectName)
      await expect(
        rail.host.locator('li.session-item:not(.archived)', { hasText: tag }).first(),
      ).toBeVisible({ timeout: 15_000 })
      await expect(
        rail.host.locator('li.session-item.archived', { hasText: tag }),
      ).toHaveCount(0, { timeout: 10_000 })

      // API truth — /api/archive drops the entry (History 清空)…
      const archiveList = await page.request.get('/api/archive')
      expect(archiveList.ok()).toBe(true)
      expect(await archiveList.text()).not.toContain(tag)
      // …the session is listed again under its ORIGINAL project…
      const rows = await listSessions(page.request)
      const row = rows.find((r) => r.encoded_key === key)
      expect(row, 'the rebuilt session must be listed after restore').toBeTruthy()
      expect(row!.project_id ?? null).toBe(projectId)
      // …and the detail exposes the SAME N transcript entries (转写完整).
      const after = await getSession(page.request, key)
      expect(after.status).toBe(200)
      expect(after.detail!.entries.length).toBe(entriesBefore)

      // Becomes writable again: a follow-up completes a full round-trip
      // (Dormant → first message routes as Resume of the original session —
      // the archived transcript stays visible across the routing-id change).
      await detail.sendFollowUp(`after-restore ${Date.now()}`)
      await detail.expectStatus('done')
      const afterFollowUp = await getSession(page.request, key)
      expect(afterFollowUp.detail!.entries.length).toBeGreaterThan(entriesBefore)
      // The pre-archive history is still visible next to the new turn.
      await expect(detail.userTurn(tag)).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })
})
