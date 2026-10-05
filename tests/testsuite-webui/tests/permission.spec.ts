/**
 * Journey 3.4 — permission review cards (spec: 拒绝路径 / 单次允许路径 /
 * 会话级允许).
 *
 * 功能：审批卡片旅程 / 子功能：拒绝路径、单次允许路径、会话级允许
 *
 * The "perm" trigger makes fake-claude call Bash("rm -rf /") through the
 * hook_callback gate. The gated request reaches the browser as a WS
 * permission.requested frame and renders as a review card — so the card
 * can only appear if the session-detail page's WebSocket subscription is
 * ALREADY live when the gate fires. Each scenario therefore navigates to
 * an idle session's detail page FIRST, then triggers "perm" through the
 * follow-up composer — the realistic operator flow and the only one that
 * deterministically captures the frame.
 */
import { expect, test } from '@playwright/test'
import { createSession, ErrorCollector, ReviewCards, FocusedSession, waitStatus } from './helpers/index'

test.describe('审批卡片旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Navigate to an idle session with a live WS, then trigger "perm". */
  async function openAndTriggerPerm(page: import('@playwright/test').Page) {
    const detail = new FocusedSession(page)
    const cards = new ReviewCards(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    await detail.sendFollowUp('perm')
    return { detail, cards }
  }

  test.describe('拒绝路径', () => {
    test('deny path — refusal semantics, turn completes', async ({ page }) => {
      const { detail, cards } = await openAndTriggerPerm(page)

      // Card appears with the gated call's semantics: Bash + the command args.
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await expect(cards.card().locator('.head .tool')).toHaveText('Bash')
      await expect(cards.card().locator('pre.args')).toContainText('rm -rf /')

      await cards.deny().click()

      // Card resolved and removed; the transcript records the denial.
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      // 工具结果文本住在 process 折叠里（默认收起 + 展开体懒渲染 + 定稿会
      // 重分组）——轮询式展开到文本出现。
      await detail.expectFoldedText('denied by fake')
      await detail.expectStatus('done')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('单次允许路径', () => {
    test('allow-once path — allowed semantics, turn completes', async ({ page }) => {
      const { detail, cards } = await openAndTriggerPerm(page)

      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowOnce().click()

      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await detail.expectFoldedText('perm done')
      await detail.expectStatus('done')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('会话级允许', () => {
    test('allow-session path — session switches to auto mode, follow-up is no longer gated', async ({ page }) => {
      // 语义更新（6bbbc25，permission-mode-auto-gate）：「本会话不再询问」
      // = 放行当前请求 + 会话 mode 切 auto（与飞书卡面同一组合）；旧
      // allowlist/grant_all 退役。driver 层是 mode 的唯一定门控：auto 档的
      // hook_callback 被静默应答、零请求跨面——后续同会话的相同调用不再
      // 产生审批卡（本用例曾钉住的「WebUI 路径不持久化」产品缺口随
      // grant_all 一并退役）。
      const { detail, cards } = await openAndTriggerPerm(page)

      // First call: allow for session.
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowSession().click()
      await detail.expectFoldedText('perm done')
      await detail.expectStatus('done')

      // Second identical call in the SAME session: auto mode answers the gate
      // driver-side, so the turn completes with NO review card at all.
      await detail.sendFollowUp('perm')
      // 第二回合的折叠也是收起态——两处结果文本都可见才算两回合都落了。
      await expect
        .poll(
          async () => {
            await detail.expandAllFolds()
            return detail.turnWith('perm done').count()
          },
          { timeout: 20_000 },
        )
        .toBe(2)
      await detail.expectStatus('done', 20000)
      await expect(cards.all()).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })

  // fix-webui-qa-round6（permission-flow「每一轮工具环的审批请求都到达审批
  // 面」浏览器半边）：已决策过一轮的同会话，第二轮 `perm` 的审批卡必须**原
  // 地再次渲染**（WS 推送路径、无刷新），泊车期间 rail 当前行亮「等待」徽标
  // ——回归钉：第二轮请求丢失（读模型空、不出卡、回合永久挂起）在这里当场
  // 爆。进程级与 API 读模型半边在 tests/testsuite_e2e_test.rs 的
  // second_turn_permission_request_still_parks_and_is_decidable。
  test.describe('多轮审批不丢', () => {
    test('second perm turn renders its approval card again, rail shows the waiting badge', async ({
      page,
    }) => {
      const { detail, cards } = await openAndTriggerPerm(page)

      // 第一轮：出卡 → allow once → 落定（既有合同）。
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await detail.expectFoldedText('perm done')
      await detail.expectStatus('done')

      // 第二轮（同一会话、同一页面、无刷新）：审批卡必须再次出现。
      await detail.sendFollowUp('perm')
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await expect(cards.card().locator('.head .tool')).toHaveText('Bash')

      // 泊车可观察面的另一半：rail 当前行亮 waiting（status_slug=waiting +
      // 「等待」徽标）——「泊车了但 rail 看不出在等人」同为 violation。
      // （fix-webui-qa-round14 4.2）本地泊车的聚焦会话在项目组与等待组各渲染
      // 一份（同带 current、同态）——`.first()` 取项目组副本，避免 strict
      // violation；两份呈现逐字相同，断言哪份都等价。
      const currentRow = page
        .locator('sebas-project-rail li.session-item.current')
        .first()
      await expect(currentRow).toHaveClass(/waiting/, { timeout: 15_000 })
      await expect(currentRow.locator('[data-testid="session-waiting"]')).toBeVisible()

      // 同一面可决策：allow once → 第二回合照常收尾，两回合的工具都落了。
      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await detail.expectStatus('done', 20_000)
      await expect
        .poll(
          async () => {
            await detail.expandAllFolds()
            return detail.turnWith('perm done').count()
          },
          { timeout: 20_000 },
        )
        .toBe(2)

      // 决策落定后 waiting 面退场。
      await expect(currentRow).not.toHaveClass(/waiting/, { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
