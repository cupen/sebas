/**
 * Journey — core 挂起窗口（SIGSTOP）的提交回执与免刷新恢复。
 *
 * 与 submit-ack.spec 的差别：那里用 `page.route` 只延迟 composer 的 HTTP
 * POST（后端进程健康）；这里把 harness 拉起的 core 进程整个 SIGSTOP——
 * HTTP 与 WS 同时静默、TCP 仍处建立态，是「上游真挂起」形态（卡死进程、
 * 网络分区半开连接都是这一类），任何 route 注入都复现不了。驱动手柄是
 * tasks.py 发布的 `<scene>/pids.json`（helpers/coreproc.ts）。
 *
 * 断言（spec live-turn-stream「Submission acknowledgment is bounded」在
 * 挂起形态下的延伸）：
 * - 挂起窗口内提交：本地等待回执指示即时可见（远小于 5s 预算），草稿
 *   保留、无 NetworkError、无 ws-down 横幅（TCP 在，客户端无从宣告断连
 *   ——可见反馈由等待回执指示承载，而不是虚假的失败/成功）；
 * - 解冻后免刷新闭环：回执到达、指示退场、回合照常落账（prompt + 回复
 *   追加进转录），composer 复位可继续输入。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  freezeCore,
  resetState,
  thawCore,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('core 挂起窗口（SIGSTOP）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('挂起期间提交：等待回执即时可见；解冻后免刷新落账', async ({ page }) => {
    const workbench = new Workbench(page)
    const indicator = page.locator(
      'sebas-workbench-composer [data-testid="submit-queued-indicator"]',
    )

    await resetState(page.request)
    const key = await createSession(page.request, { prompt: 'warmup' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(workbench.composerTextarea).toBeVisible()

    freezeCore()
    try {
      await workbench.composerTextarea.fill('while core frozen')
      await workbench.composerTextarea.press('Enter')

      // 本地回执与点击同帧级到达（2s 上限远小于 5s spec 预算）——挂起的
      // 后端不吞反馈。
      await expect(indicator).toBeVisible({ timeout: 2_000 })

      // 挂起 ≠ 断连：TCP 仍在，客户端不虚假宣告 ws-down，也不内显
      // NetworkError；草稿保留（回执未到不丢字）。
      await expect(page.locator('[data-testid="ws-down-banner"]')).toHaveCount(0)
      await expect(
        page.locator('sebas-workbench-composer [data-testid="composer-error"]'),
      ).toHaveCount(0)
      await expect(workbench.composerTextarea).toHaveValue('while core frozen')
    } finally {
      thawCore()
    }

    // 解冻后免刷新闭环：回执到达指示退场，回合照常完成并追加进转录。
    await expect(indicator).toBeHidden({ timeout: 20_000 })
    await waitStatus(page.request, key, ['done'])
    // 时间线序 = warmup 回合（prompt + 回复）+ 挂起回合（prompt + 回复），
    // 恰 4 块；尾两块是挂起回合的落账。
    const blocks = page.locator('sebas-transcript-view .turn-block')
    await expect(blocks).toHaveCount(4)
    await expect(blocks.nth(-2)).toContainText('while core frozen')
    await expect(blocks.last()).toContainText('hello world')
    // composer 复位：提交文本已随回执清空，回到可继续输入状态。
    await expect(workbench.composerTextarea).toHaveValue('')

    expect(collector.clean()).toEqual([])
  })
})
