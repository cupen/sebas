/**
 * Journey — 聚焦联动与 rail 展开持久（fix-webui-approval-restore-and-session-
 * identity 4.1/4.2，tasks.md 7.1）。
 *
 * 功能：会话管理 / 子功能：聚焦联动与展开持久
 *
 * Spec anchors: agent-workbench「Focused session drives project context」的
 * 「rail 切换会话后项目标题跟随」「项目行点击仍独立生效」+「Rail expansion
 * state is persistent and predictable」的「展开状态跨刷新保持」「聚焦会话
 * 所在项目缺省展开」「无操作不自行收起」。
 *
 * The defect this pins: the main region's project title only followed the
 * shell's selectedPath, whose sole writer used to be the project-row click —
 * focusing a session never back-projected its project (4.1). And the rail's
 * expansion was pure in-memory @state: every reload collapsed everything and
 * the collapsed-ness drifted with focus changes (4.2). The contract now:
 * focusing a session projects its project onto the shell, expansion persists
 * via `sebas.rail-expanded`, and a focused project with no record defaults to
 * expanded (materialized so later focus changes never flip it back).
 */
import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  addProject,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  listProjects,
  ProjectRail,
  resetState,
  sceneDir,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('聚焦联动与展开持久', () => {
    test('rail switch and creation landing drive the project title; project-row clicks stay independent', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)
      const workbench = new Workbench(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const { name: projectA } = await ensureSceneProject(page.request)
      // A second project inside the workspace root: clicking ITS row must
      // select it independently of where the focus points.
      const projectBPath = path.join(sceneDir(), `second-${Date.now()}`)
      fs.mkdirSync(projectBPath, { recursive: true })
      await addProject(page.request, projectBPath)
      const projectB = projectBPath.split(/[\\/]/).filter(Boolean).pop()!

      // Two projects, one session each. The last creation leaves the focus
      // pointer on B's session, so the workbench starts with B's context —
      // the baseline the rail-switch scenario has to move away from.
      const keyA = await createSession(page.request, { prompt: 'follow-a' })
      await waitStatus(page.request, keyA, ['done'])
      // B's own session: the creation lands the focus pointer on it.
      const projectBId = (await listProjects(page.request)).find(
        (p) => p.path === projectBPath,
      )!.id
      const keyB = await createSession(page.request, {
        prompt: 'follow-b',
        projectId: projectBId,
      })
      await waitStatus(page.request, keyB, ['done'])

      // Baseline: the focus pointer (B's session) drives the header.
      await page.goto('/')
      await expect(detail.userTurn('follow-b')).toBeVisible({ timeout: 15_000 })
      await expect(workbench.projectHeader.locator('.path:not(.muted)')).toHaveText(projectB, {
        timeout: 10_000,
      })

      // Spec scenario「rail 切换会话后项目标题跟随」: clicking session A's
      // row in the rail focuses it AND the main region's project title shows
      // the focused session's project without any further interaction.
      await rail.expandProject(projectA)
      await rail.sessionItem('follow-a').click()
      await expect(detail.userTurn('follow-a')).toBeVisible({ timeout: 15_000 })
      await expect(workbench.projectHeader.locator('.path:not(.muted)')).toHaveText(projectA, {
        timeout: 10_000,
      })

      // Spec scenario「项目行点击仍独立生效」: clicking a project row itself
      // still selects that project as before.
      await rail.ensureProjectExpanded(projectB)
      await rail.projectRow(projectB).click()
      await expect(workbench.projectHeader.locator('.path:not(.muted)')).toHaveText(projectB, {
        timeout: 10_000,
      })

      // Spec scenario「新建会话落地后项目标题跟随」: creating a session in A
      // lands focus on its placeholder — the title follows back to A.
      await rail.openNewSessionDialog(projectA)
      await rail.pickDialogAgent('claude')
      await rail.confirmNewSessionDialog()
      await expect(workbench.projectHeader.locator('.path:not(.muted)')).toHaveText(projectA, {
        timeout: 15_000,
      })
      expect(page.url()).not.toContain('/sessions/')

      expect(collector.clean()).toEqual([])
    })

    test('expansion persists across reloads; the focused project defaults to expanded with no record', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectA } = await ensureSceneProject(page.request)
      // A second project that never hosts the focused session: expansion
      // records are observable on it without the focus default interfering.
      const projectBPath = path.join(sceneDir(), `expand-${Date.now()}`)
      fs.mkdirSync(projectBPath, { recursive: true })
      await addProject(page.request, projectBPath)
      const projectB = projectBPath.split(/[\\/]/).filter(Boolean).pop()!

      const key = await createSession(page.request, { prompt: 'expand-me' })
      await waitStatus(page.request, key, ['done'])

      // No persisted state at all: the focused session's project shows up
      // EXPANDED on load (聚焦缺省展开).
      await page.goto('/')
      await page.evaluate(() => window.localStorage.clear())
      await page.goto(`/sessions/${key}`)
      const rowA = rail.projectRow(projectA)
      await expect(rowA).toHaveAttribute('aria-expanded', 'true', { timeout: 10_000 })
      // The project WITHOUT the focused session stays collapsed.
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'false')

      // Expand B and reload: the record persists the expansion.
      await rail.projectRow(projectB).click()
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'true')
      await page.reload()
      await expect(rail.projectRow(projectA)).toHaveAttribute('aria-expanded', 'true', {
        timeout: 10_000,
      })
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'true')

      // Reload again with NO rail interaction: the expansion state does not
      // differ from the persisted state (无操作不自行收起).
      await page.reload()
      await expect(rail.projectRow(projectA)).toHaveAttribute('aria-expanded', 'true', {
        timeout: 10_000,
      })
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'true')

      // Collapsing B persists too (B hosts no focused session, so no default
      // interferes): it stays collapsed after another reload, while A keeps
      // its explicit expanded record.
      await rail.projectRow(projectB).click()
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'false')
      await page.reload()
      await expect(rail.projectRow(projectA)).toHaveAttribute('aria-expanded', 'true', {
        timeout: 10_000,
      })
      await expect(rail.projectRow(projectB)).toHaveAttribute('aria-expanded', 'false')

      expect(collector.clean()).toEqual([])
    })
  })
})
