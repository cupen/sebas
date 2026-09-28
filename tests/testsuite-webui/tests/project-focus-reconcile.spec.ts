/**
 * Journey — displayed-project 焦点调和（fix-webui-qa-findings D4，spec
 * project-session-actions「项目注册与移除后的焦点一致性」）。
 *
 * 功能：项目管理覆盖 / 子功能：移除后的主面板调和、聚焦会话随展示项目调和
 *
 * The defect this pins (QA round2 report-A A2): with a project displayed in
 * the workbench, removing it from the rail dropped the rail row but the main
 * panel header KEPT naming the removed project (`work local
 * 未聚焦任何会话`); with zero projects left the rail said 尚未注册项目 while
 * the header still named the removed project. The contract now:
 *
 * - removing the displayed (or last) project drops the workbench header to
 *   未选择项目 WITHOUT any reload — the main panel never lags the rail
 *   (displayedProjectResolved gate);
 * - switching the displayed project away from the focused session drops the
 *   stale conversation to the session-less empty state; explicit operator
 *   action (selecting the session again) restores it — never inertia;
 * - the removal receipt toast lands (M6「项目移除成功有回执」).
 *
 * 场景基座：项目注册/移除的**编排走 rail UI**（行菜单 → 移除 → 确认弹窗）
 * ——那正是 A2 现场的操作者路径，且主面板调和考核的是「不重载的活页面」；
 * 会话与项目夹具走 API（同 projects.spec 纪律：arrange 走 HTTP，断言操作者
 * 所见留在浏览器）。
 */
import fs from 'node:fs'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  addProject,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  listProjects,
  ProjectRail,
  resetState,
  sceneDir,
  waitStatus,
} from './helpers/index'

