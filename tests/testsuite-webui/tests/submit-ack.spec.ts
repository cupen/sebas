/**
 * Journey — 提交反馈时限（close-acceptance-blind-spots 4.3，spec
 * live-turn-stream「Submission acknowledgment is bounded」）。
 *
 * 取舍说明：Playwright 拓扑的 fake-claude 参数固定为 `--slow-ms 800`
 * （tasks.py 装配的共享沙箱），`--scenario slow --delay-ms N` 无法按用例
 * 注入——「慢后端」路径改用**浏览器级** `page.route` 对 composer 的
 * POST /api/sessions/:key/message 注入 6s 延迟等价复现（比 5s 预算更长的
 * 确定性慢确认），不动沙箱 harness。`page.request` 走 APIRequestContext、
 * 不经 route 拦截，注入只命中页面自身发出的提交请求。
 *
 * - 提交一发出，提交面立即出现本地排队指示（乐观呈现，与点击同帧，远快于
 *   5s 预算——慢后端不吞反馈）；
 * - 后端确认放行后指示退场，回合照常完成（乐观呈现与后端对齐）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  resetState,
  waitStatus,
  Workbench,
} from './helpers/index'

/** 注入的确认延迟：> 5s 预算，确定性制造「核心通道确认迟到」形态。 */
const ACK_DELAY_MS = 6_000

test.describe('提交反馈时限（5s 内可见反馈）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('a submission shows the local queued indication immediately, slow ack included', async ({
    page,
  }) => {
    const workbench = new Workbench(page)

    await resetState(page.request)
    const { id: projectId } = await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'first', projectId })
    await waitStatus(page.request, key, ['done'])

    // 浏览器级慢确认注入：composer 的 POST 6s 后才放行。
    await page.route('**/api/sessions/*/message', async (route) => {
      await new Promise((r) => setTimeout(r, ACK_DELAY_MS))
      await route.continue()
    })

    await page.goto(`/sessions/${key}`)
    await workbench.composerTextarea.fill('slow ack please')
    await workbench.submitControl.click()

    // 提交面 5 秒内出现可见排队指示——实际与点击同帧（本地乐观呈现，
    // 不等后端往返）。2s 上限远小于 spec 预算，断言的是「即时」而非「碰巧快」。
    const indicator = page.locator(
      'sebas-workbench-composer [data-testid="submit-queued-indicator"]',
    )
    await expect(indicator).toBeVisible({ timeout: 2_000 })

    // 确认放行后指示退场（慢确认窗口内持续可见，确认后交给转录回执对齐），
    // 回合正常完成。
    await expect(indicator).toBeHidden({ timeout: 15_000 })
    await waitStatus(page.request, key, ['done'])
    expect(collector.clean()).toEqual([])
  })
})
