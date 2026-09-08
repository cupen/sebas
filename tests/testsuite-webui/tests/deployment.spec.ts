/**
 * Journey 5.4 — detached deployment state (spec: 部署态旅程).
 *
 * 功能：部署态可达性诚实呈现 / 子功能：不可达横幅、降级提示、恢复
 *
 * Runs on the DETACHED dual-process assembly (core + standalone webui,
 * playwright.deployment.config.ts): kill the core → the global
 * "核心不可达" banner appears with the reported cause, adding a project
 * lands degraded with its notice and the composer gate engages; respawn the
 * core → the banner and the gate disappear with no page reload. Process
 * control comes from the reusable helpers/detached fixture; only the
 * journey assertions live here.
 */
import fs from 'node:fs'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  killCore,
  ProjectRail,
  removeProject,
  resetState,
  sceneDir,
  startCore,
  waitReachable,
  waitUnreachable,
  Workbench,
} from './helpers/index'

test.describe('部署态', () => {
  test.describe('核心不可达与恢复', () => {
    test('core 停止 → 横幅/降级/门禁呈现；core 恢复 → 消失', async ({ page, request }) => {
      // Deterministic base on the connected assembly.
      await resetState(request)
      const start = await waitReachable(request)
      expect(start.reachability.ok).toBe(true)

      await page.goto('/')
      const banner = page.locator('sebas-app .core-banner[role="alert"]')
      await expect(banner).toBeHidden()
      const workbench = new Workbench(page)
      await expect(workbench.composer).toBeVisible()
      await expect(workbench.reachabilityWarning).toBeHidden()

      // Kill the core: the webui keeps serving, reachability flips with cause.
      await killCore()
      const cause = await waitUnreachable(request)
      expect(cause.length).toBeGreaterThan(0)

      // Global banner appears with the reported cause (poll-driven, no reload).
      await expect(banner).toContainText('核心不可达：', { timeout: 15_000 })
      const bannerText = (await banner.textContent()) ?? ''
      expect(bannerText.length).toBeGreaterThan('核心不可达：'.length)

      // Degraded project add: lands locally with its notice …
      const scene = sceneDir()
      const projDir = path.join(scene, 'deploy-proj')
      fs.mkdirSync(projDir, { recursive: true })
      const rail = new ProjectRail(page)
      await expect(rail.host).toBeVisible()
      await rail.openAddDialog()
      await rail.addProjectByPath(projDir)
      const notice = rail.host.locator('.degraded-notice[role="status"]')
      await expect(notice).toContainText('核心不可达，已写入本地注册表', {
        timeout: 10_000,
      })
      // … and the composer gate engages while unreachable.
      await expect(workbench.reachabilityWarning).toBeVisible({ timeout: 15_000 })

      // Respawn the core: banner and gate disappear with no page reload.
      await startCore()
      await waitReachable(request, 60_000)
      await expect(banner).toBeHidden({ timeout: 15_000 })
      await expect(workbench.reachabilityWarning).toBeHidden({ timeout: 15_000 })

      await removeProject(request, projDir)
      // NOTE: no collector.clean() assertion — killing the backend
      // mid-session provokes transport noise the journey intentionally
      // exercises; error-silence is covered by the single-process suite.
    })
  })
})
