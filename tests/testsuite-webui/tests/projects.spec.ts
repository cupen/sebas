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
import { expect, test, type APIRequestContext } from '@playwright/test'
import {
  addProject,
  addProjectRaw,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getBranch,
  listProjects,
  ProjectRail,
  removeProject,
  removeProjectRaw,
  reorderProjects,
  resetState,
  sceneDir,
  waitStatus,
} from './helpers/index'

// workbench-agent-wire-fix 2.5: the wire identifies projects by stable id;
// resolve a path → id through the list endpoint.
async function projectIdOf(
  request: APIRequestContext,
  path: string,
): Promise<string> {
  const p = (await listProjects(request)).find((x) => x.path === path)
  if (!p) throw new Error(`projectIdOf: project not registered: ${path}`)
  return p.id
}

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

      // Order-independence guard (rail-declutter-unread cascade fix): a
      // preceding journey may have left the scene project registered — the
      // dialog add would then 409 and the header would stay unselected.
      await resetState(page.request)

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()

      // The dialog's manual path field (this is the folder-picker dialog;
      // typing the directory avoids the recorded ul lazy-load bug).
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
      // 选中新项目后，工作台头部呈现项目路径（React 首页副标题即项目 path，
      // 完整路径是路径身份的唯一可见真源）。
      await rail.projectRow(projectName).click()
      const headerPath = page.getByTestId('workbench-home')
      await expect(headerPath).toContainText(projectName, { timeout: 10_000 })
      await expect(headerPath).toContainText(scene, { timeout: 10_000 })

      // Remove via the API surface; the rail reflects it on reload.
      await removeProject(page.request, await projectIdOf(page.request, scene))
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

    test('… menu removal: blocked while live sessions exist, precheck and typed rejection agree', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const rail = new ProjectRail(page)
      const scene = sceneDir()
      const projectName = scene.split(/[\\/]/).filter(Boolean).pop()!

      await resetState(page.request)
      const { id: projectId } = await ensureSceneProject(page.request)
      // 造一个绑定该项目的非归档会话（done 也在册——归档才算离开）。
      const tag = `remove-guard-${Date.now()}`
      const key = await createSession(page.request, { prompt: tag, projectId })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandProject(projectName)
      // … 菜单 → 移除 → 弹窗内提交后由服务端 409 就地说明「先归档/关闭（N 个会话）」。
      // React 的菜单触发器（项目操作 <name>）是项目行的兄弟节点，不是行内
      // 后代——按无障碍名直接定位，避免跨过行元素找按钮。
      await rail.openProjectMenu(projectName)
      await page.getByRole('menuitem', { name: '移除项目', exact: true }).click()
      const dialog = page.getByRole('dialog', { name: '移除项目' })
      await expect(dialog).toBeVisible()
      await dialog.getByRole('button', { name: /移\s*除/ }).click()
      const blocked = dialog.getByRole('alert')
      await expect(blocked).toBeVisible({ timeout: 10_000 })
      await expect(blocked).toContainText('1')

      // 后端是强制层：绕过 UI 直接移除 → 409 typed rejection（带会话数）。
      const raw = await removeProjectRaw(page.request, projectId)
      expect(raw.status).toBe(409)
      const rejection = JSON.parse(raw.body) as { code: string; session_count: number }
      expect(rejection.code).toBe('project_has_live_sessions')
      expect(rejection.session_count).toBe(1)
      await expect(rail.projectRow(projectName)).toBeVisible()

      // Cleanup: close the session so resetState's project removal passes.
      await page.request.post(`/api/sessions/${key}/close`)
      await resetState(page.request)
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

      // Illegal: a nonexistent path cannot be canonicalized, so it is judged
      // out-of-scope with the SAME wording as a certain escape
      // (add-workspace-root：范围判定先行于存在性，不区分文案)；registry
      // and rail unchanged.
      const missing = path.join(sceneDir(), `no-such-dir-${Date.now()}`)
      const bad = await addProjectRaw(page.request, missing)
      expect(bad.status).toBe(400)
      expect(bad.body).toContain('路径超出允许范围')
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
      await removeProject(page.request, await projectIdOf(page.request, dir))
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
    test('1.2 reorder persists; branch probe drives state, rail hides the branch name (D8)', async ({
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

      // Rail order helper (project rows only): the row button's accessible
      // name is `name + session count`; strip the count for the name.
      const railNames = () =>
        rail.host
          .locator('button[aria-expanded]')
          .evaluateAll((els) =>
            els
              .map((el) => el.getAttribute('aria-label') ?? '')
              .filter((label) => label !== '' && !label.startsWith('历史 '))
              .map((label) => label.replace(/\s+\d+$/, '')),
          )

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
      const wantIds = await Promise.all(wantPaths.map((p) => projectIdOf(page.request, p)))
      await reorderProjects(page.request, wantIds)
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
      const git = await getBranch(page.request, await projectIdOf(page.request, gitDir))
      expect(git.status).toBe(200)
      expect(git.branch).toContain(gitBranch)
      const plain = await getBranch(page.request, await projectIdOf(page.request, plainDir))
      expect(plain.branch).toBeNull()

      // Rail rendering (rail-declutter-unread D8): the branch NAME is no
      // longer shown in the project row — but the probe still runs (the API
      // truth above). The row's accessible name carries the project name and
      // count only, never the branch.
      const gitRow = rail.projectRow(gitName)
      await expect(gitRow).not.toContainText(gitBranch)
      await expect(rail.projectRow(plainName)).not.toContainText('main')

      // New branch still surfaces via the API after reload (defeats the 30s
      // branch TTL cache) — the probe chain survives, only the label is gone.
      const gitBranch2 = `feat-d3b-${t}`
      execSync(`git checkout -b ${gitBranch2}`, { cwd: gitDir })
      await page.reload()
      await expect(rail.host).toBeVisible()
      await expect
        .poll(
          async () => (await getBranch(page.request, await projectIdOf(page.request, gitDir))).branch,
          { timeout: 10_000, intervals: [200] },
        )
        .toContain(gitBranch2)
      await expect(gitRow).not.toContainText(gitBranch2)

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
   * tree→input→rail loop. The ul lazy-append pageerror is a recorded
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
      const scene = sceneDir()
      const work = path.join(scene, 'work')
      const parent = path.join(work, `pick-parent-${t}`)
      const child = path.join(parent, `pick-child-${t}`)
      fs.mkdirSync(child, { recursive: true })

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()

      const dialog = rail.addDialog()
      // 选择器根 = workspace root（沙箱里即场景目录）；路径以 code 元素的
      // 文本如实呈现（antd Typography.Text 把 title 放在外层 span 上，code
      // 本身没有 title 属性，因此按可见文本断言）。
      const pathDisplay = dialog.locator('code').first()
      await expect(pathDisplay).toContainText(scene.replace(/\\/g, '/'), { timeout: 10_000 })
      // 目录树按层导航：root → work → 夹具父目录 → 子目录，当前路径逐层推进。
      const openDir = async (name: string) => {
        const item = dialog.getByRole('button', { name, exact: true })
        await expect(item).toBeVisible({ timeout: 10_000 })
        await item.click()
        return item
      }
      await openDir('work')
      await openDir(`pick-parent-${t}`)
      await openDir(`pick-child-${t}`)
      // 进入子目录后「选择此目录」把该路径回填为提交值。
      const expectedSelected = `${scene}/work/pick-parent-${t}/pick-child-${t}`.replace(/\\/g, '/')
      await expect
        .poll(async () => (await pathDisplay.innerText()).trim().replace(/\\/g, '/'), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(expectedSelected)

      // Submit through the dialog footer; the project lands in the rail.
      await dialog.getByRole('button', { name: '选择此目录' }).click()
      await expect
        .poll(async () => (await listProjects(page.request)).some((p) => p.path === child), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(true)
      await expect(rail.projectRow(`pick-child-${t}`)).toBeVisible({ timeout: 10_000 })

      // Cleanup: unregister and remove the fixture dirs.
      await removeProject(page.request, await projectIdOf(page.request, child))
      fs.rmSync(parent, { recursive: true, force: true })

      expect(collector.clean()).toEqual([])
    })

    test('P2 empty path disables submit; missing path gets the precheck reason, dialog stays', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const before = (await listProjects(page.request)).length

      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()

      const dialog = rail.addDialog()
      // Empty manual path: the man-path submit is disabled (tree submit would
      // register the current root, so the manual gate is the empty-path one).
      const pathInput = dialog.getByLabel('项目路径')
      const submit = dialog.getByRole('button', { name: '使用此路径' })
      await expect(submit).toBeDisabled()

      // Missing path: the browse-dirs precheck answers with the concrete
      // reason under the input and the submit STAYS disabled.
      const missing = path.join(sceneDir(), `no-such-dir-${Date.now()}`)
      await pathInput.click()
      await pathInput.pressSequentially(missing)
      const hint = dialog.getByRole('alert')
      await expect(hint).toContainText('不存在', { timeout: 10_000 })
      await expect(submit).toBeDisabled()
      // The dialog is still open (title visible) and the registry untouched.
      await expect(dialog.getByText('选择项目目录', { exact: true })).toBeVisible()
      expect((await listProjects(page.request)).length).toBe(before)

      await dialog.getByRole('button', { name: /取\s*消/ }).click()

      expect(collector.clean()).toEqual([])
    })
  })
})
