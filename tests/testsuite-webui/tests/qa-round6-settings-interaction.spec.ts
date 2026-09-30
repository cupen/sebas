/**
 * Journey — fix-webui-qa-round6 4.2 / 4.3：设置弹窗的对话框与分区交互稳定性。
 *
 * 缺陷账本（2026-10-01 GUI 轮）现象 B/C：
 *   B. 设置分区按钮/新建 agent 按钮点击偶发不生效（对话框 hide 动画期的
 *      顶层遮罩吞点击——wa-dialog 关闭动画未收尾时整个视口的点击都落在
 *      top layer 上）；
 *   C. 「新建 agent」表单保存失败后无法再打开（wa-dialog handleOpenChange
 *      的强制重开竞态：open 翻 false 时组件强制 open=true 走 requestClose，
 *      hide 窗口内翻回 true 命中 no-op 分支）。
 *
 * 修复面：agent 表单/删除对话框改条件渲染（关闭即整棵移出 DOM，与会话
 * 创建对话框 round5 5.3 同一模式）——本 journey 钉修复后的交互合同：
 *   1) 打开设置 → 立即点「＋ 新建 agent」→ 表单打开；取消后**再点一次**
 *      → 全新空表单再开（重开不被吞）。
 *   2) 设置内点击 模型/技能 分区**首次点击**即切换（aria-current 翻转）。
 *
 * 断言口径：可见性与状态属性，不钉像素。wa-dialog 宿主在 top layer 读作
 * hidden——可见性断言落在渲染出的内部元素上（agents.spec 同款纪律）。
 */
import { expect, test } from '@playwright/test'
import { ErrorCollector, SettingsModal, resetState } from './helpers'

test.describe('fix-webui-qa-round6 设置交互稳定性', () => {
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('新建 agent 表单：设置打开后立即可开，取消后可重复再开（4.2）', async ({ page }) => {
    await resetState(page.request)
    const settings = new SettingsModal(page)
    await page.goto('/')
    await settings.openViaSidebar()
    await settings.openSection('Agent')

    const newAgentButton = settings.panel
      .locator('wa-button')
      .filter({ hasText: '新建 agent' })
      .first()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="新建 agent"]')
    const saveButton = dialog.locator('wa-button').filter({ hasText: '保存' })
    const idInput = dialog.locator('wa-input[data-testid="agent-form-id"] input')

    // 1) 打开设置后**立即**点「＋ 新建 agent」：表单打开且 id 字段可输入。
    await newAgentButton.click()
    await expect(saveButton).toBeVisible()
    await idInput.click()
    await idInput.pressSequentially('round6-a')

    // 2) 取消关闭 → 表单整棵移出（条件渲染，无残留模板）。
    await dialog.locator('wa-button').filter({ hasText: '取消' }).click()
    await expect(dialog).toHaveCount(0)

    // 3) 同一设置会话内再点一次：全新空表单再开——hide 窗口的吞点击不再存在。
    await newAgentButton.click()
    await expect(saveButton).toBeVisible()
    await expect(idInput).toHaveValue('')

    // 4) 保存路径仍通（live 值读取）：键入 id → 保存 → 行内动作回执。
    await idInput.fill('')
    await idInput.pressSequentially('round6-b')
    await page.route('**/api/agents/round6-b', async (route) => {
      await route.fulfill({ status: 201, body: '{"created":"round6-b"}' })
    })
    await saveButton.click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="agent-action"]'),
    ).toContainText('已创建 round6-b')
  })

  test('设置分区按钮首次点击即切换（4.3）', async ({ page }) => {
    await resetState(page.request)
    const settings = new SettingsModal(page)
    await page.goto('/')
    await settings.openViaSidebar()

    // 打开即点（不静置）：模型分区首次点击即切换，再点技能同样一次到位。
    await settings.panel.locator('.nav-item', { hasText: '模型' }).click()
    await expect(settings.panel.locator('.nav-item', { hasText: '模型' })).toHaveAttribute(
      'aria-current',
      'true',
    )
    await settings.panel.locator('.nav-item', { hasText: '技能' }).click()
    await expect(settings.panel.locator('.nav-item', { hasText: '技能' })).toHaveAttribute(
      'aria-current',
      'true',
    )
    await expect(settings.panel.locator('.nav-item', { hasText: '模型' })).toHaveAttribute(
      'aria-current',
      'false',
    )
    await settings.close()
  })
})
