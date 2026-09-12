/**
 * Journey — 提交控件状态机与停止链路（workbench-interaction-polish D4/D5，
 * agent-workbench「Submit control reflects submission and turn state」）。
 *
 * 场景基座：沙箱 fake-claude 带 `--slow-ms 800`（tasks.py 装配），"stream"
 * 触发 5 帧 × 250ms 停顿——WORKING 窗口 ≈2s，停止/排队形态都有确定性窗口。
 *
 * - 流式且输入空 → 红色停止方块；点击走 cancel 链路，turn 离开 working，
 *   控件自动复位；会话存活可继续对话（interrupt-and-heal）。
 * - 流式且有字 → 排队形态；提交进既有 turn-queue（pending-stack 可见）。
 * - 取消不丢弃排队提交（spec「cancel does not drop pending submissions」）。
 * - 空输入且空闲 → 禁用；有字 → send。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  resetState,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('提交控件状态机', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('idle + empty input is disabled; typing enables send', async ({ page }) => {
    const workbench = new Workbench(page)

    await resetState(page.request)
    const { id: projectId } = await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'idle', projectId })
    await waitStatus(page.request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    await expect(workbench.submitControl).toBeVisible()
    await expect(workbench.submitControl).toHaveAttribute('data-state', 'disabled')
    await expect(workbench.submitControl).toBeDisabled()

    await workbench.composerTextarea.fill('ready')
    await expect(workbench.submitControl).toHaveAttribute('data-state', 'send')
    await expect(workbench.submitControl).toBeEnabled()
    expect(collector.clean()).toEqual([])
  })

  test('streaming with empty input offers stop; clicking it cancels and the session survives', async ({
    page,
  }) => {
    const workbench = new Workbench(page)

    await resetState(page.request)
    const { id: projectId } = await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'stream', projectId })
    await page.goto(`/sessions/${key}`)

    // 等 turn 真正进入 WORKING（首个内容帧）。
    await waitStatus(page.request, key, ['working'], 15_000)
    await expect(workbench.submitControl).toHaveAttribute('data-state', 'stop', {
      timeout: 10_000,
    })

    // 点击停止 → cancel 链路；turn 离开 working，控件复位。
    await workbench.submitControl.click()
    await expect
      .poll(async () => (await getSession(page.request, key)).detail?.status_slug, {
        timeout: 15_000,
        intervals: [200],
      })
      .not.toBe('working')
    await expect(workbench.submitControl).not.toHaveAttribute('data-state', 'stop', {
      timeout: 10_000,
    })

    // 会话存活（interrupt-and-heal）：继续对话有效。
    await workbench.sendPrompt('still there?')
    await expect(workbench.composerTextarea).toHaveValue('', { timeout: 10_000 })
    await waitStatus(page.request, key, ['done'], 30_000)
    expect(collector.clean()).toEqual([])
  })

  test('streaming with text offers the queued affordance; queued submissions survive a cancel', async ({
    page,
  }) => {
    const workbench = new Workbench(page)
    const railNote = page.locator('sebas-pending-stack [data-testid="pending-stack"]')

    await resetState(page.request)
    const { id: projectId } = await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'stream', projectId })
    await page.goto(`/sessions/${key}`)

    await waitStatus(page.request, key, ['working'], 15_000)

    // 有字 → 排队形态（非停止、非发送）。
    await workbench.composerTextarea.fill('queued behind this turn')
    await expect(workbench.submitControl).toHaveAttribute('data-state', 'queued', {
      timeout: 10_000,
    })

    // 提交 → 进既有 turn-queue：堆叠区可见该提交（不占 transcript）。
    await workbench.composerTextarea.press('Enter')
    await expect(railNote).toBeVisible({ timeout: 10_000 })
    await expect(railNote).toContainText('queued behind this turn')

    // 输入清空、回到停止形态；再点停止 → 排队提交不被丢弃。
    await expect(workbench.submitControl).toHaveAttribute('data-state', 'stop', {
      timeout: 10_000,
    })
    await workbench.submitControl.click()
    await expect
      .poll(async () => (await getSession(page.request, key)).detail?.status_slug, {
        timeout: 15_000,
        intervals: [200],
      })
      .not.toBe('working')

    // 被中断的 turn 结束后，排队的提交照常执行（interrupt-and-drain）。
    await waitStatus(page.request, key, ['done'], 45_000)
    expect(collector.clean()).toEqual([])
  })
})
