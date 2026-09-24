/**
 * Journey — 对话流实时上屏（spec: 流式分批渲染，add-conversation-streaming-journey）。
 *
 * 功能：agent 对话覆盖 / 子功能：流式分批
 *
 * 与 `streaming.spec.ts` 的分工（design D3）：那份以**服务端 API** 为主
 * oracle，只断言「一回合 = 一气泡」的聚合口径（多 chunk 合并）；本用例以
 * **浏览器 DOM** 为主 oracle，断言「增量正文已上屏」发生在会话进入终态
 * **之前**——只有中途断言能区分「实时渲染」与「结束整块渲染」（design D1）。
 *
 * design D4 起点纪律：先建 0-turn placeholder 会话（不预跑回合）→ 打开其
 * 深链 → 在 composer 输入并提交。观测起点严格早于回合开始，避免「回合在
 * 页面加载前已完成」让中途断言退化为空断言。
 *
 * 数据源：沙箱的 `claude-stream` agent（`--delta-gap-ms 500`，tasks.py 装配）。
 * 以流式触发词 `drip` 提交——桩按 500ms 间隔逐段发出正文 delta（帧间静默
 * < driver 的 1.5s 挂起探测预算），3 段正文（drip0/1/2）的「回合进行中」
 * 窗口约 1s。
 *
 * design D5：本 spec 局部 `retries: 0`——实现缺陷不得被 retry 掩盖成 flaky。
 * 零固定 sleep：所有等待走 `expect.poll`（50ms 间隔）与 Playwright 自动等待。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  getSession,
  type StatusSlug,
} from './helpers/index'

/** 会话相位七词（webui models.rs）：非终态 = 回合仍在飞。 */
const RUNNING: StatusSlug[] = ['starting', 'queued', 'working', 'waiting']
/** 终态 = 回合已结算。 */
const TERMINAL: StatusSlug[] = ['done', 'failed', 'dormant']

