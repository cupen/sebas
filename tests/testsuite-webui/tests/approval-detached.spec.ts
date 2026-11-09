/**
 * Journey — approval_answer over the DETACHED dual-process topology
 * (cover-core-channel-test-gaps B1.2, design D4).
 *
 * 功能：审批卡片旅程 / 子功能：detached 双进程审批闭环 + 未知 rid 拒绝
 *
 * Same "perm" trigger as permission.spec.ts (single-process), but the whole
 * approval loop crosses the core session channel. React 入口的审批面是
 * role=alertdialog「待审批 <tool>」，动作是允许一次 / 本会话允许 / 拒绝；
 * 折叠体用 details 承载，断言前展开。
 *
 * Runs under playwright.detached.config.ts (React port 5288).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  detachedSceneDir,
  ErrorCollector,
  isCoreAlive,
  reachabilityOk,
  startCore,
  waitForCoreReachability,
  waitStatus,
} from './helpers/index'
import { ReactWorkbench } from './helpers/pages/react'

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
  ): Promise<ReactWorkbench> {
    await ensureCoreUp(page.request)

    const workbench = new ReactWorkbench(page)
    const key = await createSession(page.request, { prompt: 'idle' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(workbench.sessionPage).toBeVisible()
    // Cold webui start: the composer is gated on the standalone backend's
    // first reachability sample — wait for it to enable before sending.
    await expect(workbench.composerTextarea).toBeEnabled({ timeout: 30_000 })
    await workbench.send('perm')
    return workbench
  }

  async function expectStatus(page: import('@playwright/test').Page, slug: string): Promise<void> {
    const workbench = new ReactWorkbench(page)
    await expect
      .poll(async () => workbench.statusText(), { timeout: 20_000 })
      .toBe(slug === 'done' ? '完成' : slug)
  }

  test.describe('detached 审批闭环', () => {
    test('allow path — ApprovalRequested 跨进程到达 review-card，allow 后 transcript 记录允许语义', async ({
      page,
    }) => {
      const workbench = await openDetachedIdleAndTriggerPerm(page)

      // Card appears with the gated call's semantics: Bash + the command args.
      const card = workbench.approvalCard('Bash')
      await expect(card).toBeVisible({ timeout: 30_000 })
      await expect(card).toContainText('rm -rf /')

      await card.getByRole('button', { name: /允\s*许\s*一\s*次/ }).click()

      // Card resolved; the transcript records the ALLOWED tool result and
      // the turn completes — all the way through the ACP child on the core.
      await expect(workbench.approvalCards()).toHaveCount(0, { timeout: 20_000 })
      await workbench.expectFoldedText('perm done')
      await expectStatus(page, 'done')

      expect(collector.clean()).toEqual([])
    })

    test('deny path — deny 后 transcript 记录拒绝语义，回合完成', async ({ page }) => {
      const workbench = await openDetachedIdleAndTriggerPerm(page)

      const card = workbench.approvalCard('Bash')
      await expect(card).toBeVisible({ timeout: 30_000 })
      await card.getByRole('button', { name: /拒\s*绝/ }).click()

      await expect(workbench.approvalCards()).toHaveCount(0, { timeout: 20_000 })
      await workbench.expectFoldedText('denied by fake')
      await expectStatus(page, 'done')

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
