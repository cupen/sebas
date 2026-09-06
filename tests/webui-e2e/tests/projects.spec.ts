/**
 * Journey 3.6 — project add & remove (spec: 项目增删).
 *
 * Add a directory INSIDE the sandbox through the add-project dialog (the
 * folder-picker dialog), then remove it. The dialog's folder-picker tree
 * carries a recorded Web Awesome bug (lazy-load append crashes a null
 * nextSibling — see helpers/errors.ts KNOWN_PAGE_ERRORS), so the
 * deterministic path is the dialog's own manual path field. We assert the
 * functional contract: the project appears in the rail, the workbench
 * header reflects it, and removal (API-only surface) drops it on reload.
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, listProjects, ProjectRail, removeProject, sceneDir } from './helpers/index'

test.describe('project add & remove', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('add via project dialog appears in rail; removed project disappears', async ({ page }) => {
    const rail = new ProjectRail(page)
    const scene = sceneDir()
    const projectName = scene.split('/').filter(Boolean).pop()!

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.openAddDialog()

    // The dialog's manual path field (this is the folder-picker dialog;
    // typing the directory avoids the recorded wa-tree lazy-load bug).
    await rail.addProjectByPath(scene)

    // Functural contract: the project lands server-side and the rail shows
    // it. (The dialog host read is popover-hidden, so assert on the rail and
    // the workbench header rather than the closing dialog.)
    await expect
      .poll(async () => (await listProjects(page.request)).some((p) => p.path === scene), {
        timeout: 10_000,
        intervals: [200],
      })
      .toBe(true)
    await expect(rail.projectRow(projectName)).toBeVisible({ timeout: 10_000 })
    await expect(page.locator('sebas-dashboard .project-header .path')).toHaveText(projectName, {
      timeout: 10_000,
    })

    // Remove via the API surface; the rail reflects it on reload.
    await removeProject(page.request, scene)
    await expect
      .poll(async () => (await listProjects(page.request)).some((p) => p.path === scene), {
        timeout: 10_000,
        intervals: [200],
      })
      .toBe(false)
    await page.reload()
    await expect(rail.projectRow(projectName)).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })
})
