/**
 * Journey — agent mode 选择（add-agent-mode-selection，workbench-live-
 * conversation-flow 4.2 收口：创建时 mode 选择在创建对话框，会话中切换在
 * composer 底沿——头部去交互化后只留展示）。
 *
 * - 创建对话框提供权限模式下拉，缺省项是诚实的「默认（逐次询问）」
 *   （= 不发送 mode 字段，wire 上缺省）。
 * - 创建带 mode=allow 的会话后，会话头部的 mode 标签呈现 desired=allow
 *   （数据源：创建请求记入映射的 desired_mode）。
 * - composer 底沿的 mode 下拉切换 → POST /api/sessions/{key}/mode 送达 →
 *   快照 desired_mode 更新（effective 随执行体反馈落定；fake-claude 对运行时
 *   set_permission_mode 回 success，测试只断言 desired 即时更新——不把
 *   「命令送达」假装成「已生效」）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('agent mode 选择', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('creation dialog offers the permission-mode select with an honest default; composer carries no mode control', async ({
    page,
  }) => {
    const rail = new ProjectRail(page)
    await resetState(page.request)
    const { name: projectName } = await ensureSceneProject(page.request)
    await page.goto('/')

    await rail.openNewSessionDialog(projectName)
    const modeSelect = rail.newSessionDialog().locator('[data-testid="dialog-mode-select"]')
    await expect(modeSelect).toBeVisible()
    // 缺省项 = agent 默认（不发送 mode 字段）；四个控制面词汇都在列。
    await expect(modeSelect).toContainText('默认（逐次询问）')
    await expect(modeSelect).toContainText('ask')
    await expect(modeSelect).toContainText('edit')
    await expect(modeSelect).toContainText('allow')
    await expect(modeSelect).toContainText('auto')

    // 无聚焦会话时 composer 只渲染 rail 创建指引——不承载任何 mode 控件
    // （创建选择归对话框；会话中切换归 composer 底沿，见下方用例）。
    await expect(
      page.locator('sebas-workbench-composer [data-testid="mode-switch"]'),
    ).toHaveCount(0)
    await expect(
      page.locator('sebas-workbench-composer wa-select[aria-label="Permission mode"]'),
    ).toHaveCount(0)

    await rail.cancelNewSessionDialog()
    expect(collector.clean()).toEqual([])
  })

  test('create with mode=allow surfaces the desired mode tag in the session head', async ({
    page,
    request,
  }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'hello', mode: 'allow' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    const head = page.locator('sebas-dashboard .session-head')
    await expect(head).toBeVisible()
    const tag = head.locator('[data-testid="session-mode"]')
    await expect(tag).toBeVisible()
    await expect(tag).toHaveAttribute('data-mode', 'allow')
    // 展示文案走共享词汇 modeBadgeLabel（4.2 模式章合一）：allow = 「放行」。
    await expect(tag).toContainText('放行')
    expect(collector.clean()).toEqual([])
  })

  test('composer mode switch delivers the new desired mode', async ({ page, request }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    // 4.2 头部去交互化：mode 切换控件在 composer 底沿（头部只留展示标签）。
    const composerSwitch = page.locator(
      'sebas-workbench-composer wa-select[data-testid="mode-switch"]',
    )
    await expect(composerSwitch).toBeVisible()

    // Web Awesome 派发标准 change 事件；直接置值 + 派发，走组件自己的监听。
    await composerSwitch.evaluate((el) => {
      ;(el as unknown as { value: string }).value = 'allow'
      el.dispatchEvent(new Event('change', { bubbles: true }))
    })

    // 命令送达后快照 desired_mode 更新（effective 随执行体反馈另行落定）；
    // 头部标签（展示）同步翻到 allow。
    await expect
      .poll(async () => {
        const { detail } = await getSession(request, key)
        return detail?.desired_mode ?? null
      })
      .toBe('allow')
    await expect(
      page.locator('sebas-dashboard .session-head [data-testid="session-mode"]'),
    ).toHaveAttribute('data-mode', 'allow', { timeout: 10_000 })
    expect(collector.clean()).toEqual([])
  })

  test.describe('composer 模式下拉形态', () => {
    test('composer mode dropdown options stay single-line (round3 6.2)', async ({
      page,
      request,
    }) => {
      // round3 4.1 的放宽写在 document 级 wa-overrides.css，够不到 shadow 树
      // 里的 wa-select——composer 模式下拉的中文长文案（「allow（放行并留审
      // 计）」等）仍按字折行挤高选项行。修复把同款规则（listbox min-width:
      // max-content / 320px 封顶 + option 单行省略）补进 workbench-composer
      // 自己的样式表。本旅程展开真实下拉，量测每个选项盒高度：单行 ≈26px
      // 量级，折行必然 ≥2×行高（≈44px+）——40px 阈值两侧有充分间隔。
      const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      const modeSwitch = page.locator('sebas-workbench-composer wa-select[data-testid="mode-switch"]')
      await expect(modeSwitch).toBeVisible()

      await modeSwitch.click()
      // 五个选项（默认 + 四模式）都可见——面板真实展开，不是残留浮层。
      const options = modeSwitch.locator('wa-option')
      await expect(options).toHaveCount(5, { timeout: 10_000 })
      const longest = options.filter({ hasText: 'allow（放行并留审计）' })
      await expect(longest).toBeVisible()

      const heights = await options.evaluateAll((els) =>
        els.map((el) => el.getBoundingClientRect().height),
      )
      expect(heights.length).toBe(5)
      for (const h of heights) {
        expect(h, `option box height ${h}px must be single-line (<40px)`).toBeLessThan(40)
      }

      // 选项文案不折行的机制面：label part 计算样式 white-space: nowrap
      // （shadow 内样式表命中的直接证据；agent 下拉当时的假阳性就是短文案
      // 掩盖了规则缺席）。
      const labelWhiteSpace = await longest.evaluate((el) => {
        const label = el.shadowRoot?.querySelector('[part="label"]')
        return label ? getComputedStyle(label).whiteSpace : ''
      })
      expect(labelWhiteSpace).toBe('nowrap')

      // 收起，不做选择（change 只在真选择时派发）。
      await page.keyboard.press('Escape')
      expect(collector.clean()).toEqual([])
    })
  })
})
