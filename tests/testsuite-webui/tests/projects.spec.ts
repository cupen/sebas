/**
 * Journey 3.6 — project add & remove (spec: 项目增删).
 *
 * 功能：项目管理覆盖 / 子功能：增删、异常拒绝、排序与持久化、选择器交互
 *
 * Add a directory INSIDE the sandbox through the add-project dialog (the
 * folder-picker dialog), then remove it. The dialog's folder-picker tree
 * carries a recorded Web Awesome bug (lazy-load append crashes a null
 * nextSibling — see helpers/errors.ts KNOWN_PAGE_ERRORS), so the
 * deterministic path is the dialog's own manual path field. We assert the
 * functional contract: the project appears in the rail, the workbench
 * header reflects it, and removal (API-only surface) drops it on reload.
 */
import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  addProject,
  addProjectRaw,
  ErrorCollector,
  getBranch,
  listProjects,
  ProjectRail,
  removeProject,
  reorderProjects,
  resetState,
  sceneDir,
} from './helpers/index'

test.describe('项目管理覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('增删', () => {
    test('add via project dialog appears in rail; removed project disappears', async ({ page }) => {
      const rail = new ProjectRail(page)
      const scene = sceneDir()
      // path.join 产出反斜杠路径，basename 必须按两种分隔符切开。
      const projectName = scene.split(/[\\/]/).filter(Boolean).pop()!

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
      // 工作台头部 .path 渲染项目 basename（产品刻意行为：dashboard.ts 只放
      // basename，完整路径放 title 悬浮属性——与 204a901 收紧项目 rail 一致）；
      // 用例据此断言 basename 文本 + title 携带完整路径以锚定路径身份。
      const headerPath = page.locator('sebas-dashboard .project-header .path')
      await expect(headerPath).toHaveText(projectName, { timeout: 10_000 })
      await expect(headerPath).toHaveAttribute('title', scene, { timeout: 10_000 })

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

  test.describe('异常拒绝', () => {
    test('1.1 illegal path 400, duplicate 409, removal persists across reload', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const before = (await listProjects(page.request)).length

      // Illegal: nonexistent path is rejected, registry and rail unchanged.
      const missing = path.join(sceneDir(), `no-such-dir-${Date.now()}`)
      const bad = await addProjectRaw(page.request, missing)
      expect(bad.status).toBe(400)
      expect(bad.body).toContain('不存在')
      expect((await listProjects(page.request)).length).toBe(before)

      // Duplicate: the second register conflicts (409), rail shows a single row.
      const dir = path.join(sceneDir(), `dup-${Date.now()}`)
      fs.mkdirSync(dir, { recursive: true })
      await addProject(page.request, dir)
      const dup = await addProjectRaw(page.request, dir)
      expect(dup.status).toBe(409)
      expect(dup.body).toContain('已注册')
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      const name = dir.split(/[\\/]/).filter(Boolean).pop()!
      await expect(rail.projectRow(name)).toHaveCount(1, { timeout: 10_000 })

      // Removal persists: gone from the API and still gone after reload.
      await removeProject(page.request, dir)
      await expect
        .poll(async () => (await listProjects(page.request)).some((p) => p.path === dir), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(false)
      await page.reload()
      await expect(rail.host).toBeVisible()
      await expect(rail.projectRow(name)).toHaveCount(0)
      fs.rmdirSync(dir)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('排序与持久化', () => {
    test('1.2 reorder persists; git branch shows, plain dir shows none', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const t = Date.now()
      const scene = sceneDir()
      const names = [`sort-a-${t}`, `sort-b-${t}`, `sort-c-${t}`]
      const dirs = names.map((n) => path.join(scene, n))
      for (const d of dirs) fs.mkdirSync(d, { recursive: true })
      const gitDir = path.join(scene, `sort-git-${t}`)
      const gitBranch = `feat-d3-${t}`
      fs.mkdirSync(gitDir, { recursive: true })
      execSync('git init -b main', { cwd: gitDir })
      execSync('git commit --allow-empty -m init', {
        cwd: gitDir,
        env: {
          ...process.env,
          GIT_AUTHOR_NAME: 't',
          GIT_AUTHOR_EMAIL: 't@t',
          GIT_COMMITTER_NAME: 't',
          GIT_COMMITTER_EMAIL: 't@t',
        },
      })
      execSync(`git checkout -b ${gitBranch}`, { cwd: gitDir })
      const plainDir = path.join(scene, `sort-plain-${t}`)
      fs.mkdirSync(plainDir, { recursive: true })

      for (const d of [...dirs, gitDir, plainDir]) await addProject(page.request, d)

      // Rail order helper (project rows only).
      const railNames = () =>
        rail.host.locator('.row .name').allTextContents().then((ts) => ts.map((s) => s.trim()))

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await expect
        .poll(async () => (await railNames()).slice(0, 3), {
          timeout: 10_000,
          intervals: [200],
        })
        .toEqual(names)

      // Reorder the FULL list via the API (a partial list leaves the tail to
      // add-time order, and same-second added_at ties make HashMap tail order
      // nondeterministic — that flaked 2/7 runs before this pinned all five).
      const gitName = `sort-git-${t}`
      const plainName = `sort-plain-${t}`
      const reversed = [...dirs].reverse()
      const wantPaths = [...reversed, gitDir, plainDir]
      const wantOrder = [...names].reverse().concat([gitName, plainName])
      await reorderProjects(page.request, wantPaths)
      await expect
        .poll(async () => (await listProjects(page.request)).map((p) => p.path), {
          timeout: 10_000,
          intervals: [200],
        })
        .toEqual(wantPaths)

      // Order survives reload (registry persistence).
      await page.reload()
      await expect(rail.host).toBeVisible()
      await expect
        .poll(railNames, { timeout: 10_000, intervals: [200] })
        .toEqual(wantOrder)

      // Branch: API truth first (contains, never literal main/master).
      const git = await getBranch(page.request, gitDir)
      expect(git.status).toBe(200)
      expect(git.branch).toContain(gitBranch)
      const plain = await getBranch(page.request, plainDir)
      expect(plain.branch).toBeNull()

      // Rail rendering: git row carries the branch label, plain row has none.
      const gitRow = rail.projectRow(gitName)
      await expect(gitRow.locator('.meta .branch')).toContainText(gitBranch, {
        timeout: 10_000,
      })
      await expect(
        rail.projectRow(plainName).locator('.meta .branch'),
      ).toHaveCount(0)

      // New branch surfaces after reload (defeats the 30s branch TTL cache).
      const gitBranch2 = `feat-d3b-${t}`
      execSync(`git checkout -b ${gitBranch2}`, { cwd: gitDir })
      await page.reload()
      await expect(rail.host).toBeVisible()
      await expect(gitRow.locator('.meta .branch')).toContainText(gitBranch2, {
        timeout: 10_000,
      })

      // Cleanup: unregister everything this journey created.
      await resetState(page.request)
      for (const d of [...dirs, gitDir, plainDir]) fs.rmSync(d, { recursive: true, force: true })

      expect(collector.clean()).toEqual([])
    })
  })

  /**
   * Journey P.x — project picker interactions (phase-3 tasks P1–P2).
   *
   * P1 drives the folder-picker TREE (not the manual path field): lazy-expand
   * a prepared parent dir, click-select the child, and submit — the full
   * tree→input→rail loop. The wa-tree lazy-append pageerror is a recorded
   * known-benign (helpers/errors.ts) and stays filtered.
   * P2 pins the inline-error contract: empty path disables submit; a missing
   * path surfaces the server rejection inside the dialog without closing it
   * and without touching the registry.
   */
  test.describe('选择器交互', () => {
    test('P1 tree expand, click-select fills path, submit lands in rail', async ({ page }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const t = Date.now()
      const work = path.join(sceneDir(), 'work')
      const parent = path.join(work, `pick-parent-${t}`)
      const child = path.join(parent, `pick-child-${t}`)
      fs.mkdirSync(child, { recursive: true })

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()

      const dialog = rail.addDialog()
      const tree = dialog.locator('sebas-folder-picker')
      // The picker roots at the server work dir.
      await expect(tree.locator('.root-path')).toContainText(work, { timeout: 10_000 })

      // Lazy-expand the prepared parent; the child appears underneath it.
      // data-path 携带反斜杠路径——原始 CSS 属性选择器会把 `\U` 当转义
      // 吃掉，永匹配不到；fixture 名带时间戳，按文本过滤是唯一匹配。
      const parentItem = tree.locator('wa-tree-item').filter({ hasText: `pick-parent-${t}` })
      await expect(parentItem).toBeVisible({ timeout: 10_000 })
      await parentItem.click()
      // 子项渲染在 slot="children" 槽内且是父项的 DOM 后代——按 slot 圈定，
      // 否则父项的 hasText 也会命中子项文本造成 strict violation。
      const childItem = tree
        .locator('wa-tree-item[slot="children"]')
        .filter({ hasText: `pick-child-${t}` })
      await expect(childItem).toBeVisible({ timeout: 10_000 })

      // Click-select fills the manual path field (the dialog's submit gate).
      await childItem.click()
      // 选择器填入「根回显(反斜杠) + / 子名」的混合分隔符形——这是
      // joinChildPath 的既定契约（有单测锚定）；注册经后端
      // canonicalize_plain 归一为纯反斜杠普通形，故两处期望不同形。
      const pathInput = dialog.locator('wa-input[label="Project path"] input')
      await expect(pathInput).toHaveValue(`${work}/pick-parent-${t}/pick-child-${t}`, {
        timeout: 10_000,
      })

      // Submit through the dialog footer; the project lands in the rail.
      await dialog.locator('wa-button').filter({ hasText: 'Add project' }).click()
      await expect
        .poll(async () => (await listProjects(page.request)).some((p) => p.path === child), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(true)
      await expect(rail.projectRow(`pick-child-${t}`)).toBeVisible({ timeout: 10_000 })

      // Cleanup: unregister and remove the fixture dirs.
      await removeProject(page.request, child)
      fs.rmSync(parent, { recursive: true, force: true })

      expect(collector.clean()).toEqual([])
    })

    test('P2 empty path disables submit; missing path errors inline, dialog stays', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const before = (await listProjects(page.request)).length

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()

      const dialog = rail.addDialog()
      const submit = dialog.locator('wa-button').filter({ hasText: 'Add project' })
      // Empty path: the footer submit carries the disabled attribute (wa-button
      // is a custom element — assert the attribute, not native semantics).
      await expect(submit).toHaveAttribute('disabled', '')

      // Missing path: server rejects, the rejection surfaces INSIDE the dialog
      // (no close, no registry write).
      const missing = path.join(sceneDir(), `no-such-dir-${Date.now()}`)
      const pathInput = dialog.locator('wa-input[label="Project path"] input')
      await pathInput.click()
      await pathInput.pressSequentially(missing)
      await expect(submit).not.toHaveAttribute('disabled', '')
      await submit.click()
      // The rejection renders in the div right after the path field (the
      // dialog's only inline-error slot — a bare `div` matcher would also hit
      // its ancestors, so scope structurally).
      const inlineError = dialog.locator('wa-input[label="Project path"] + div')
      await expect(inlineError).toContainText('不存在', { timeout: 10_000 })
      // The dialog is still open (heading visible) and the registry untouched.
      await expect(
        dialog.locator('h2, [role="heading"]', { hasText: 'Add project' }).first(),
      ).toBeVisible()
      expect((await listProjects(page.request)).length).toBe(before)

      await dialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()

      expect(collector.clean()).toEqual([])
    })
  })
})
