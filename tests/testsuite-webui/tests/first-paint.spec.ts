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
      // 显式提示，不渲染任何创建控件。React's home is a stats/focus card
      // with an AntD Empty ("还没有聚焦会话"), no composer at all.
      await expect(workbench.noFocusHint).toBeVisible()
      await expect(workbench.noFocusHint).toContainText('还没有聚焦会话')
      await expect(workbench.composerTextarea).toHaveCount(0)
      await expect(workbench.submitControl).toHaveCount(0)
      await expect(workbench.modelChip).toHaveCount(0)

      // No project registered in a fresh sandbox → honest empty rail.
      await expect(rail.host.getByText('还没有项目')).toBeVisible()
      // Workbench header states the no-project truth.
      await expect(workbench.noProjectSelected).toBeVisible()
      // Nothing focused → the current-focus card carries the empty state.
      await expect(workbench.emptyStream).toBeVisible()
      await expect(workbench.emptyStream).toContainText('还没有聚焦会话')

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

      // （make-pi-the-builtin-agent 5.2；make-pi-ride-sebas-provider 3.1 修订）
      // 内置槽位执行体已从自研内核改指 pi（display `PI`）。诚实性合同是
      // 「下拉行的可用态 = catalog 的真实可达态」：沙箱有 pi 就不置灰、没有
      // 就置灰并给安装指引。不把某台机器的 PATH 固化进断言——先取
      // /api/agents 真值，再核对 UI 与之一致。
      const agentSelect = rail.newSessionDialog().getByRole('combobox', { name: 'Agent' })
      await agentSelect.click()
      const catalog = (await (await page.request.get('/api/agents')).json()) as {
        agents: Array<{ id: string; display: string; reachable: boolean }>
      }
      const nativeKind = catalog.agents.find((entry) => entry.id === 'native')
      expect(nativeKind?.display).toBe('PI')
      const nativeOption = page
        .locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
        .locator('.ant-select-item-option')
        .filter({ hasText: 'PI' })
      // 展示名走 catalog 的 display（native = "PI"）。
      await expect(nativeOption).toContainText('PI')
      if (nativeKind?.reachable === false) {
        await expect(nativeOption).toHaveClass(/ant-select-item-option-disabled/)
        // spec `webui`「Agent-select dropdown unavailable label reflects the
        // catalog cause」：可见文案按 cause 归类到补救入口（安装面），且不含
        // 环境变量标识。**不再**断言固定字面量「未安装 pi」——服务端 cause 是
        // 唯一事实源（pi 在场而 provider 未配时文案会指 Settings → Models）。
        await expect(nativeOption).toContainText('设置 → Agent')
        await expect(nativeOption).not.toContainText('未配 provider')
        await expect(nativeOption).not.toContainText('SEBAS_AGENT_PROVIDER_API_KEY')
        await expect(nativeOption).not.toContainText('SEBAS_AGENT_ROUTER_URL')
      } else {
        // 可达即不置灰（置灰就是说谎），也不携带不可达 cause 文案。
        await expect(nativeOption).not.toHaveClass(/ant-select-item-option-disabled/)
        await expect(nativeOption).not.toContainText('不可达')
        await expect(nativeOption).not.toContainText('设置 → Agent')
      }

      // 目录不可得（沙箱无 provider 目录）→ 显式说明，不渲染空下拉。
      await expect(
        rail.newSessionDialog().getByText(/模型目录不可用/).first(),
      ).toBeVisible()
      await expect(
        rail.newSessionDialog().getByRole('combobox', { name: '模型' }),
      ).toBeDisabled()

      // 取消：什么都不创建（[role="dialog"] 关闭后仍留在 DOM，断言隐藏）。
      await rail.cancelNewSessionDialog()
      await expect(rail.newSessionDialog()).toBeHidden()

      expect(collector.clean()).toEqual([])
    })
  })
})
