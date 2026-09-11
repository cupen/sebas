/**
 * Journey 3.1 — first paint (spec: 首屏与 reachability).
 *
 * 功能：工作台首屏 / 子功能：首屏结构与 reachability
 *
 * Opens the root path in the auth-off sandbox and asserts the workbench
 * renders its structural surface (project rail, composer, project header)
 * and that reachability shows the sandbox's REAL execution state: the core
 * is connected (no "core not connected" banner), acp is available, and the
 * native backend is honestly disabled with its cause — the sandbox has no
 * provider credentials, and the UI must not pretend otherwise.
 */
import { expect, test } from '@playwright/test'
import {
  AppShell,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  Workbench,
} from './helpers/index'

test.describe('工作台首屏', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首屏结构与 reachability', () => {
    test('renders rail, composer and honest reachability', async ({ page }) => {
      const shell = new AppShell(page)
      const rail = new ProjectRail(page)
      const workbench = new Workbench(page)

      // Deterministic base: no sessions (closing clears the focus pointer),
      // no projects — independent of journey ordering and retry leftovers.
      await resetState(page.request)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      await expect(rail.host).toBeVisible()
      await expect(workbench.host).toBeVisible()
      // workbench-interaction-polish 4.1：composer 纯跟随化——无聚焦时给
      // 显式提示（指向 rail 创建入口），不渲染任何创建控件。
      await expect(workbench.noFocusHint).toBeVisible()
      await expect(workbench.noFocusHint).toContainText('+')
      await expect(workbench.composerTextarea).toHaveCount(0)
      await expect(workbench.submitControl).toHaveCount(0)
      await expect(workbench.modelChip).toHaveCount(0)

      // No project registered in a fresh sandbox → honest empty rail.
      await expect(rail.host.locator('.empty', { hasText: '尚未注册项目' })).toBeVisible()
      // Workbench header states the no-project truth.
      await expect(workbench.noProjectSelected).toBeVisible()
      // Nothing focused → empty-stream stage with the rail-entry hint.
      await expect(workbench.emptyStream).toBeVisible()
      await expect(workbench.emptyStream).toContainText('sidebar')

      // Reachability, honest form: core connected (no warning banner).
      await expect(workbench.reachabilityWarning).toBeHidden()

      // D4 honest absence: with no focused session there is no submit
      // control anywhere (already asserted above) — and no model chip.

      expect(collector.clean()).toEqual([])
    })

    test('the creation dialog is the only agent choice and marks native honestly', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      await page.goto('/')
      await expect(rail.host).toBeVisible()

      await rail.openNewSessionDialog(projectName)

      // agent 必选：native 不可达时禁选并标注 cause（沙箱无 provider 凭据）。
      const nativeOption = rail
        .newSessionDialog()
        .locator('wa-option[value="native"]')
      await expect(nativeOption).toBeDisabled()
      await expect(nativeOption).toContainText('native')
      await expect(nativeOption).toContainText('unavailable')

      // 目录不可得（沙箱无 provider 目录）→ 显式说明，不渲染空下拉。
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-catalog-unavailable"]'),
      ).toBeVisible()
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-provider-select"]'),
      ).toHaveCount(0)

      // 取消：什么都不创建（wa-dialog 关闭后仍留在 DOM，断言隐藏）。
      await rail.cancelNewSessionDialog()
      await expect(rail.newSessionDialog()).toBeHidden()

      expect(collector.clean()).toEqual([])
    })
  })
})
