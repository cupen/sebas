/**
 * Journey 3.5 — error semantics (spec: 错误呈现).
 *
 * 功能：agent 对话覆盖 / 子功能：错误诚实呈现
 *
 * "refuse": the stub ends the turn with a refusal — a NON-terminal error.
 * The session must survive it honestly: no fake Done, the row stays, and
 * the next message still completes. "crash": the stub emits one last text
 * frame and the child process dies — the mapping is torn down and the UI
 * must present that death (session gone / not-found), never a fabricated
 * success. Both trigger through the follow-up composer on an already-open
 * detail page (live WS), the realistic operator flow.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
  listSessions,
  FocusedSession,
  Transcript,
  waitStatus,
} from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Open an idle session's detail page (live WS) and return the key. */
  async function openIdle(page: import('@playwright/test').Page) {
    const detail = new FocusedSession(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    return { key, detail }
  }

  test.describe('错误诚实呈现', () => {
    test('refuse — non-terminal: session survives, next message works', async ({ page }) => {
      const { key, detail } = await openIdle(page)

      // On an already-DONE session a refusal is a NON-terminal error: it is
      // surfaced on the feishu card path (not the webui turn_log), so the
      // honest webui-observable contract is that the session is NOT torn down
      // and the next message still completes a full round-trip. We must not
      // fabricate a failure text that the webui transcript never carries.
      await detail.sendFollowUp('refuse')

      // The session mapping survives the refusal (not removed / not failed).
      const afterRefuse = await getSession(page.request, key)
      expect(afterRefuse.detail).not.toBeNull()
      expect(afterRefuse.detail!.status_slug).not.toBe('failed')

      // The next message still goes through a full round-trip.
      await detail.sendFollowUp('hello')
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible({
        timeout: 20_000,
      })
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })

    test('crash — honest death: mapping torn down, row gone, open view retains transcript', async ({ page }) => {
      const { key, detail } = await openIdle(page)

      await detail.sendFollowUp('boom crash')

      // The child dies mid-turn; the session mapping is torn down. The API
      // truth is the honest-death contract: the row leaves the live list and
      // the detail answers not-found — no fabricated success anywhere.
      await expect
        .poll(
          async () => (await listSessions(page.request)).some((r) => r.encoded_key === key),
          { timeout: 20_000, intervals: [200] },
        )
        .toBe(false)
      expect((await getSession(page.request, key)).status).toBe(404)

      // conversation-incremental-sync：已打开的视图对增量续拉失败（会话已
      // 拆除 → 404）保留已渲染的 transcript 与游标自愈——「Session
      // unavailable」空态保留给首拉失败，不再出现在中途死亡的会话上
      // （前端单测 dashboard.test.ts 同款钉子）。呈现上没有假成功：transcript
      // 停在死前最后一帧（"boom"），不出现任何完成态文案。
      const transcript = new Transcript(page)
      await expect(transcript.turnWith('boom').first()).toBeVisible()
      await expect(detail.unavailableNote).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('spawn 失败内显', () => {
    /**
     * fail-fast-on-startup-errors（openspec/changes/fail-fast-on-startup-errors
     * 任务 4.1）：为一个未知 acp kind（`acp:missing-agent`）创建会话 → spawn
     * 必然失败。webui 合同：spawn failure SHALL 立即 inline 到 transcript
     * （错误事件带原因），会话状态标记 spawn-failed 且保持可见——Removed
     * 不再是失败的首现路径，会话也不得凭空消失。
     */
    test('spawn failure inline: error event in transcript, session stays as spawn-failed', async ({
      page,
      request,
    }) => {
      const detail = new FocusedSession(page)

      // 触发器：未知 agent kind → acp 驱动拒绝 spawn。
      const key = await createSession(request, {
        prompt: 'spawnfail probe',
        agent: 'missing-agent',
      })

      // API 真源：会话收敛到 spawn-failed（Failed），带 error 元素的
      // transcript（原因内显），且 detail 仍然 200（未被拆除）。
      const converged = await waitStatus(request, key, ['failed'], 60_000)
      expect(converged.status_slug).toBe('failed')
      const errorBlocks = converged.entries.filter((b) => b.element_type === 'error')
      expect(errorBlocks.length).toBeGreaterThanOrEqual(1)
      expect(errorBlocks[0].content).toContain('spawn failed')
      expect(errorBlocks[0].content).toContain('missing-agent')

      // 会话未从列表消失（Removed 不是首现路径）。
      const rows = await listSessions(request)
      expect(rows.some((r) => r.encoded_key === key)).toBe(true)

      // 浏览器呈现：工作台聚焦渲染（唯一对话面），transcript 内显错误
      // 气泡，状态徽章为 failed。
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await expect(detail.unavailableNote).toHaveCount(0)
      const transcript = new Transcript(page)
      await expect(transcript.turnWith('spawn failed').first()).toBeVisible({
        timeout: 10_000,
      })
      await expect(transcript.turnWith('missing-agent').first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'failed')

      expect(collector.clean()).toEqual([])
    })
  })
})
