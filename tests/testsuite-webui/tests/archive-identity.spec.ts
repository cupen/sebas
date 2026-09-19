/**
 * Journey — 归档恢复携带会话身份（fix-webui-approval-restore-and-session-
 * identity 3.1/3.2/3.3，tasks.md 7.1）。
 *
 * 功能：会话管理 / 子功能：归档恢复身份
 *
 * Spec anchors: project-session-actions（MODIFIED）「Session archive」的
 * 「恢复保留 agent 身份与模型面」「旧归档条目如实回退」。
 *
 * The defect this pins: the archive entry used to drop the session's
 * identity (agent binding, desired mode, model catalog) — restoring rebuilt
 * the session on defaults, silently migrating it to another agent surface.
 * The contract now: the entry carries the identity four-tuple and restoring
 * rebuilds the session with it intact (the header honestly shows the same
 * agent); legacy entries without identity fields restore on the existing
 * fallbacks and the header presents THAT fallback honestly ("default
 * agent"), without fabricating an identity.
 */
import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  archiveSession,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  listSessions,
  ProjectRail,
  resetState,
  sceneDir,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Drive History → read-only archived view → restore confirm dialog. */
  async function restoreFromHistory(page: import('@playwright/test').Page, tag: string) {
    const rail = new ProjectRail(page)
    await rail.expandHistory()
    const archivedRow = rail.host
      .locator('li.session-item.archived', { hasText: tag })
      .first()
    await expect(archivedRow).toBeVisible({ timeout: 10_000 })
    await archivedRow.click()
    const archivedView = page.locator('sebas-dashboard [data-testid="archived-view"]')
    await expect(archivedView).toBeVisible({ timeout: 10_000 })
    await archivedView.locator('[data-testid="archived-restore"]').click()
    await page.locator('sebas-dashboard [data-testid="restore-confirm"]').click()
    await expect(archivedView).toHaveCount(0, { timeout: 15_000 })
  }

  test.describe('归档恢复身份', () => {
    test('archived→restored keeps agent_kind, mode and the model catalog; legacy entries fall back honestly', async ({
      page,
    }) => {
      test.setTimeout(120_000)
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName, id: projectId } = await ensureSceneProject(page.request)

      // fakeacp advertises a model catalog (bad-model/ok-model, initial =
      // first) and 'ask' records the desired mode — a non-default identity
      // worth carrying across the archive boundary.
      const tag = `identity-${Date.now()}`
      const key = await createSession(page.request, {
        prompt: tag,
        agent: 'fakeacp',
        mode: 'ask',
      })
      await waitStatus(page.request, key, ['done'])
      const before = (await getSession(page.request, key)).detail!
      expect(before.agent_kind).toBe('fakeacp')
      expect(before.desired_mode).toBe('ask')
      expect(before.available_models ?? []).toEqual(['bad-model', 'ok-model'])
      expect(before.current_model).toBe('bad-model')

      // ── archive via the rail row's … menu ─────────────────────────────
      await page.goto('/')
      await rail.ensureProjectExpanded(projectName)
      await rail.archiveSession(tag)
      await expect(rail.sessionItem(tag)).toHaveCount(0, { timeout: 10_000 })
      // API truth: the ENTRY carries the identity four-tuple.
      const archiveList = await page.request.get('/api/archive')
      expect(archiveList.ok()).toBe(true)
      const entry = ((await archiveList.json()) as { archived_sessions: Record<string, unknown>[] })
        .archived_sessions.find((e) => e.label === tag)
      expect(entry).toBeTruthy()
      expect(entry!['agent_kind']).toBe('fakeacp')
      expect(entry!['desired_mode']).toBe('ask')
      expect(entry!['current_model']).toBe('bad-model')
      expect(entry!['available_models']).toEqual(['bad-model', 'ok-model'])

      // ── restore through the History journey ───────────────────────────
      await restoreFromHistory(page, tag)
      // The header shows the SAME agent honestly (no silent re-binding).
      await expect(
        page
          .locator('sebas-dashboard .session-head [data-testid="agent-lock"]')
          .first(),
      ).toContainText('fakeacp', { timeout: 15_000 })
      const restored = (await getSession(page.request, key)).detail!
      expect(restored.agent_kind).toBe('fakeacp')
      expect(restored.desired_mode).toBe('ask')
      expect(restored.available_models ?? []).toEqual(['bad-model', 'ok-model'])
      expect(restored.current_model).toBe('bad-model')
      // Rail row back under the original project; History consumed.
      const row = (await listSessions(page.request)).find((r) => r.encoded_key === key)
      expect(row).toBeTruthy()
      expect(row!.project_id ?? null).toBe(projectId)
      const afterRestoreArchive = await page.request.get('/api/archive')
      expect((await afterRestoreArchive.text())).not.toContain(tag)

      // ── legacy entry: strip the identity fields from archive.json ─────
      await archiveSession(page.request, key)
      const archivePath = path.join(sceneDir(), 'archive.json')
      const file = JSON.parse(fs.readFileSync(archivePath, 'utf8')) as {
        entries: Record<string, unknown>[]
      }
      const legacyTag = file.entries[0]?.label ?? ''
      expect(legacyTag).toBeTruthy()
      for (const e of file.entries) {
        delete e.agent_kind
        delete e.desired_mode
        delete e.current_model
        delete e.available_models
      }
      fs.writeFileSync(archivePath, JSON.stringify(file))
      await page.reload()
      await restoreFromHistory(page, legacyTag)

      // The fallback is presented honestly: "default agent", NOT a
      // fabricated identity. API agrees (agent_kind null).
      await expect(
        page
          .locator('sebas-dashboard .session-head [data-testid="agent-lock"]')
          .first(),
      ).toContainText('default agent', { timeout: 15_000 })
      const legacy = (await getSession(page.request, key)).detail!
      expect(legacy.agent_kind ?? null).toBeNull()
      // The rebuilt session is listed again (row back under the rail; the
      // row LABEL legitimately regresses to the fallback chain — a restored
      // mapping carries no operator label).
      await expect(
        rail.host.locator('li.session-item:not(.archived)').first(),
      ).toBeVisible({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
