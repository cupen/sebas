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
      await expect(workbench.composer).toBeVisible()

      // No project registered in a fresh sandbox → honest empty rail.
      await expect(rail.host.locator('.empty', { hasText: '尚未注册项目' })).toBeVisible()
      // Workbench header states the no-project truth.
      await expect(workbench.noProjectSelected).toBeVisible()
      // Nothing focused → empty-stream stage, composer in creation mode.
      await expect(workbench.emptyStream).toBeVisible()
      await expect(workbench.newSessionChip).toBeHidden()
      // rail-declutter-unread D6：创建必须显式选项目——无项目可选时提交门禁
      // 禁用并就地说明（不再提供 inbox 绑定）。
      await expect(workbench.composerTextarea).toBeDisabled()
      await expect(
        workbench.composer.locator('[data-testid="project-required"]'),
      ).toContainText('未选择项目')

      // Reachability, honest form: core connected (no warning banner) and the
      // native backend option disabled with its cause spelled out. The
      // provider label shows the store-backed truth (make-core-own-provider-
      // data 3.1: the state seam forwards to the live engine even in the
      // embedded form): the sandbox store is reachable and starts empty, so
      // the label reads "no provider configured" — not the stale
      // "unavailable" wording that used to paper over the non-forwarding
      // composite backend.
      await expect(workbench.reachabilityWarning).toBeHidden()
      await expect(workbench.providerLabel()).toHaveText('no provider configured', {
        timeout: 10_000,
      })
      await expect(workbench.nativeOption()).toBeDisabled()
      await expect(workbench.nativeOption()).toContainText('native')
      await expect(workbench.nativeOption()).toContainText('unavailable')

      // D4 honest absence: with no model options the model dropdown never renders.
      await expect(workbench.modelSelect()).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
