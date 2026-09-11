/**
 * Journey — agent mode 选择（add-agent-mode-selection，workbench-interaction-
 * polish 收口：创建时 mode 选择在创建对话框，会话中切换留在会话头部）。
 *
 * - 创建对话框提供权限模式下拉，缺省项是诚实的「默认（逐次询问）」
 *   （= 不发送 mode 字段，wire 上缺省）；composer 工具条不承载 mode 控件。
 * - 创建带 mode=allow 的会话后，会话头部的 mode 标签呈现 desired=allow
 *   （数据源：创建请求记入映射的 desired_mode）。
 * - 头部下拉切换 mode → POST /api/sessions/{key}/mode 送达 → 快照
 *   desired_mode 更新（effective 随执行体反馈落定；fake-claude 对运行时
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

    // composer 工具条不承载 mode 控件（创建选择归对话框、会话中切换归
    // 会话头部）。
    await expect(
      page.locator('sebas-workbench-composer [data-testid="mode-select"]'),
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
    await expect(tag).toContainText('allow')
    expect(collector.clean()).toEqual([])
  })

  test('head mode switch delivers the new desired mode', async ({ page, request }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    const head = page.locator('sebas-dashboard .session-head')
    await expect(head.locator('[data-testid="mode-switch"]')).toBeVisible()

    // Web Awesome 派发标准 change 事件；直接置值 + 派发，走组件自己的监听。
    await head
      .locator('wa-select[data-testid="mode-switch"]')
      .evaluate((el) => {
        ;(el as unknown as { value: string }).value = 'allow'
        el.dispatchEvent(new Event('change', { bubbles: true }))
      })

    // 命令送达后快照 desired_mode 更新（effective 随执行体反馈另行落定）。
    await expect
      .poll(async () => {
        const { detail } = await getSession(request, key)
        return detail?.desired_mode ?? null
      })
      .toBe('allow')
    await expect(head.locator('[data-testid="session-mode"]')).toHaveAttribute(
      'data-mode',
      'allow',
      { timeout: 10_000 },
    )
    expect(collector.clean()).toEqual([])
  })
})
