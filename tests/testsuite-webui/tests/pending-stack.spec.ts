/**
 * Journey — 待执行堆叠区 (workbench-turn-queue 7.1–7.3, design D8).
 *
 * 功能：工作台堆叠区 / 子功能：忙中提交可见、刷新后仍在、关闭点名丢弃、
 * 终结提示（未执行）。
 *
 * 场景基座：沙箱 fake-claude 带 `--slow-ms 800`（tasks.py 装配）。首 prompt
 * 用 "stream" 内容触发场景（5 帧 × 250ms 停顿 + settle）——WORKING 窗口
 * ≈2s，忙中提交确定性入队；帧间 stdin 可读，driver 的 watchdog 探测
 * （1.5s 超时）始终可被应答。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  ProjectRail,
  resetState,
  sendMessage,
  waitStatus,
} from './helpers/index'

test.describe('待执行堆叠区', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('busy-time submission rides the stack, survives refresh, and close names the loss', async ({
    page,
  }) => {
    const rail = new ProjectRail(page)
    await resetState(page.request)

    // 首会话：流式场景让 turn 在飞一段时间。
    const key = await createSession(page.request, { prompt: 'stream' })
    await page.goto('/')

    // turn 已开轮：转录出现流式 chunk（WORKING 窗口内）。
    await expect(
      page.locator('sebas-transcript-view').first(),
    ).toContainText(/chunk/, { timeout: 15_000 })

    // 忙中提交 → 堆叠区可见该提交（不占 transcript，带处置文案）。
    await sendMessage(page.request, key, 'queued while busy')
    const stack = page.locator('sebas-pending-stack [data-testid="pending-stack"]')
    await expect(stack).toBeVisible({ timeout: 5_000 })
    await expect(stack).toContainText('queued while busy')
    await expect(stack).toContainText('待执行 · 第 1 位')

    // 刷新后仍以服务端 payload 呈现（D8 对账语义）。
    await page.reload()
    await expect(page.locator('sebas-pending-stack [data-testid="pending-stack"]')).toContainText(
      'queued while busy',
      { timeout: 10_000 },
    )

    // 等两个回合（排队回合也会跑完）收敛到 done，再造一个确定性的
    // WORKING 窗口用于「带队列关闭」段。
    await waitStatus(page.request, key, ['done'], 20_000)
    await sendMessage(page.request, key, 'stream')
    // 等 turn 真正进入 WORKING（首个内容帧），随后的提交才确定性入队——
    // 卡片尚在 SEED 时提交会开新轮而不是排队。
    await waitStatus(page.request, key, ['working'], 10_000)
    await sendMessage(page.request, key, 'drop me')

    // Rail 关闭该会话：确认对话框点名「将丢弃 1 条」，确认后出现一次性
    // 「未执行」提示，逐条点名被丢弃的提交（7.3）。
    await rail.expandInbox()
    await rail.host
      .locator('li.session-item button[aria-label^="Close"]')
      .first()
      .click()
    // wa-dialog host 在 top layer 读作 hidden——断言渲染出的内部元素
    // （与 settings.spec 的既有纪律一致）。
    const dialog = page.locator('wa-dialog', { hasText: '关闭会话' })
    const discardLine = dialog.locator('[data-testid="close-discards-pending"]')
    await expect(discardLine).toContainText('1', { timeout: 10_000 })
    await dialog.locator('wa-button').filter({ hasText: '关闭会话' }).click()

    // 会话终结：未执行提示在焦点清空后仍然可见（提示是唯一记录）。
    const notice = page.locator('sebas-pending-stack [role="alert"]')
    await expect(notice).toBeVisible({ timeout: 10_000 })
    await expect(notice).toContainText('drop me')
    await expect(notice).toContainText('未被执行')

    expect(collector.clean()).toEqual([])
  })
})
