/**
 * Journey — 并行审批决策精确路由与显示序稳定（spec:
 * permission-flow「Parallel approvals render concurrently」MODIFIED，
 * fix-parallel-approval-routing）。
 *
 * 功能：审批卡片旅程 / 子功能：并行决策精确路由（乱序 + 刷新后）与显示序稳定
 *
 * Spec anchors（本 change 增量新增的场景）:
 * - decision routes to the clicked card（乱序决策不互换）
 * - decision after reload still routes correctly（刷新后再决策仍各归其位）
 * - pending card order is stable（同一待批集合刷新前后同一显示序）
 * - allowed entry remains after decision（允许/拒绝各带结果留在转写）
 *
 * 与 parallel-permissions.spec.ts 的分工：那条钉 round2 时代的「两张独立卡、
 * 逐一决策」（不假设两卡同时在 DOM）；本条钉 fix-parallel-approval-routing 之
 * 后的更强呈现契约——vendor D-C5 补丁（fix-webui-qa-round2，放开 hook 回调表
 * 锁）后两条 hook_callback 同时在飞、两卡**同时挂起**是常态（本 change GUI 手
 * 测 R1–R4 实证），卡片显示序已按 request_id 字典序收敛（与读模型枚举同键）。
 * 于是可以断言：同屏两卡、按内容寻址（工具名+参数 ↔ 卡自身按钮）、乱序决策、
 * 刷新重建后同一显示序、两工具结果不互换。若驱动侧退回串行泊车，「两卡同屏」
 * 的等待会超时即红——那是 spec 场景「同时挂起、可同时决策」前提本身的回归，
 * 不该被静默放宽。
 *
 * 决策顺序刻意与显示序相反（先决策 request_id 靠后的卡）：原始缺陷形态是
 * 「按待批队列序隐式配对」，乱序决策正是它的最小复现。
 *
 * retries: 0（对齐 parallel-permissions：实现缺陷不得被 retry 掩盖）。
 */
import { expect, test, type Locator, type Page } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  getSessionApprovals,
  ReviewCards,
  waitStatus,
  type PendingApproval,
} from './helpers/index'

