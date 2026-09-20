/**
 * Journey — 泊车态停止可达性（round6 GUI 验收固化，2026-09-21）。
 *
 * 功能：停止收尾 / 子功能：泊车审批态下停止控件可点
 *
 * Spec anchors: agent-workbench（ADDED）「Stop reply fully settles the turn」
 * 的「停止控件随回合结算消失」+ permission-flow「Cancel 释放泊车审批
 * （fail-closed）」的「停止回复清空未决审批」。
 *
 * The defect this pins（round6 GUI 验收活体复现）: the parked review card
 * squeezes the composer column; the wa-textarea's `min-height: 36px` made its
 * host box overflow the shrinking `.input-wrap` (min-height: 0) and cover
 * `.composer-bottom` — the stop control stayed VISIBLE but every click
 * (real or Playwright) landed on the textarea (`intercepts pointer events`,
 * 54 retries in stop-settle). Fix moves the 36px floor onto `.input-wrap` so
 * the textarea always fits its parent. This journey pins the contract
 * geometrically AND behaviorally: in the parked state the stop control's box
 * does not intersect the composer input's box, and a plain (non-forced)
 * click on it stops the turn.
 *
 * 场景基座：与 stop-settle.spec 同款——"perm" 泊车一张审批卡，停止窗口
 * 无时限（泊车是稳态，点击落点确定）。
 */
import { expect, test, type Locator } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSessionApprovals,
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

  test.describe('泊车审批态下停止控件可点', () => {
    test('parked: stop control is unobstructed by the composer input and a plain click lands', async ({
      page,
    }) => {
      const workbench = new Workbench(page)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await detail.sendFollowUp('perm')
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await expect(workbench.submitControl).toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })

      // 几何契约：停止控件与 composer 输入盒不相交。缺陷形态里 textarea
      // 宿主盒溢出 .input-wrap、右下角压住控件（控件可见但命中测试落在
      // textarea）——两盒相交即回归，先于任何点击给出可诊断的失败面。
      // locator.evaluate 穿透 shadow root（document.querySelector 不会）。
      const boxOf = (loc: Locator) =>
        loc.evaluate((el) => {
          const r = el.getBoundingClientRect()
          return { left: r.left, right: r.right, top: r.top, bottom: r.bottom }
        })
      const stop = await boxOf(workbench.submitControl)
      const input = await boxOf(
        page.locator('sebas-workbench-composer [data-testid="composer-input"]'),
      )
      const intersects =
        stop.left < input.right &&
        input.left < stop.right &&
        stop.top < input.bottom &&
        input.top < stop.bottom
      expect(intersects, 'stop control must not be covered by the composer input').toBe(false)

      // 行为契约：plain click（无 force——actionability 含命中目标校验，
      // 遮挡回归在此即红）落在控件上：审批读模型排空、控件离开 stop 态、
      // 转录落地「回合被停止」条目。释放语义的深断言（迟到批复 404、过期
      // 卡片、reload 不复活）归 stop-settle.spec 泊车旅程，这里不重复。
      await workbench.submitControl.click()
      const released = await getSessionApprovals(page.request, key)
      expect(released.approvals).toEqual([])
      await expect(workbench.submitControl).not.toHaveAttribute('data-state', 'stop', {
        timeout: 10_000,
      })
      await expect(
        page.locator('sebas-dashboard sebas-transcript-view').getByText('回合被停止'),
      ).toBeVisible({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
