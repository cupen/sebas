/**
 * Journey — approval_answer over the DETACHED dual-process topology
 * (cover-core-channel-test-gaps B1.2, design D4).
 *
 * 功能：审批卡片旅程 / 子功能：detached 双进程审批闭环 + 未知 rid 拒绝
 *
 * Same "perm" trigger as permission.spec.ts (single-process), but the whole
 * approval loop crosses the core session channel: the standalone webui and
 * the core are separate PROCESSES, so the gated tool call has to travel
 * fake-claude → core channel server → ApprovalRequested frame → standalone
 * webui review card → POST /api/permissions/{rid}/answer → ApprovalAnswer
 * frame → core → ACP child. The two topologies do not substitute for each
 * other (design D4); the single-process journeys stay green untouched.
 *
 * Cold-scene note: the approval feed is fire-and-forget — ApprovalRequested
 * frames are NOT replayed to a subscriber that was not attached when the
 * frame fired (documented fail-closed contract: a gate with no reachable
 * client is denied by the kernel, never faked). On a freshly assembled scene
 * the first browser page can therefore miss the first frame while the
 * standalone webui's channel client is still converging; the journeys wait
 * for the composer gate and the review-cards subscription before triggering,
 * and Playwright's retry absorbs any residual cold-window loss.
 *
 * Runs under playwright.detached.config.ts (port 9897) — stopping the core
 * here would be lethal to the shared single-process sandbox, so this file is
 * ignored by the main config (and vice versa: only the detached topology
 * runs it).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  detachedSceneDir,
  ErrorCollector,
  isCoreAlive,
  reachabilityOk,
  ReviewCards,
  SessionDetailPage,
  startCore,
  waitForCoreReachability,
  waitStatus,
} from './helpers/index'

test.describe('审批卡片旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Restore the healthy baseline if a previous attempt left the core stopped. */
  async function ensureCoreUp(request: import('@playwright/test').APIRequestContext) {
    const scene = detachedSceneDir()
    if ((await reachabilityOk(request)) !== true) {
      if (!isCoreAlive(scene)) {
        startCore(scene)
      }
      await waitForCoreReachability(request, true, 45_000)
    }
  }

  /** Idle session whose detail page holds a live WS, then trigger "perm". */
  async function openDetachedIdleAndTriggerPerm(
    page: import('@playwright/test').Page,
  ) {
    await ensureCoreUp(page.request)

    const detail = new SessionDetailPage(page)
    const cards = new ReviewCards(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(detail.host).toBeVisible()
    // Cold webui start: the composer is gated on the standalone backend's
    // first reachability sample (5s poll cadence) — wait for it to enable
    // before sending, otherwise the follow-up silently never lands. (The
    // parked gate does NOT flip the status slug off 'done' — the review
    // card is the only start-of-turn signal, in either topology.)
    await expect(detail.composerTextarea).toBeEnabled({ timeout: 30_000 })
    // The approval feed is fire-and-forget: make sure the review-cards
    // element is attached — i.e. its ws subscription is live — BEFORE the
    // gate fires.
    await expect(page.locator('sebas-review-cards')).toHaveCount(1)
    await detail.sendFollowUp('perm')
    return { detail, cards }
  }

  test.describe('detached 审批闭环', () => {
    test('allow path — ApprovalRequested 跨进程到达 review-card，allow 后 transcript 记录允许语义', async ({
      page,
    }) => {
      const { detail, cards } = await openDetachedIdleAndTriggerPerm(page)

      // Card appears with the gated call's semantics: Bash + the command args.
      await expect(cards.all().first()).toBeVisible({ timeout: 30_000 })
      await expect(cards.card().locator('.head .tool')).toHaveText('Bash')
      await expect(cards.card().locator('pre.args')).toContainText('rm -rf /')

      await cards.allowOnce().click()

      // Card resolved; the transcript records the ALLOWED tool result and
      // the turn completes — all the way through the ACP child on the core.
      await expect(cards.all()).toHaveCount(0, { timeout: 20_000 })
      await expect(detail.turnWith('perm done').first()).toBeVisible({
        timeout: 20_000,
      })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', {
        timeout: 20_000,
      })

      expect(collector.clean()).toEqual([])
    })

    test('deny path — deny 后 transcript 记录拒绝语义，回合完成', async ({
      page,
    }) => {
      const { detail, cards } = await openDetachedIdleAndTriggerPerm(page)

      await expect(cards.all().first()).toBeVisible({ timeout: 30_000 })
      await cards.deny().click()

      await expect(cards.all()).toHaveCount(0, { timeout: 20_000 })
      await expect(detail.turnWith('denied by fake').first()).toBeVisible({
        timeout: 20_000,
      })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', {
        timeout: 20_000,
      })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('unknown rid', () => {
    test('POST /api/permissions/{rid}/answer with an unknown rid → 404 typed rejection', async ({
      page,
    }) => {
      await ensureCoreUp(page.request)

      const resp = await page.request.post(
        '/api/permissions/toolu_does_not_exist/answer',
        // The answer body nests the internally-tagged PermissionDecision
        // (same shape the frontend sends: `{decision: {decision: ...}}`).
        { data: { decision: { decision: 'allow_once' } } },
      )
      expect(resp.status()).toBe(404)
      const body = (await resp.json()) as { error?: string }
      expect(body.error).toContain('no pending permission request')

      expect(collector.clean()).toEqual([])
    })
  })
})
