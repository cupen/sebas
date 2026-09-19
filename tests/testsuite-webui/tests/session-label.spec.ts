/**
 * Journey — 会话行重命名（fix-webui-approval-restore-and-session-identity
 * 5.1，tasks.md 7.1）。
 *
 * 功能：会话管理 / 子功能：行重命名
 *
 * Spec anchors: project-session-actions（MODIFIED）「Session rows are named
 * by the first prompt」的「operator label takes precedence」「renaming from
 * the rail」（rail 行与对话框同源命名、清空回退首条 prompt 预览）。
 *
 * The contract: the rail row's name is label → first-prompt preview → short
 * id. The operator sets/changes/clears the label from the row's … menu; the
 * rename lands without a page reload, survives reloads, and the rail's
 * dialogs name the session by the same label. Clearing falls back to the
 * first-message preview.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  listSessions,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /**
   * Drive a session row's … menu to the rename dialog. The trigger is
   * hover-revealed (same discipline as openSessionMenu in the shared page
   * objects); the rename entry is `wa-dropdown-item[value="rename"]`.
   * (The wa-dialog host reads hidden while in the top layer — assert the
   * rendered contents, same as sessions.spec / pending-stack.spec.)
   */
  async function openRenameDialog(rail: ProjectRail, rowLabel: string) {
    const row = rail.host
      .locator('li.session-item:not(.archived)', { hasText: rowLabel })
      .first()
    await row.hover()
    await row.locator('wa-dropdown button[title="Session actions"]').click()
    const item = row.locator('wa-dropdown-item[value="rename"]')
    await expect(item).toBeVisible()
    await item.click()
    const dialog = rail.page.locator('sebas-project-rail wa-dialog[label="重命名会话"]')
    await expect(
      dialog.locator('h2, [role="heading"]', { hasText: '重命名会话' }).first(),
    ).toBeVisible()
    return dialog
  }

  test.describe('行重命名', () => {
    test('label takes precedence over the first-prompt preview, survives reloads, and clearing falls back', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const tag = `rename-me-${Date.now()}`
      const key = await createSession(page.request, { prompt: tag })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/')
      await rail.ensureProjectExpanded(projectName)
      const row = rail.host
        .locator('li.session-item:not(.archived)', { hasText: tag })
        .first()
      await expect(row).toBeVisible({ timeout: 10_000 })
      // Baseline: the row is named by the first message's preview.
      await expect(row.locator('.session-name')).toContainText(tag)

      // ── rename from the rail … menu ────────────────────────────────────
      const dialog = await openRenameDialog(rail, tag)
      const input = dialog.locator('wa-input[data-testid="rename-input"] input')
      await input.click()
      await input.pressSequentially('发布代号')
      await dialog.locator('wa-button').filter({ hasText: '保存' }).click()

      // The row updates in place — no reload — and shows the label, with the
      // full text on the row title. (The hasText-tag locator stops matching
      // once the label replaces the preview — re-locate by the label.)
      const labeledRow = rail.host
        .locator('li.session-item:not(.archived)', { hasText: '发布代号' })
        .first()
      await expect(labeledRow.locator('.session-name')).toHaveText('发布代号', {
        timeout: 10_000,
      })
      await expect(labeledRow).toHaveAttribute('title', '发布代号')
      // API truth: the label reached the session row.
      const labeled = (await listSessions(page.request)).find((r) => r.encoded_key === key)
      expect(labeled?.label).toBe('发布代号')

      // The rail's dialogs name the session by the same label: open the
      // archive confirm dialog and cancel it.
      const archiveDialog = await rail.openArchiveDialog('发布代号')
      await expect(archiveDialog).toContainText('发布代号')
      await archiveDialog.locator('wa-button').filter({ hasText: '取消' }).click()
      await expect(
        rail.host.locator('li.session-item.archived', { hasText: '发布代号' }),
      ).toHaveCount(0)

      // The label is stable across reloads (label → preview order, stored
      // server-side).
      await page.reload()
      await rail.ensureProjectExpanded(projectName)
      const rowAfterReload = rail.host
        .locator('li.session-item:not(.archived)', { hasText: '发布代号' })
        .first()
      await expect(rowAfterReload.locator('.session-name')).toHaveText('发布代号', {
        timeout: 10_000,
      })

      // ── clearing the label falls back to the first-prompt preview ──────
      const dialog2 = await openRenameDialog(rail, '发布代号')
      await dialog2.locator('wa-input[data-testid="rename-input"] input').fill('')
      await dialog2.locator('wa-button').filter({ hasText: '保存' }).click()
      // The label is gone: the row re-locates by the FIRST-PROMPT preview
      // again (the hasText-label locator from above intentionally no longer
      // matches).
      const fallbackRow = rail.host
        .locator('li.session-item:not(.archived)', { hasText: tag })
        .first()
      await expect(fallbackRow.locator('.session-name')).toContainText(tag, {
        timeout: 10_000,
      })
      const cleared = (await listSessions(page.request)).find((r) => r.encoded_key === key)
      expect(cleared?.label ?? null).toBeNull()

      expect(collector.clean()).toEqual([])
    })
  })
})
