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
      await expect(detail.turnWith('perm done').first()).toBeVisible({ timeout: 15_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Second identical call in the SAME session: auto mode answers the gate
      // driver-side, so the turn completes with NO review card at all.
      await detail.sendFollowUp('perm')
      await expect(detail.turnWith('perm done')).toHaveCount(2, { timeout: 20_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 20_000 })
      await expect(cards.all()).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
