/**
 * Journey — interrupt 全程收尾（fix-webui-approval-restore-and-session-identity
 * 2.1/2.2/2.4，tasks.md 7.1）。
 *
 * 功能：停止收尾 / 子功能：回合被停止条目、停止释放泊车审批
 *
 * Spec anchors: agent-workbench（ADDED）「Stop reply fully settles the turn」
 * 的「停止后 transcript 有停止条目」「停止控件随回合结算消失」「刷新后不复活
 * 在飞状态」+ permission-flow「Cancel 释放泊车审批（fail-closed）」的「停止
 * 回复清空未决审批」「释放后回合状态复位」「释放的请求不可再批复」。
 *
 * The defect this pins: cancel used to be a half settlement — the parked
 * approvals survived as orphans (turn_engaged stayed true forever), and the
 * stopped turn vanished silently (no transcript entry). The contract now:
 * stop releases every parked request (read model drains, a late decision is
 * rejected), appends an error-class「回合被停止」entry, and resets the
 * engaged state so the stop control disappears — stably across reloads.
 *
 * 场景基座：沙箱 fake-claude 带 `--slow-ms 800`，"stream" 触发 5 帧 × 250ms
 * ——在飞窗口 ≈2s，停止点击有确定性落点（同 submit-control.spec 的纪律）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSessionApprovals,
  getSession,
  ProjectRail,
  resetState,
  ReviewCards,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('停止收尾', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('回合被停止条目', () => {
    test('stopping a streaming turn appends the stop entry, resets the control, and stays settled across reloads', async ({
      page,
    }) => {
      const workbench = new Workbench(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])
      const entriesBefore = (await getSession(page.request, key)).detail!.entries.length

      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // Start a streaming turn from the live page and catch the stop window
      // (~2s: 5 frames × 250ms + slow-ms 800).
      await detail.sendFollowUp('stream')
      await expect(workbench.submitControl).toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })
      await workbench.submitControl.click()

      // The stopped turn is VISIBLE: an error-class entry states the turn was
      // stopped, instead of the user's message silently hanging.
      await expect(
        page.locator('sebas-dashboard sebas-transcript-view').getByText('回合被停止'),
      ).toBeVisible({ timeout: 15_000 })

      // The composer no longer offers stop once the turn settled.
      await expect(workbench.submitControl).not.toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })
      // API truth: the settled turn is not engaged (turn_engaged only rides
      // the wire while true) and the transcript carries the stop entry.
      const settled = (await getSession(page.request, key)).detail!
      expect(settled.turn_engaged).toBeUndefined()
      expect(
        settled.entries.some((e) => e.element_type === 'error' && e.content.includes('回合被停止')),
      ).toBe(true)

      // A reloaded page must NOT present the stopped turn as still in flight:
      // entry persists, no stop control, session idle.
      await page.reload()
      await expect(
        page.locator('sebas-dashboard sebas-transcript-view').getByText('回合被停止'),
      ).toBeVisible({ timeout: 15_000 })
      await expect(workbench.submitControl).not.toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })

      // The session survived the interrupt: a follow-up completes normally
      // (and the transcript keeps both the stop entry and the new turn).
      await detail.sendFollowUp('still there?')
      await detail.expectStatus('done')
      const after = (await getSession(page.request, key)).detail!
      expect(after.entries.length).toBeGreaterThan(entriesBefore + 1)
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('停止释放泊车审批', () => {
    test('stopping a parked turn releases the approval fail-closed: read model drains, late decision is rejected, stop entry lands', async ({
      page,
    }) => {
      const workbench = new Workbench(page)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await expect(page.locator('sebas-review-cards')).toHaveCount(1)
      await detail.sendFollowUp('perm')
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      const requestId = await cards.card().getAttribute('data-request-id')
      expect(requestId).toBeTruthy()

      // Parked = engaged: the stop control applies while a decision is
      // outstanding (turn_engaged covers parked_count > 0).
      await expect(workbench.submitControl).toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })

      // Stop the reply: every parked request of the session is released…
      await workbench.submitControl.click()
      const released = await getSessionApprovals(page.request, key)
      expect(released.status).toBe(200)
      expect(released.approvals).toEqual([])
      // …the rail's waiting projection flips back…
      const row = page
        .locator('sebas-project-rail li.session-item', { hasText: 'idle' })
        .first()
      await rail.ensureProjectExpanded(projectName)
      await expect(row).toBeVisible({ timeout: 10_000 })
      await expect(row.locator('[data-testid="session-waiting"]')).toHaveCount(0, {
        timeout: 10_000,
      })
      // …and the turn settles with the visible stop entry.
      await expect(
        page.locator('sebas-dashboard sebas-transcript-view').getByText('回合被停止'),
      ).toBeVisible({ timeout: 15_000 })

      // 释放的请求不可再批复 — API face: a late decision for the released id
      // is a typed rejection (404), unblocking nothing.
      const late = await page.request.post(`/api/permissions/${requestId}/answer`, {
        data: { decision: { decision: 'allow_once' } },
      })
      expect(late.status()).toBe(404)
      expect(((await late.json()) as { error?: string }).error).toContain(
        'no pending permission request',
      )

      // UI face: the stale card stays visible but inert — deciding it answers
      // 404 and the card honestly degrades to the expired state.
      await cards.deny().click()
      await expect(cards.card()).toHaveAttribute('data-state', 'expired', { timeout: 10_000 })

      // Reload: no stop control (released → turn_engaged false), the stop
      // entry persists, and the decided/expired card is not resurrected.
      await page.reload()
      await expect(workbench.submitControl).not.toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })
      await expect(
        page.locator('sebas-dashboard sebas-transcript-view').getByText('回合被停止'),
      ).toBeVisible({ timeout: 15_000 })
      const settled = (await getSession(page.request, key)).detail!
      expect(settled.turn_engaged).toBeUndefined()
      expect(
        settled.entries.some((e) => e.element_type === 'error' && e.content.includes('回合被停止')),
      ).toBe(true)
      expect(collector.clean()).toEqual([])
    })
  })
})
