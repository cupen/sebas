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
import { createSession, ErrorCollector, ReviewCards, SessionDetailPage, waitStatus } from './helpers/index'

test.describe('审批卡片旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Navigate to an idle session with a live WS, then trigger "perm". */
  async function openAndTriggerPerm(page: import('@playwright/test').Page) {
    const detail = new SessionDetailPage(page)
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
      await expect(detail.turnWith('denied by fake').first()).toBeVisible({ timeout: 15_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('单次允许路径', () => {
    test('allow-once path — allowed semantics, turn completes', async ({ page }) => {
      const { detail, cards } = await openAndTriggerPerm(page)

      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowOnce().click()

      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await expect(detail.turnWith('perm done').first()).toBeVisible({ timeout: 15_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('会话级允许', () => {
    test('allow-session path — observed product gap: the follow-up call is gated again', async ({ page }) => {
      // ⚠️ REAL product finding (recorded in design.md 实现期发现 1): the
      // WebUI `answer_permission` path sends the ACP PermissionReply but never
      // calls `SessionAllowlist::grant_all` — that registration lives only on
      // the feishu card-click path (inbound.rs). So from the WebUI, "allow
      // session" does NOT persist: an identical follow-up tool call in the
      // same session is gated again. We assert that observed, stable behavior.
      const { detail, cards } = await openAndTriggerPerm(page)

      // First call: allow for session.
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowSession().click()
      await expect(detail.turnWith('perm done').first()).toBeVisible({ timeout: 15_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Second identical call in the SAME session: since the WebUI path never
      // registered grant_all, it is gated again (a new review card appears).
      await detail.sendFollowUp('perm')
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })

      // Resolve it so the turn finishes and the suite stays deterministic.
      await cards.allowOnce().click()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 20_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
