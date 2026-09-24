/**
 * Journey — 并行审批卡片各自独立（spec: permission-flow「Parallel tool calls each
 * get their own request id」，add-acp-stream-approval-journeys 3.2）。
 *
 * 功能：审批卡片旅程 / 子功能：并行工具调用各自独立 request_id
 *
 * 驱动载体：ACP 桩（fake-claude 的 `parallel` 触发词，走既有 `claude` 档，零装配
 * 改动）。桩在**一个 ACP 回合**里发两个 tool_use（Bash `echo first` +
 * Read `/tmp/fake-parallel.txt`）并连发两个 hook_callback；每个请求带自己的
 * request_id，卡片据此各自独立渲染（独立卡片、独立工具名/参数，不合并）。
 *
 * 与 test-model-scenarios.spec.ts 的「test/tools-parallel」是**有意双载体**：
 * 那条走 native 内核 × router test 模型（事件生产者是 router 内答），本用例走
 * ACP 子进程 × hook 泊车（生产者是桩）。浏览器呈现层虽同，从生产者到 UI 的链路
 * 不同，任一通路的回归都不能被另一条发现（分工见
 * openspec/changes/add-acp-stream-approval-journeys/specs/testsuite-webui-browser
 * 增量的分工段）。两侧 SHALL NOT 互相替代或豁免。
 *
 * ⚠ 实测边界（design D3 的「重叠泊车」假设在 SDK 边界不成立）：cc-agent-sdk
 * 0.1.7 在 hook 回调分发处跨 await 持回调表锁，第二条 hook_callback 的回调要等
 * 第一条决定返回才开始——驱动侧一次只泊一条。由此两点实测事实：其一，**泊车次序
 * 竞速**（回调表锁不保证 FIFO，实测先是 Read、后是 Bash，进程级跑又见 Bash 先），
 * 故用例按 request_id / 工具名动态取卡，不写死顺序；其二，第二张卡常在首张决策后
 * **同一批 WS 帧内接替**，DOM 计数从 1 直接到 1 而不出现 0 窗口，故断言「换成另一张
 * request_id 的独立卡片」而非「计数归零」。桩侧的「连发」wire 契约由进程级 journey
 * （parallel_scenario_parks_each_approval_and_settles_the_turn）的 journal 断言钉死。
 *
 * 断言口径：「两张各自独立（各自 request_id / 工具名 / 参数，不合并）、逐一决策、
 * 回合推进、两工具结果呈现」，不假设两卡同时在 DOM，也不假设任一次序。
 *
 * retries: 0（task 3.2 纪律，对齐 conversation-streaming）：实现缺陷不得被 retry
 * 掩盖。零固定 sleep：全部走 expect / expect.poll 自动等待。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  getSessionApprovals,
  ReviewCards,
  waitStatus,
} from './helpers/index'

test.describe('审批卡片旅程', () => {
  // design D5/D6：本 spec 不接受 retry 兜底——首跑即须通过。
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('并行工具调用：卡片各自独立（ACP 桩）', () => {
    test('parallel 触发词：两个 tool_use 各自一张卡（独立 request_id / 工具名 / 参数），逐一决策后回合推进、两工具结果呈现', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      // 起点纪律（对齐 permission.spec）：先把会话跑到 idle、打开深链让 WS 在场，
      // 再从 composer 提交 `parallel`——卡面经 WS 推来的 `permission.requested`
      // 才渲染得出来。
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      await detail.sendFollowUp('parallel')

      // 第一张卡：自己的 request_id、自己的工具名与参数；不含另一个工具的痕迹
      // （「独立卡片，未合并」）。**不假设泊车次序**——两条 hook_callback 的回调
      // 在 SDK 边界竞速（回调表锁不保证 FIFO），Bash / Read 谁先到都可能；契约是
      // 「两张各自独立」，不是「Bash 一定先出现」。
      await expect(cards.all().first()).toBeVisible({ timeout: 30_000 })
      const firstId = await cards.card().getAttribute('data-request-id')
      expect(firstId).toBeTruthy()
      const firstTool = (await cards.card().locator('.head .tool').innerText()).trim()
      expect(['Bash', 'Read'], 'the card names one of the two scripted tools').toContain(firstTool)
      const secondTool = firstTool === 'Bash' ? 'Read' : 'Bash'
      const firstArgs = firstTool === 'Bash' ? 'echo first' : '/tmp/fake-parallel.txt'
      const secondArgs = firstTool === 'Bash' ? '/tmp/fake-parallel.txt' : 'echo first'
      await expect(cards.card().locator('pre.args')).toContainText(firstArgs)
      await expect(cards.card().locator('pre.args')).not.toContainText(secondArgs)

      await cards.allowOnce().click()

      // 第一张卡决策后由第二张卡接替。**不钉「计数归零」**：驱动侧一次只泊一条
      // （SDK 回调表锁），第二张卡的 permission.requested 可能在第一张卡的
      // permission.resolved 同一批 WS 帧里到达，DOM 从「1 张」直接换到「1 张」
      // 而不经过 0——零窗口是竞速的，不是契约。契约是「换成另一张独立卡片」：
      // 等首张消失、且可见卡片全部换成了别的 request_id。
      await expect
        .poll(
          async () => {
            const ids = await cards
              .all()
              .evaluateAll((els) => els.map((el) => el.getAttribute('data-request-id') ?? ''))
            return ids.length > 0 && ids.every((id) => id !== firstId)
          },
          { timeout: 30_000 },
        )
        .toBe(true)

      // 第二张卡：另一个工具、另一个 request_id（独立泊车与决策）。
      const secondId = await cards.card().getAttribute('data-request-id')
      expect(secondId).toBeTruthy()
      expect(secondId).not.toBe(firstId)
      await expect(cards.card().locator('.head .tool')).toHaveText(secondTool)
      await expect(cards.card().locator('pre.args')).toContainText(secondArgs)
      await expect(cards.card().locator('pre.args')).not.toContainText(firstArgs)

      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 20_000 })

      // 两个决定后回合推进到终态，两条工具结果如实呈现（工具结果住在 process
      // 折叠里——先展开再断言）。
      await detail.expectStatus('done', 30_000)
      await detail.expectFoldedText('Bash ok')
      await detail.expectFoldedText('Read ok')

      // API face：泊车读模型已排空（两条请求都已决策）。
      const approvals = await getSessionApprovals(page.request, key)
      expect(approvals.status).toBe(200)
      expect(approvals.approvals).toEqual([])

      expect(collector.clean()).toEqual([])
    })
  })
})