test.describe('agent 对话覆盖', () => {
  // design D5：本 spec 不接受 retry 兜底——首跑即须通过。
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('流式分批', () => {
    test('incremental reply text reaches the DOM while the turn is still running', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const detail = new FocusedSession(page)
      // 既有选择器（task 2.4）：不新增测试专用 DOM 钩子。
      const assistantTurns = page.locator(
        'sebas-dashboard sebas-transcript-view .turn-block.is-assistant',
      )

      // D4：0-turn placeholder（prompt 缺省 = null，不 spawn 子进程），绑定
      // 专门配置了 `--delta-gap-ms 500` 的 claude-stream agent。
      const key = await createSession(page.request, { prompt: null, agent: 'claude-stream' })

      // 打开深链——观测起点在此，严格早于回合开始。
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })

      // 提交前确定性事实：该回合尚无任何助手正文（观测起点在回合之前）。
      await expect(assistantTurns).toHaveCount(0)

      // 提交即开轮；之后立刻进入 50ms 粒度观测。`drip` 是流式触发词：桩按
      // 500ms 间隔逐段发出正文 delta（drip0/1/2）。
      await detail.sendFollowUp('drip')

      // 主断言：会话**仍 running** 时，focused conversation 的 DOM 已出现
      // 增量正文。服务端 `status_slug` 是真 oracle——若 DOM 只在中止时刻整块
      // 上屏，采到时 status 已是终态，永远命中不了 RUNNING 分支。
      const startedAt = Date.now()
      let earlyText = ''
      let earlySlug: StatusSlug | '' = ''
      // （add-acp-stream-approval-journeys 3.4）中途采样时末尾正文的渲染形态：
      // true = live-tail 纯文本（div.body.text-live）。
      let earlyLiveTail = false
      // 同一时刻 live-tail 的正文本身（结算后要与它比拼接连续性）。
      let earlyBodyText = ''
      await expect
        .poll(
          async () => {
            const { detail: api } = await getSession(page.request, key)
            const slug = api?.status_slug
            const texts = (await assistantTurns.allTextContents())
              .map((t) => t.trim())
              .filter((t) => t.length > 0)
            if (texts.length > 0 && slug !== undefined && RUNNING.includes(slug)) {
              earlyText = texts.join(' ')
              earlySlug = slug
              // 与文本同一时刻采样：回合进行中，末尾正文以 live-tail 纯文本形态
              // 上屏（3.4 的「中途形态」半边），并留下此刻的正文用于拼接对照。
              const liveBody = assistantTurns.first().locator('.body.text-live')
              earlyLiveTail = (await liveBody.count()) > 0
              if (earlyLiveTail) earlyBodyText = (await liveBody.first().textContent()) ?? ''
              return 'incremental-while-running'
            }
            if (slug !== undefined && TERMINAL.includes(slug)) return `terminal:${slug}`
            return 'waiting'
          },
          { timeout: 20_000, intervals: [50] },
        )
        .toBe('incremental-while-running')
      const incrementalAt = Date.now()

      // 随后收敛 Done（终态；旧 streaming.spec 的 API oracle 只覆盖到这一步）。
      await detail.expectStatus('done')
      const doneAt = Date.now()

      // 硬化 oracle（review #3，排除 TOCTOU 假绿）：中途采到的文本必须是最终
      // 回复的**更短真前缀**。只断言"终态前 DOM 非空"在"回合结束时整块渲染 +
      // 服务端状态短暂滞后"的竞态下可能误判——那时 DOM 已有全量正文而
      // status_slug 仍读到 working。要求 early 是 final 的真前缀且更短，
      // 才能证明"部分正文在回合进行中已上屏"。
      const norm = (s: string) => s.replace(/\s+/g, ' ').trim()
      const finalText = norm((await assistantTurns.first().textContent()) ?? '')
      const earlyNorm = norm(earlyText)
      expect(earlyNorm.length).toBeGreaterThan(0)
      expect(finalText.startsWith(earlyNorm)).toBe(true)
      expect(earlyNorm.length).toBeLessThan(finalText.length)

      // ── add-acp-stream-approval-journeys 3.4：结算时刻的形态切换 ──
      // 中途（与上面同一采样时刻）：增量正文是 live-tail 纯文本形态。既有断言
      // 只钉「回合进行中文本已上屏」，这里补钉它当时长什么样。
      expect(
        earlyLiveTail,
        'incremental text must render as the live-tail plain-text form (div.body.text-live) while running',
      ).toBe(true)

      // 结算后：同一正文切回 markdown 渲染——live-tail 形态从 DOM 消失，正文落进
      // markdown 块（renderMarkdown 产出 <p>）。
      await expect(assistantTurns.locator('.body.text-live')).toHaveCount(0)
      const settledBody = assistantTurns.first().locator('.body').last()
      await expect(settledBody.locator('p').first()).toBeVisible()

      // 拼接一致、无重复条目：结算正文恰为三段 drip 的拼接，且以流式期间采到的
      // live-tail 正文为前缀（中途看到的那部分一字不改地接上后两段），每段在整条
      // 转写里只出现一次（不重复、不丢段）。
      const settledText = norm((await settledBody.textContent()) ?? '')
      expect(settledText).toBe('drip0 drip1 drip2')
      const earlyBodyNorm = norm(earlyBodyText)
      expect(earlyBodyNorm.length).toBeGreaterThan(0)
      expect(settledText.startsWith(earlyBodyNorm)).toBe(true)

      // 结构上的「无重复条目」：一回合只有一个 assistant 气泡、一个正文块，
      // 且三段 drip 在转写里各只出现一次（不重复、不丢段）。取文本用**穿透
      // shadow root 的 locator**（`allTextContents`），不能取宿主元素的
      // `textContent`/`innerText`——`sebas-transcript-view` 是 shadow DOM 宿主，
      // 宿主的 textContent 不含影子树内容，会得到假阴性的 0。
      await expect(assistantTurns).toHaveCount(1)
      await expect(
        assistantTurns.first().locator('.body:not(.fold-body):not(.item-body)'),
      ).toHaveCount(1)
      const transcriptText = norm(
        (
          await page
            .locator('sebas-dashboard sebas-transcript-view .turn-block')
            .allTextContents()
        ).join(' '),
      )
      for (const chunk of ['drip0', 'drip1', 'drip2']) {
        expect(
          transcriptText.match(new RegExp(chunk, 'g'))?.length ?? 0,
          `${chunk} must appear exactly once in the settled transcript`,
        ).toBe(1)
      }

      // 时间线证据（供人工复核实时性；不做会抖动的硬阈值）：文本何时上屏
      // vs 何时 done。上屏必须显著早于 done 才叫「实时」。
      test.info().annotations.push({
        type: 'timeline',
        description:
          `incremental text first seen in DOM at +${incrementalAt - startedAt}ms ` +
          `(status=${earlySlug}, text=${JSON.stringify(earlyText)}); ` +
          `done at +${doneAt - startedAt}ms`,
      })

      // 上屏的确实是 stub 的回复正文，且回合结束后仍在。
      await expect(assistantTurns.first()).toContainText('drip')

      expect(collector.clean()).toEqual([])
    })
  })
})