test.describe('审批卡片旅程', () => {
  // design 纪律（对齐 parallel-permissions）：本 spec 不接受 retry 兜底。
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** 某张卡的 section 定位（按 data-request-id 精确寻址）。 */
  function cardById(page: Page, requestId: string): Locator {
    return page.locator(`sebas-review-cards section.review-card[data-request-id="${requestId}"]`)
  }

  /** 某张卡的 section 定位（按操作员可见的工具名内容寻址）。 */
  function cardByTool(page: Page, tool: string): Locator {
    return page
      .locator('sebas-review-cards section.review-card')
      .filter({ hasText: tool })
  }

  /** 卡片的 DOM 序（data-request-id 列表）。 */
  function domOrder(cards: ReviewCards): Promise<string[]> {
    return cards
      .all()
      .evaluateAll((els) => els.map((el) => el.getAttribute('data-request-id') ?? ''))
  }

  interface CardFace {
    id: string
    tool: string
    args: string
  }

  /** 全部卡片的（id, 工具名, 参数）三元组，按 DOM 序。 */
  function cardFaces(cards: ReviewCards): Promise<CardFace[]> {
    return cards.all().evaluateAll((els) =>
      els.map((el) => ({
        id: el.getAttribute('data-request-id') ?? '',
        tool: el.querySelector('.head .tool')?.textContent?.trim() ?? '',
        args: el.querySelector('pre.args')?.textContent ?? '',
      })),
    )
  }

  /** 内容 ↔ id 配对：每张卡的工具名与参数必须与读模型同 id 行逐字一致。 */
  async function expectCardsPairedWithReadModel(
    cards: ReviewCards,
    approvals: PendingApproval[],
  ): Promise<void> {
    const faces = await cardFaces(cards)
    expect(faces.map((f) => f.id)).toEqual(approvals.map((a) => a.request_id))
    for (const a of approvals) {
      const face = faces.find((f) => f.id === a.request_id)
      expect(face, `card for ${a.request_id} must be present`).toBeTruthy()
      expect(face!.tool, `tool on card ${a.request_id}`).toBe(a.tool_name)
      expect(face!.args, `args on card ${a.request_id}`).toContain(
        JSON.stringify(a.args, null, 2),
      )
    }
  }

  /**
   * 起点纪律（对齐 permission.spec / parallel-permissions）：idle 会话先到
   * done、深链打开让 WS 在场，再经 composer 提交 `parallel`；然后等两卡同屏。
   */
  async function openAndTriggerParallel(page: Page) {
    const detail = new FocusedSession(page)
    const cards = new ReviewCards(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    await detail.sendFollowUp('parallel')
    await expect(cards.all()).toHaveCount(2, { timeout: 30_000 })
    return { detail, cards, key }
  }

  /**
   * 乱序决策：先点**显示序靠后**那张卡（request_id 字典序较大者）的 Deny，
   * 再点靠前那张卡的 Allow once——每个决定携带该卡自己的 request_id。
   */
  async function decideReversedOrder(page: Page, cards: ReviewCards): Promise<void> {
    const faces = await cardFaces(cards)
    const sorted = [...faces].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    const later = sorted[sorted.length - 1]!
    const earlier = sorted[0]!
    // 按内容寻址（操作员看到什么点什么）；配对断言保证内容 ↔ id 一致。
    const denyTool = (await cardByTool(page, 'Read').locator('.head .tool').innerText()).trim()
    const allowTool = (await cardByTool(page, 'Bash').locator('.head .tool').innerText()).trim()
    expect(denyTool).toBe('Read')
    expect(allowTool).toBe('Bash')
    expect(later.tool).toBe('Read')
    expect(earlier.tool).toBe('Bash')
    await cardByTool(page, 'Read').locator('wa-button.deny').click()
    // 被决策的那张卡退场，另一张必须原样在场（不得连带消失或换内容）。
    await expect(cardById(page, later.id)).toHaveCount(0, { timeout: 15_000 })
    await expect(cardById(page, earlier.id)).toBeVisible()
    await cardByTool(page, 'Bash').locator('wa-button.allow-once').click()
    await expect(cards.all()).toHaveCount(0, { timeout: 20_000 })
  }

  /** 决策后的落点断言：Bash 被执行、Read 被拒、无互换、读模型排空。 */
  async function expectResultsFollowTheirOwnCards(
    detail: FocusedSession,
    request: import('@playwright/test').APIRequestContext,
    key: string,
  ): Promise<void> {
    await detail.expectStatus('done', 30_000)
    // 各归其位：Bash 被允许执行（Bash ok），Read 被拒（denied by fake）。
    // 若发生互换（原始缺陷：决定路由到另一张卡），这里会看到 Read ok 而非
    // Bash ok——两条断言从正反两面钉死。
    await detail.expectFoldedText('Bash ok')
    await detail.expectFoldedText('denied by fake')
    await expect
      .poll(
        async () => {
          await detail.expandAllFolds()
          return detail.turnWith('Read ok').count()
        },
        { timeout: 10_000 },
      )
      .toBe(0)
    // API face：泊车读模型已排空（两条请求都已决策）。
    const approvals = await getSessionApprovals(request, key)
    expect(approvals.status).toBe(200)
    expect(approvals.approvals).toEqual([])
  }

  test.describe('并行决策精确路由（fix-parallel-approval-routing）', () => {
    test('乱序决策：先 Deny 后 Allow，各卡决定精确路由到各自的工具（spec: decision routes to the clicked card）', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const { detail, cards, key } = await openAndTriggerParallel(page)

      // 显示序与读模型枚举序同键（request_id 字典序）；内容 ↔ id 配对。
      const api = await getSessionApprovals(page.request, key)
      expect(api.status).toBe(200)
      expect(await domOrder(cards)).toEqual(api.approvals.map((a) => a.request_id))
      await expectCardsPairedWithReadModel(cards, api.approvals)

      await decideReversedOrder(page, cards)
      await expectResultsFollowTheirOwnCards(detail, page.request, key)

      expect(collector.clean()).toEqual([])
    })

    test('刷新后决策仍精确路由 + 待批卡显示序稳定（spec: decision after reload still routes correctly / pending card order is stable）', async ({
      page,
    }) => {
      test.setTimeout(120_000)
      const { detail, cards, key } = await openAndTriggerParallel(page)

      const orderBefore = await domOrder(cards)
      expect(orderBefore).toHaveLength(2)

      // 挂起期间刷新：审批面从读模型重建（WS 推送不在场）。
      await page.reload()
      await expect(detail.host).toBeVisible()
      await expect(cards.all()).toHaveCount(2, { timeout: 30_000 })

      // 同一待批集合，刷新前后同一显示序（不得无因翻转），且仍与读模型同键。
      expect(await domOrder(cards)).toEqual(orderBefore)
      const api = await getSessionApprovals(page.request, key)
      expect(api.approvals.map((a) => a.request_id)).toEqual(orderBefore)
      await expectCardsPairedWithReadModel(cards, api.approvals)

      // 刷新重建后的乱序决策仍各归其位。
      await decideReversedOrder(page, cards)
      await expectResultsFollowTheirOwnCards(detail, page.request, key)

      expect(collector.clean()).toEqual([])
    })
  })
})
