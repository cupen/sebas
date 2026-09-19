/**
 * Journey — 审批读模型恢复（fix-webui-approval-restore-and-session-identity
 * 1.3，tasks.md 7.1）。
 *
 * 功能：审批卡片旅程 / 子功能：审批读模型恢复
 *
 * Spec anchors: permission-flow「未决请求可从读模型取得」「刷新或重连后仍可
 * 取得」「批复后从读模型消失」+ agent-workbench（MODIFIED）「刷新后审批面从
 * 读模型重建」「重建与推送按 request_id 幂等合并」「waiting is not reported
 * as working」。
 *
 * The defect this pins: the review surface's only data source used to be the
 * one-shot WS `permission.requested` push — a reload (or reconnect) lost the
 * parked request forever (broadcast does not replay to new subscribers). The
 * contract now: `GET /api/sessions/{key}/approvals` is the read model; the
 * review card rebuilds from it when the page (re)opens, merging any push by
 * `request_id` so the parked request surfaces exactly once, and a decided id
 * never resurrects a card. The rail additionally presents the parked session
 * as WAITING (wait badge + waiting dot), not working.
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSessionApprovals,
  ProjectRail,
  resetState,
  ReviewCards,
  waitStatus,
} from './helpers/index'

test.describe('审批卡片旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('审批读模型恢复', () => {
    test('reload rebuilds the review card from the read model under the same request_id, and deciding clears it for good', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])

      // Live WS on the focused session BEFORE the gate fires — the same
      // discipline as permission.spec.ts (the card is instant via the push;
      // the read model is what must survive the reload below).
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await expect(page.locator('sebas-review-cards')).toHaveCount(1)
      await detail.sendFollowUp('perm')

      // Card arrives with the gated call's semantics…
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await expect(cards.card().locator('.head .tool')).toHaveText('Bash')
      await expect(cards.card().locator('pre.args')).toContainText('rm -rf /')
      const requestId = await cards.card().getAttribute('data-request-id')
      expect(requestId).toBeTruthy()

      // …the read model agrees (request_id / tool / args), independent of WS.
      const parked = await getSessionApprovals(page.request, key)
      expect(parked.status).toBe(200)
      expect(parked.approvals).toHaveLength(1)
      expect(parked.approvals[0].request_id).toBe(requestId)
      expect(parked.approvals[0].tool_name).toBe('Bash')
      expect(JSON.stringify(parked.approvals[0].args)).toContain('rm -rf /')

      // waiting is not reported as working: the rail row carries the wait
      // badge and the waiting dot while the decision is outstanding.
      await rail.ensureProjectExpanded(projectName)
      const row = page
        .locator('sebas-project-rail li.session-item', { hasText: 'idle' })
        .first()
      await expect(row).toBeVisible({ timeout: 10_000 })
      await expect(row.locator('[data-testid="session-waiting"]')).toBeVisible({
        timeout: 10_000,
      })
      await expect(row.locator('.session-dot')).toHaveAttribute('data-status', 'waiting', {
        timeout: 10_000,
      })

      // ── the pinned defect: reload while parked ─────────────────────────
      // No push can replay across a reload — the rebuilt card can only come
      // from the read model. Same request_id, exactly ONE decision surface
      // (the idempotent merge contract), buttons live.
      await page.reload()
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await expect(cards.all()).toHaveCount(1)
      await expect(cards.card()).toHaveAttribute('data-request-id', requestId!)
      await expect(cards.allowOnce()).toBeEnabled()
      // The read model still lists it after the reload (刷新或重连后仍可取得).
      const parkedAfterReload = await getSessionApprovals(page.request, key)
      expect(parkedAfterReload.approvals.map((a) => a.request_id)).toEqual([requestId])

      // Deciding from the REBUILT surface settles the turn end-to-end…
      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 20_000 })
      await detail.expectFoldedText('perm done')
      await detail.expectStatus('done')

      // …the read model drains (批复后从读模型消失)…
      const drained = await getSessionApprovals(page.request, key)
      expect(drained.approvals).toEqual([])

      // …and a further reload never resurrects a card for the decided id
      // (墓碑：推送与读模型都不能让已决请求复活).
      await page.reload()
      await expect(cards.all()).toHaveCount(0, { timeout: 10_000 })
      expect(collector.clean()).toEqual([])
    })
  })
})