test.describe('displayed-project 焦点调和', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('removing the displayed project drops the workbench header without a reload (A2 residue)', async ({
    page,
  }) => {
    test.setTimeout(60_000)
    const rail = new ProjectRail(page)
    const t = Date.now()
    const alpha = `d4-alpha-${t}`
    const beta = `d4-beta-${t}`
    const alphaDir = path.join(sceneDir(), alpha)
    const betaDir = path.join(sceneDir(), beta)
    fs.mkdirSync(alphaDir, { recursive: true })
    fs.mkdirSync(betaDir, { recursive: true })

    await resetState(page.request)
    await addProject(page.request, alphaDir)
    await addProject(page.request, betaDir)

    await page.goto('/')
    await expect(rail.host).toBeVisible()

    // Display alpha: clicking the project row selects it into the workbench.
    await rail.projectRow(alpha).click()
    const headerPath = page.locator('sebas-dashboard .project-header .path')
    await expect(headerPath).toHaveText(alpha, { timeout: 10_000 })

    // Remove the DISPLAYED project through the rail (the operator path from
    // the A2 evidence): row menu → 移除项目 → confirm.
    const row = rail.projectRow(alpha)
    await row.hover()
    await row.locator('button[title="Project actions"]').click()
    await row.locator('wa-dropdown-item[value="remove"]').click()
    // wa-dialog 是 top-layer popover：host 本体读作 hidden，内容可见——
    // 可见性断言打在标题上（同 rail.openAddDialog 纪律）。
    const dialog = page.locator('wa-dialog', { hasText: '移除项目' })
    await expect(
      dialog.locator('h2, [role="heading"]', { hasText: 'Remove project' }).first(),
    ).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: '移除' }).click()

    // Rail drops the row, the removal receipt lands (M6)…
    await expect(rail.projectRow(alpha)).toHaveCount(0, { timeout: 10_000 })
    await expect(
      page.locator('wa-toast-item').filter({ hasText: '已移除注册' }),
    ).toBeVisible({ timeout: 10_000 })

    // …and the main panel MUST stop naming the removed project — same live
    // page, no reload. This is THE A2 regression lock: the header used to
    // keep showing `alpha` while the rail had already dropped it.
    await expect(headerPath).toHaveText('未选择项目', { timeout: 10_000 })

    // API truth: the registry no longer has alpha; beta is untouched.
    const paths = (await listProjects(page.request)).map((p) => p.path)
    expect(paths).toContain(betaDir)
    expect(paths).not.toContain(alphaDir)

    await resetState(page.request)
    fs.rmSync(alphaDir, { recursive: true, force: true })
    fs.rmSync(betaDir, { recursive: true, force: true })
    expect(collector.clean()).toEqual([])
  })

  test('removing the last project leaves no lingering header while the rail shows the empty state', async ({
    page,
  }) => {
    test.setTimeout(60_000)
    const rail = new ProjectRail(page)
    const t = Date.now()
    const last = `d4-last-${t}`
    const lastDir = path.join(sceneDir(), last)
    fs.mkdirSync(lastDir, { recursive: true })

    await resetState(page.request)
    await addProject(page.request, lastDir)

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.projectRow(last).click()
    const headerPath = page.locator('sebas-dashboard .project-header .path')
    await expect(headerPath).toHaveText(last, { timeout: 10_000 })

    // Remove the ONLY project: the rail falls to 尚未注册项目…
    const row = rail.projectRow(last)
    await row.hover()
    await row.locator('button[title="Project actions"]').click()
    await row.locator('wa-dropdown-item[value="remove"]').click()
    const dialog = page.locator('wa-dialog', { hasText: '移除项目' })
    await expect(
      dialog.locator('h2, [role="heading"]', { hasText: 'Remove project' }).first(),
    ).toBeVisible()
    await dialog.locator('wa-button').filter({ hasText: '移除' }).click()

    await expect(rail.projectRow(last)).toHaveCount(0, { timeout: 10_000 })
    await expect(rail.host).toContainText('尚未注册项目', { timeout: 10_000 })

    // …and the header must not keep naming the removed project (the A2
    // zero-projects form: rail empty, main panel still `last local`).
    await expect(headerPath).toHaveText('未选择项目', { timeout: 10_000 })

    await resetState(page.request)
    fs.rmSync(lastDir, { recursive: true, force: true })
    expect(collector.clean()).toEqual([])
  })

  test('switching the displayed project drops the focused session to the empty state; selecting it again restores', async ({
    page,
  }) => {
    test.setTimeout(60_000)
    const rail = new ProjectRail(page)
    const t = Date.now()
    const beta = `d4-beta-${t}`
    const betaDir = path.join(sceneDir(), beta)
    fs.mkdirSync(betaDir, { recursive: true })

    await resetState(page.request)
    // Project A = the scene project; its session gets a done turn so the
    // focused conversation has content to (not) linger.
    const { name: alphaName } = await ensureSceneProject(page.request)
    await addProject(page.request, betaDir)
    const prompt = `d4-focus-${t}`
    const key = await createSession(page.request, { prompt })
    await waitStatus(page.request, key, ['done'])

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.ensureProjectExpanded(alphaName)
    // Focus the session from the rail (explicit operator action).
    await rail.sessionItem(prompt).click()
    await expect(page.locator('sebas-dashboard sebas-transcript-view')).toBeVisible({
      timeout: 15_000,
    })
    const headerPath = page.locator('sebas-dashboard .project-header .path')
    await expect(headerPath).toHaveText(alphaName, { timeout: 10_000 })
    // Settle past the rail-focus override window (the 500ms throttled summary
    // refresh clears focusOverride; clicking the other project before that
    // would ride the override's explicit-action exemption instead of the
    // reconciliation under test).
    await page.waitForTimeout(800)

    // Switch the displayed project to beta: the focused session belongs to
    // alpha, so the main panel drops to the session-less empty state — the
    // stale conversation must NOT linger, and the composer yields to the
    // stale placeholder instead of offering input for an unfocused session.
    await rail.projectRow(beta).click()
    await expect(headerPath).toHaveText(beta, { timeout: 10_000 })
    await expect(page.locator('sebas-dashboard sebas-transcript-view')).toHaveCount(0, {
      timeout: 10_000,
    })
    await expect(page.locator('sebas-dashboard .empty-stream')).toBeVisible()
    await expect(page.locator('[data-testid="composer-stale-project"]')).toBeVisible()

    // Explicit restore (spec: the stale view MAY be restored by explicit
    // operator action, never by inertia): selecting the session again brings
    // the conversation — and the project context — back.
    await rail.sessionItem(prompt).click()
    await expect(page.locator('sebas-dashboard sebas-transcript-view')).toBeVisible({
      timeout: 15_000,
    })
    await expect(headerPath).toHaveText(alphaName, { timeout: 10_000 })

    await resetState(page.request)
    fs.rmSync(betaDir, { recursive: true, force: true })
    expect(collector.clean()).toEqual([])
  })
})
