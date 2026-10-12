/**
 * Journey 3.1 — first paint (spec: 首屏与 reachability).
 *
 * 功能：工作台首屏 / 子功能：首屏结构与 reachability
 *
 * Opens the root path in the auth-off sandbox and asserts the workbench
 * renders its structural surface (project rail, composer, project header)
 * and that reachability shows the sandbox's REAL execution state: the core
 * is connected (no "core not connected" banner), acp is available, and the
 * built-in slot (native/PI) is honestly disabled with its cause — the
 * sandbox has no pi binary installed, and the UI must point at the pi
 * install surface rather than pretend otherwise.
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
      // Nothing focused → empty-stream stage with the rail-entry hint
      // （polish-workbench-walkthrough-ux 已把提示改为操作者中文措辞）。
      await expect(workbench.emptyStream).toBeVisible()
      await expect(workbench.emptyStream).toContainText('未聚焦任何会话')
      await expect(workbench.emptyStream).toContainText('从左侧项目树选择一个会话')

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

      // （make-pi-the-builtin-agent 5.2）内置槽位执行体已从自研内核改指 pi
      // （display `PI`）；不可用的真实条件变为「pi 未安装」——沙箱 PATH 上无
      // pi，故置灰并标注安装指引（不再提 core env 变量）。
      const nativeOption = rail
        .newSessionDialog()
        .locator('wa-option[value="native"]')
      await expect(nativeOption).toBeDisabled()
      // 展示名走 catalog 的 display（native = "PI"）。
      await expect(nativeOption).toContainText('PI')
      // spec「unavailable cause speaks operator language」：指引指向安装面，
      // 且不含环境变量标识（SEBAS_AGENT_PROVIDER_API_KEY 维度已从内置槽位消失）。
      await expect(nativeOption).toContainText('未安装 pi')
      await expect(nativeOption).toContainText('设置 → Agent')
      await expect(nativeOption).not.toContainText('SEBAS_AGENT_PROVIDER_API_KEY')
      await expect(nativeOption).not.toContainText('SEBAS_AGENT_ROUTER_URL')

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
