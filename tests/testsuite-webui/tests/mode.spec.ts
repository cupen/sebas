/**
 * Journey — agent mode 选择（add-agent-mode-selection，workbench-live-
 * conversation-flow 4.2 收口：创建时 mode 选择在创建对话框，会话中切换在
 * composer 底沿——头部去交互化后只留展示）。
 *
 * - 创建对话框提供权限模式下拉，恰为 Ask/Edit/Allow/Auto 四词、无空值
 *   占位项（simplify-mode-menus），预选 ask；wire 值 ask|edit|allow|auto
 *   不变。模式解释走非内联通道：选项悬浮 title + 下拉 hint 行随选中项。
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
  pickSelectOption,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('agent mode 选择', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('creation dialog offers the four-word mode select prefilled with ask; composer carries no mode control', async ({
    page,
  }) => {
    const rail = new ProjectRail(page)
    await resetState(page.request)
    const { name: projectName } = await ensureSceneProject(page.request)
    await page.goto('/')

    await rail.openNewSessionDialog(projectName)
    const modeSelect = rail.newSessionDialog().getByRole('combobox', { name: '权限模式' })
    await expect(modeSelect).toBeVisible()
    // Four-word mode vocabulary; the combined dialog preselects ask. AntD 6
    // renders the selected label in `.ant-select-content`, not the input.
    await expect(
      rail.newSessionDialog().locator('.ant-select-content').filter({ hasText: 'Ask' }),
    ).toBeVisible()
    await modeSelect.click()
    const optionRows = page
      .locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
      .locator('.ant-select-item-option')
    await expect(optionRows).toHaveCount(4)
    await expect(optionRows.nth(0)).toContainText('Ask')
    await expect(optionRows.nth(1)).toContainText('Edit')
    await expect(optionRows.nth(2)).toContainText('Allow')
    await expect(optionRows.nth(3)).toContainText('Auto')
    const labels = await optionRows.evaluateAll((els) =>
      els.map((el) => (el.textContent ?? '').trim()),
    )
    expect(labels).toEqual([
      'Ask · 逐次询问',
      'Edit · 自动接受编辑',
      'Allow · 放行',
      'Auto · 自动执行',
    ])
    // Non-inline description channel: AntD option titles carry the label; the
    // React surface has no separate description hint text.
    const askTitle = await optionRows.nth(0).getAttribute('title')
    expect(askTitle).toBe('Ask · 逐次询问')
    await page.keyboard.press('Escape')

    // 无聚焦会话时 composer 只渲染 rail 创建指引——不承载任何 mode 控件
    // （创建选择归对话框；会话中切换归 composer 底沿，见下方用例）。
    await expect(page.getByTestId('composer')).toHaveCount(0)

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
    await expect(page.getByTestId('workbench-session')).toBeVisible()
    // React presents the desired mode on the composer's mode control (the
    // session head carries status/model chips only): allow = 放行.
    const modeSelect = page
      .getByTestId('composer')
      .getByRole('combobox', { name: '权限模式' })
    await expect(modeSelect).toBeVisible()
    // AntD 6 的单选标签是 .ant-select-content 的文本。
    const modeContent = page
      .getByTestId('composer')
      .locator('.ant-select-content')
      .filter({ hasText: 'Allow' })
    await expect(modeContent).toContainText('Allow')
    await expect(modeContent).toContainText('放行')
    expect(collector.clean()).toEqual([])
  })

  test('composer mode switch delivers the new desired mode', async ({ page, request }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    // 4.2 头部去交互化：mode 切换控件在 composer 底沿。
    const composerSwitch = page
      .getByTestId('composer')
      .getByRole('combobox', { name: '权限模式' })
    await expect(composerSwitch).toBeVisible()
    const switchedContent = page
      .getByTestId('composer')
      .locator('.ant-select-content')
      .filter({ hasText: 'Allow' })

    await pickSelectOption(page, composerSwitch, 'Allow')

    // 命令送达后快照 desired_mode 更新（effective 随执行体反馈另行落定）；
    // composer 的展示同步翻到 allow。
    await expect
      .poll(async () => {
        const { detail } = await getSession(request, key)
        return detail?.desired_mode ?? null
      })
      .toBe('allow')
    await expect(switchedContent).toBeVisible({ timeout: 10_000 })
    expect(collector.clean()).toEqual([])
  })

  // fix-webui-qa-round6 5.1（spec「等价模式有标注」浏览器半边）：claude 驱动
  // 的会话上 allow 与 auto 同档（bypass）——操作员打开 composer 模式菜单时，
  // 两个选项的 helper text（悬浮 title）必须互相点名等价；标签保持裸词、
  // 发送值与门控语义不受标注影响（前端单测 workbench-composer.test.ts 钉词
  // 汇，本旅程钉真实下拉里的呈现）。
  test('composer mode menu states the allow/auto equivalence in option helper text (round6 5.1)', async ({
    page,
    request,
  }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    const modeSwitch = page
      .getByTestId('composer')
      .getByRole('combobox', { name: '权限模式' })
    await expect(modeSwitch).toBeVisible()

    // 打开真实下拉（spec 场景字面：operator opens the composer mode menu）。
    await modeSwitch.click()
    const options = page
      .locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
      .locator('.ant-select-item-option')
    await expect(options).toHaveCount(4, { timeout: 10_000 })

    const meta = await options.evaluateAll((els) =>
      els.map((el) => ({
        title: el.getAttribute('title') ?? '',
        label: (el.textContent ?? '').trim(),
      })),
    )
    const allow = meta.find((o) => o.label.startsWith('Allow'))
    const auto = meta.find((o) => o.label.startsWith('Auto'))
    expect(allow?.title).toContain('与 Auto 等价')
    expect(auto?.title).toContain('与 Allow 等价')
    // 标签双语化（fix-webui-qa-round7 4.1；等价声明仍走 title helper）。
    expect(allow?.label).toBe('Allow · 放行')
    expect(auto?.label).toBe('Auto · 自动执行')

    // 收起，不做选择（change 只在真选择时派发）。
    await page.keyboard.press('Escape')
    expect(collector.clean()).toEqual([])
  })

  test.describe('composer 模式下拉形态', () => {
    test('composer mode dropdown options stay single-line (round3 6.2)', async ({
      page,
      request,
    }) => {
      // round3 4.1 的放宽写在 document 级 wa-overrides.css，够不到 shadow 树
      // 里的 [role="combobox"]——composer 模式下拉的长文案仍会按字折行挤高选项行。
      // 修复把同款规则（listbox min-width: max-content / 320px 封顶 + option
      // 单行省略）补进 workbench-composer 自己的样式表。本旅程展开真实下拉，
      // 量测每个选项盒高度：单行 ≈26px 量级，折行必然 ≥2×行高（≈44px+）
      // ——40px 阈值两侧有充分间隔。（add-agent-settings-and-session-titles
      // 7.2 后选项标签已瘦身成 Ask/Edit/Allow/Auto——折行风险随之消失，但
      // 单行契约（nowrap + <40px）仍是这个下拉的形状规约，旅程保留。）
      const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      const modeSwitch = page
        .getByTestId('composer')
        .getByRole('combobox', { name: '权限模式' })
      await expect(modeSwitch).toBeVisible()

      await modeSwitch.click()
      // 四个选项都可见——面板真实展开，不是残留浮层（simplify-mode-menus
      // 后恰为四模式，空值占位项已退役）。
      const options = page
        .locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
        .locator('.ant-select-item-option')
      await expect(options).toHaveCount(4, { timeout: 10_000 })
      const longest = options.filter({ hasText: 'Allow' })
      await expect(longest).toBeVisible()

      const heights = await options.evaluateAll((els) =>
        els.map((el) => el.getBoundingClientRect().height),
      )
      expect(heights.length).toBe(4)
      for (const h of heights) {
        expect(h, `option box height ${h}px must be single-line (<40px)`).toBeLessThan(40)
      }

      // 选项文案不折行的机制面：option content 的计算样式 white-space。
      const labelWhiteSpace = await longest.evaluate((el) => {
        const label = el.querySelector('.ant-select-item-option-content') ?? el
        return getComputedStyle(label).whiteSpace
      })
      expect(labelWhiteSpace).toBe('nowrap')

      // 收起，不做选择（change 只在真选择时派发）。
      await page.keyboard.press('Escape')
      expect(collector.clean()).toEqual([])
    })
  })
})
