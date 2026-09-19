/**
 * Journey — 越界/不存在路径的注册禁用原因（fix-webui-approval-restore-and-
 * session-identity 5.2，tasks.md 7.1）。
 *
 * 功能：项目管理覆盖 / 子功能：越界禁用原因
 *
 * Spec anchor: workspace-root（MODIFIED）「范围判定是规范化且 fail-closed
 * 的」的「手填越界路径给出禁用原因」。
 *
 * The defect this pins (root-cause fix, 5.2): the add-project precheck only
 * matched the register-side wording, while browse-dirs — the actual precheck
 * authority — answers out-of-root candidates with「路径超出根目录范围」and
 * missing ones with「路径不存在或无法访问」; the operator got a dead grey
 * button with NO reason. The contract now: typing an out-of-root (or
 * nonexistent) path shows the concrete reason under the input and keeps the
 * submit disabled; a valid path clears the hint and enables submission.
 */
import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  ErrorCollector,
  listProjects,
  ProjectRail,
  removeProject,
  resetState,
  sceneDir,
} from './helpers/index'

test.describe('项目管理覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('越界禁用原因', () => {
    test('out-of-root and missing paths state their reason with submit disabled; a valid path enables it', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const scene = sceneDir()

      await page.goto('/')
      await rail.openAddDialog()
      const input = rail.addDialog().locator('wa-input[label="Project path"] input')
      const hint = rail.addDialog().locator('[data-testid="add-project-scope-hint"]')
      const submit = rail
        .addDialog()
        .locator('wa-button')
        .filter({ hasText: 'Add project' })

      // An EXISTING directory outside the workspace root → the boundary
      // reason, submit stays disabled. (browse-dirs resolves the candidate
      // against the root; an absolute request replaces the base and fails
      // the component-wise prefix check.)
      const outside = process.platform === 'win32' ? 'C:\\Windows' : '/etc'
      await input.click()
      await input.pressSequentially(outside)
      await expect(hint).toBeVisible({ timeout: 10_000 })
      await expect(hint).toContainText('workspace root 之外')
      // wa-button does not reflect disabled to a host attribute — assert the
      // property (Playwright can't see WA's internal disabled semantics).
      await expect(submit).toHaveJSProperty('disabled', true)

      // A path that does not exist → its own reason, still disabled.
      await input.press('Control+a')
      await input.pressSequentially(`no-such-dir-${Date.now()}`)
      await expect(hint).toBeVisible({ timeout: 10_000 })
      await expect(hint).toContainText('路径不存在或无法访问')
      await expect(submit).toHaveJSProperty('disabled', true)

      // A real directory inside the root: the hint clears and submission
      // enables — the fence explains rejections, not legitimate entries.
      const legit = path.join(scene, `legit-${Date.now()}`)
      fs.mkdirSync(legit, { recursive: true })
      await input.press('Control+a')
      await input.pressSequentially(legit)
      await expect(hint).toHaveCount(0, { timeout: 10_000 })
      await expect(submit).toHaveJSProperty('disabled', false)
      await submit.click()
      const added = (await listProjects(page.request)).find((p) => p.path === legit)
      expect(added, 'the in-root project must register').toBeTruthy()
      await expect(
        rail.host.locator('.row', { hasText: added!.name }).first(),
      ).toBeVisible({ timeout: 10_000 })

      // Polite cleanup: deregister the scratch project.
      await removeProject(page.request, added!.id)

      expect(collector.clean()).toEqual([])
    })
  })
})
