/**
 * Journey 3.3 — streamed turn as ONE conversation turn (spec: 流式分批渲染,
 * evolved by workbench-conversation-view 2.1).
 *
 * 功能：agent 对话覆盖 / 子功能：流式分批
 *
 * 职责分工（add-conversation-streaming-journey 4.3，design D3）：本用例只承担
 * 「**聚合口径**」——多 chunk（5 段文本 delta）回合结束后在 DOM 合成 ONE
 * assistant 气泡、内容按到达序拼接、会话收敛 Done；主 oracle 是**服务端 API**
 * 的 5 个 chunk 条目，`working` 瞬态只作 annotation（调度抖动的 CI 箱可能把
 * 整个回合塞进一个 poll 间隙，硬断言会假红）。**实时性**（回合进行中 DOM 已
 * 增量上屏）由新 spec `conversation-streaming.spec.ts` 以浏览器 DOM 硬断言
 * 承担（placeholder + composer 起点的 50ms 轮询、`retries: 0`）——两处口径
 * 不重复：本文件管「一回合=一气泡」，新文件管「回合进行中即上屏」。
 *
 * The "stream" trigger makes fake-claude emit 5 text chunks, pause 800ms,
 * then finish. We spawn with the trigger and open the page immediately.
 * The conversation view renders the whole streamed turn as ONE assistant
 * bubble — the chunks concatenate in arrival order inside the turn (2.1:
 * one agent turn = one bubble, never a series of per-chunk bubbles) — and
 * the status converges to Done.
 *
 * The transient `working` state is timing-dependent (a scheduled CI box can
 * observe the whole turn inside one poll gap), so we record it as an
 * annotation rather than a hard assert — the D4 discipline against
 * order/timing-dependent flakes.
 */
import { expect, test } from '@playwright/test'
import { createSession, ErrorCollector, getSession, FocusedSession } from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('流式分批', () => {
    test('5 streamed chunks land as ONE assistant turn bubble and converge to done', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const detail = new FocusedSession(page)

      // Spawn with the stream trigger and open the page at once.
      const key = await createSession(page.request, { prompt: 'stream' })
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // Sample the server transcript at 100ms (no fixed sleep) until all 5
      // chunk entries are present. Zero fixed sleeps anywhere.
      let sawTransientRunning = false
      await expect
        .poll(async () => {
          const { detail: api } = await getSession(page.request, key)
          if (!api) return -1
          if (api.status_slug === 'working') sawTransientRunning = true
          return api.entries.filter((b) => /^chunk\d/.test(b.content)).length
        }, { timeout: 20_000, intervals: [100] })
        .toBe(5)
      if (sawTransientRunning) {
        test.info().annotations.push({
          type: 'spuriousInfo',
          description: 'observed transient working state during the stream turn',
        })
      }

      // 2.1: the turn renders as ONE assistant bubble carrying every chunk
      // in order — not five per-chunk bubbles.
      const assistantTurns = page
        .locator('sebas-dashboard sebas-transcript-view .turn-block.is-assistant')
      await expect(assistantTurns).toHaveCount(1, { timeout: 15_000 })
      const turnText = (await assistantTurns.first().textContent()) ?? ''
      for (let i = 0; i < 5; i++) {
        expect(turnText).toContain(`chunk${i}`)
      }
      // The chunks stay ordered inside the concatenated turn.
      expect(turnText.indexOf('chunk0')).toBeLessThan(turnText.indexOf('chunk4'))

      // The operator's submission is its own turn beside the agent's (2.3).
      await expect(detail.userTurn('stream')).toBeVisible()

      // Final convergence: Done, honestly.
      await detail.expectStatus('done')

      expect(collector.clean()).toEqual([])
    })
  })
})
