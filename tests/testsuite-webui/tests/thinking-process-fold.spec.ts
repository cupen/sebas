/**
 * Journey — thinking 过程折叠真链路呈现（spec: agent-workbench「过程折叠」，
 * add-acp-stream-approval-journeys 3.3）。
 *
 * 功能：agent 对话覆盖 / 子功能：过程折叠（thinking 呈现）
 *
 * 装配：沙箱的 `claude-thinking` agent（`--scenario thinking --slow-ms 800`，
 * tasks.py 按 claude-empty/claude-stream 惯例新增）——桩在一个回合里**交替**发
 * thinking 与正文段：
 *   thinking "hmm" → 正文 "thought out loud" → thinking "hmm again" → 正文 "and the answer"
 * 于是 splitAgentRuns 得到 4 个 run：fold → text → fold → text。
 *
 * 断言口径（契约层，不钉像素/滚动）：两段 thinking 以过程折叠呈现且**默认收起**；
 * 两段正文独立于折叠之外、按序可见；结算后展开折叠，thinking 内容不丢失、顺序保持。
 * 渲染细节（截断、懒渲染）由前端单测 transcript-view.test.ts 覆盖，此处只钉真链路
 * 的 DOM 契约。
 *
 * 与 native 侧 test/thinking（extend-test-model-scenarios 进程级 journey）是**有意
 * 双载体**：事件生产者不同（ACP 子进程 vs router 内答），互不替代。
 *
 * retries: 0：实现缺陷不得被 retry 掩盖。零固定 sleep。
 */
import { expect, test } from '@playwright/test'
import { createSession, ErrorCollector, FocusedSession } from './helpers/index'

test.describe('agent 对话覆盖', () => {
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('过程折叠：thinking 呈现', () => {
    test('thinking/正文交替回合：thinking 进默认收起的过程折叠、正文独立按序，结算后顺序与内容不丢失', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const detail = new FocusedSession(page)

      // D4 起点纪律：0-turn placeholder（不预跑回合）→ 打开深链 → composer 提交，
      // 观测起点严格早于回合开始。
      const key = await createSession(page.request, { prompt: null, agent: 'claude-thinking' })
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })

      // agent 已把剧本钉在 argv（`--scenario thinking`），触发词无关——任意文本
      // 都跑同一个 thinking/正文交替回合。
      await detail.sendFollowUp('think please')
      await detail.expectStatus('done', 30_000)

      const assistantTurn = page
        .locator('sebas-dashboard sebas-transcript-view .turn-block.is-assistant')
        .first()
      await expect(assistantTurn).toBeVisible()

      // 1) 两段 thinking 各成一个过程折叠，且**默认收起**（折叠体懒渲染，不在 DOM）。
      const foldLinks = assistantTurn.locator('div.process-fold button.fold-link')
      await expect(foldLinks).toHaveCount(2, { timeout: 15_000 })
      for (let i = 0; i < 2; i += 1) {
        await expect(foldLinks.nth(i)).toHaveAttribute('aria-expanded', 'false')
        // 折叠行的可读标签点名这是 thinking（不是正文；也不是工具）。
        await expect(foldLinks.nth(i)).toContainText('thinking')
      }
      await expect(assistantTurn.locator('div.process-fold .fold-body')).toHaveCount(0)

      // 2) 正文独立于折叠之外，run 序 fold → text → fold → text（顺序保持）。
      const runKinds = await assistantTurn.locator('.flow').evaluate((flow) =>
        Array.from(flow.children)
          .filter((el) => !el.classList.contains('meta'))
          .map((el) =>
            el.classList.contains('process-fold')
              ? 'fold'
              : el.classList.contains('body')
                ? 'text'
                : `other:${el.className}`,
          ),
      )
      expect(runKinds).toEqual(['fold', 'text', 'fold', 'text'])

      // 3) 两段正文在折叠之外按序可见（不需要展开任何折叠）。
      const textBodies = assistantTurn.locator('.body:not(.fold-body):not(.item-body)')
      await expect(textBodies).toHaveCount(2)
      expect((await textBodies.allInnerTexts()).map((t) => t.trim())).toEqual([
        'thought out loud',
        'and the answer',
      ])

      // 4) 结算后展开折叠：thinking 内容不丢失（两段都在，且顺序保持）。
      let foldText = ''
      await expect
        .poll(
          async () => {
            await detail.expandAllFolds()
            foldText = (await assistantTurn.locator('div.process-fold .fold-body').allInnerTexts())
              .join(' | ')
              .trim()
            return foldText
          },
          { timeout: 20_000 },
        )
        .toContain('hmm again')
      expect(foldText).toContain('hmm')
      // 展开只影响折叠体，正文 run 序不变（fold → text → fold → text 仍在）。
      const runKindsAfterExpand = await assistantTurn.locator('.flow').evaluate((flow) =>
        Array.from(flow.children)
          .filter((el) => !el.classList.contains('meta'))
          .map((el) =>
            el.classList.contains('process-fold')
              ? 'fold'
              : el.classList.contains('body')
                ? 'text'
                : `other:${el.className}`,
          ),
      )
      expect(runKindsAfterExpand).toEqual(['fold', 'text', 'fold', 'text'])

      expect(collector.clean()).toEqual([])
    })
  })
})
