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
   *
   * 展开带目标行可见性复核：goto('/') 后工作台会异步自动聚焦最近会话，
   * rail-select 重绘可能把刚展开的项目行重新收起——只展开一次会让后续的
   * 行内菜单 hover 在收起态等满超时（本旅程的 flaky 根因）。展开→复核→
   * 再展开，直到目标行真正可见。
   */
  async function ensureProjectExpanded(
    rail: ProjectRail,
    name: string,
    sessionText: string,
  ): Promise<void> {
    for (let guard = 0; guard < 6; guard++) {
      const row = rail.projectRow(name)
      if ((await row.getAttribute('aria-expanded')) === 'false') {
        await row.click()
      }
      const target = rail.host
        .locator('button:not(:has-text("归档于"))', { hasText: sessionText })
        .first()
      try {
        await target.waitFor({ state: 'visible', timeout: 2_500 })
        return
      } catch {
        // 重绘又收起了 → 再展开一轮
      }
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
      await ensureProjectExpanded(rail, projectName, tag)
      await rail.archiveSession(tag)
      await expect(rail.sessionItem(tag)).toHaveCount(0, { timeout: 10_000 })

      // History group holds the archived entry.
      await rail.expandHistory()
      const archivedRow = rail.host
        .locator('button:has-text("归档于")', { hasText: tag })
        .first()
      await expect(archivedRow).toBeVisible({ timeout: 10_000 })

      // ── restore: History row → archive snapshot dialog → restore ─────
      await archivedRow.click()
      const dialog = page.getByRole('dialog', { name: /归档会话：/ })
      await expect(dialog).toBeVisible({ timeout: 10_000 })
      // The snapshot carries the archived transcript, so the operator can
      // inspect it before restoring.
      await expect(dialog).toContainText(tag)
      await dialog.getByRole('button', { name: '恢复会话' }).click()
      await expect(dialog).toBeHidden({ timeout: 15_000 })

      // The session row is rebuilt under its original project; focusing it
      // renders the restored conversation (React keeps restore and navigation
      // as two explicit operator steps).
      await expect(rail.projectRow(projectName)).toBeVisible({ timeout: 15_000 })
      await rail.ensureProjectExpanded(projectName)
      // AntD 的 Modal 关闭后 wrap/mask 仍挂载一小段时间并继续拦截指针事件
      // （实测三次 click 重试都被 `.ant-modal-wrap` 吃掉，行未触发聚焦）。
      // 等可见遮罩真正退场再点行，而不是盲点后由 20s 超时掩盖。
      await expect(page.locator('.ant-modal-wrap:visible')).toHaveCount(0, { timeout: 15_000 })
      await rail.sessionItem(tag).click()
      await expect(detail.userTurn(tag)).toBeVisible({ timeout: 20_000 })
      expect(page.url()).toContain('/sessions/')

      // Rail 可见：the active (non-archived) row is back under its original
      // project; the History group no longer lists the entry. round4 3.1
      // (design M1) migrates the naming sources across the archive round-trip,
      // so the restored row's rail label is the archived prompt preview
      // (= tag here) — the restored mapping is NOT label-less: the pre-round4
      // session_id_short fallback assertion is obsolete by design.
      await ensureProjectExpanded(rail, projectName, tag)
      await expect(rail.sessionItem(tag)).toBeVisible({ timeout: 15_000 })
      await expect(
        rail.host.locator('button:has-text("归档于")', { hasText: tag }),
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
