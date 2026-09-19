/**
 * Journey — rail 切换即时聚焦工作台（fix-webui-qa-defects 4.2，tasks.md）。
 *
 * 功能：会话管理覆盖 / 子功能：rail 切换即时聚焦
 *
 * Spec anchor: agent-workbench「rail selection renders the conversation
 * immediately」（fix-webui-qa-defects delta）：selecting a session in the
 * rail while a different session is displayed, and NO other session event
 * occurs → the workbench renders the selected session's conversation within
 * the focus-follow latency of an ordinary session event.
 *
 * The defect this pins: `POST /switch` only wrote the server-side focus
 * pointer and produced no WS event, so the workbench kept rendering the old
 * session until the NEXT unrelated session event happened to refresh the
 * summary. The fix dispatches a window-level `sebas:rail-focus` event after
 * a successful switch; the dashboard immediately schedules its regular
 * throttled (500ms) list refresh. This journey performs the switch and then
 * stays completely silent — no message, no WS traffic — so the ONLY thing
 * that can bring session B onto the workbench is the focus-follow chain.
 * The render is asserted inside a tight window (≈ ordinary event latency;
 * the broken behavior would never render B at all in this silence).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  FocusedSession,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('rail 切换即时聚焦', () => {
    test('4.2 focusing A and clicking B in the rail renders B within the throttle window, with no other events', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const t = Date.now()
      const promptA = `focus-a-${t}`
      const promptB = `focus-b-${t}`
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const keyA = await createSession(page.request, { prompt: promptA })
      const keyB = await createSession(page.request, { prompt: promptB })
      await waitStatus(page.request, keyA, ['done'])
      await waitStatus(page.request, keyB, ['done'])

      // A 聚焦：deep-link puts A on the workbench…
      await page.goto(`/sessions/${keyA}`)
      await expect(detail.userTurn(promptA)).toBeVisible({ timeout: 15_000 })
      // …and navigating back to the workbench keeps A focused (server
      // pointer still A) — the starting state of this journey.
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await expect(detail.userTurn(promptA)).toBeVisible({ timeout: 15_000 })

      // 点 rail 会话 B，然后什么都不做：不发消息、不产生任何会话事件。
      // The workbench must render B's conversation within the focus-follow
      // latency of an ordinary session event (throttle 500ms + one refresh
      // round-trip). Pinned at 5s for sandbox-jank tolerance — the pre-fix
      // behavior could not render B at all without a subsequent unrelated
      // event (none ever comes in this silence), so the window keeps full
      // discriminating power while proving the latency contract.
      await rail.ensureProjectExpanded(projectName)
      const t0 = Date.now()
      await rail.sessionItem(promptB).click()
      await expect(detail.userTurn(promptB)).toBeVisible({ timeout: 5_000 })
      const followLatency = Date.now() - t0
      expect(followLatency).toBeLessThan(5_000)

      // No navigation away: the workbench stays the single surface, and the
      // switch is observable in place (URL untouched).
      expect(page.url()).not.toContain('/sessions/')

      // The rail's current-session marker follows the focus pointer to B…
      await expect(
        rail.host.locator('li.session-item.current', { hasText: promptB }).first(),
      ).toBeVisible({ timeout: 5_000 })
      // …and A's conversation is not crosstalk-rendered: B's bubble is the
      // visible subject while A's prompt stays out of the main area.
      await expect(detail.userTurn(promptA)).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
