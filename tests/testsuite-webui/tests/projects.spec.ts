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
    const name = dir.split('/').filter(Boolean).pop()!
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
