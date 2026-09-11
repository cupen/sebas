/**
 * Journey — conversation view semantics (workbench-conversation-view 2.1–2.4,
 * 4.3/4.4 browser journeys for task 5.4).
 *
 * - 两侧交替: submissions and agent replies alternate in transcript order,
 *   each submission rendered as the operator's own turn bubble (2.3).
 * - 工具组展开: a gated tool call ("perm" trigger) renders as the turn's
 *   expandable "used N tools" group — distinguishable from prose (2.2) —
 *   and expands to reveal the invocation details.
 * - 就地聚焦: a rail click focuses the session in place; the workbench
 *   renders its conversation without a page change (3.1).
 * - 模型两级选择: with a provider catalog configured in Settings, the
 *   creation-mode composer offers provider → model in two levels (4.3);
 *   an unreachable catalog states its unavailability instead of an empty
 *   list (4.4).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSession,
  ProjectRail,
  resetState,
  ReviewCards,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('对话视图（workbench-conversation-view）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('两侧交替', () => {
    test('submissions and replies alternate in order across several turns', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const t = Date.now()
      const q1 = `q1-${t}`
      const q2 = `q2-${t}`
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: q1 })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()
      await expect(detail.userTurn(q1)).toBeVisible()
      await expect(detail.agentTurn('world').first()).toBeVisible({ timeout: 15_000 })

      // Second round via the live composer (never send into a running turn).
      await detail.sendFollowUp(q2)
      await expect(detail.userTurn(q2)).toBeVisible({ timeout: 20_000 })
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 20_000 })

      // Alternation: operator turn q1 < agent reply < operator turn q2 in
      // the rendered conversation stream.
      const blocks = page.locator(
        'sebas-dashboard sebas-transcript-view .turn-block',
      )
      const count = await blocks.count()
      expect(count).toBeGreaterThanOrEqual(4)
      const texts = await blocks.allTextContents()
      const classes = []
      for (let i = 0; i < count; i++) {
        classes.push(await blocks.nth(i).getAttribute('class'))
      }
      const q1Idx = texts.findIndex((s) => s.includes(q1))
      const firstAgentIdx = classes.findIndex((c) => c?.includes('is-assistant'))
      const q2Idx = texts.findIndex((s) => s.includes(q2))
      expect(q1Idx).toBeGreaterThanOrEqual(0)
      expect(firstAgentIdx).toBeGreaterThan(q1Idx)
      expect(q2Idx).toBeGreaterThan(firstAgentIdx)
      // The operator turns really are the user-styled bubbles (两侧交替).
      for (const [i, c] of classes.entries()) {
        if (texts[i].includes(q1) || texts[i].includes(q2)) {
          expect(c?.includes('is-user'), `turn ${i} must be operator-styled`).toBe(true)
        }
      }

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('工具组展开', () => {
    test('the gated tool call renders as the turn tool group and expands', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await detail.sendFollowUp('perm')

      // Resolve the gate; the turn completes with the tool group inside it.
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })

      // The turn's tool invocations are grouped — NOT ordinary prose (2.2).
      const group = detail.toolGroup().first()
      await expect(group).toBeVisible({ timeout: 15_000 })
      const label = (await group.locator('summary .label').textContent()) ?? ''
      expect(label).toMatch(/used \d+ tools?/)
      // Collapsed by default; the invocation payload is hidden until expanded.
      expect(await group.getAttribute('open')).toBeNull()

      // Expand (the same action a keyboard user's Enter on the summary
      // triggers — native details/summary): the tool detail becomes visible.
      await group.locator('summary').click()
      await expect(group).toHaveAttribute('open', '')
      await expect(group).toContainText('perm done')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('就地聚焦', () => {
    test('rail click focuses the session in place on the workbench', async ({ page }) => {
      test.setTimeout(90_000)
      const t = Date.now()
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)
      const workbench = new Workbench(page)

      await resetState(page.request)
      // rail-declutter-unread：Inbox 分组移除 → 会话绑定 scene 项目才会
      // 出现在 rail；行名即首条 prompt。
      const { id: projectId, name: projectName } = await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: `inplace-${t}`, projectId })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/')
      await expect(workbench.composer).toBeVisible()
      await rail.expandProject(projectName)
      await rail.sessionItem(`inplace-${t}`).click()

      // The conversation renders IN PLACE: same workbench surface, no
      // navigation to any other view.
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.userTurn(`inplace-${t}`)).toBeVisible()
      expect(page.url()).not.toContain('/sessions/')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('模型两级选择', () => {
    test('the creation dialog offers provider → model from the Settings catalog; empty catalog is stated honestly', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)

      // Empty catalog first: the dialog states unavailability honestly
      // instead of rendering an empty or fabricated list (4.4).
      await page.goto('/')
      await rail.openNewSessionDialog(projectName)
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-catalog-unavailable"]'),
      ).toBeVisible({ timeout: 15_000 })
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-provider-select"]'),
      ).toHaveCount(0)
      await rail.cancelNewSessionDialog()

      // Configure a catalog (the same write path the Settings Models editor
      // uses) — the dialog must reflect it without any session existing.
      // Re-run safety: drop a leftover from an earlier journey first.
      await page.request.delete('/router/api/providers/catalog-provider')
      const created = await page.request.post('/router/api/providers', {
        data: {
          name: 'catalog-provider',
          base_url_openai_chat: 'http://127.0.0.1:9/v1',
          models: [
            { id: 'cat-small', tags: [] },
            { id: 'cat-large', tags: ['vision'] },
          ],
        },
      })
      expect(created.ok()).toBe(true)

      // Reopen the dialog (sebas:refetch re-runs the shared catalog load).
      await rail.openNewSessionDialog(projectName)
      // Two levels: provider first…
      const providerSelect = rail
        .newSessionDialog()
        .locator('[data-testid="dialog-provider-select"]')
      await expect(providerSelect).toBeVisible({ timeout: 15_000 })
      await expect(providerSelect).toContainText('catalog-provider')
      // …then the model of the chosen provider.
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-model-select"]'),
      ).toBeVisible()
      await expect(
        rail.newSessionDialog().locator('[data-testid="dialog-model-select"]'),
      ).toContainText('cat-small')

      // Clean up: the provider lives in the SHARED core store — leaving it
      // behind would flip later journeys' honest "no provider configured"
      // first-paint assertions.
      await rail.cancelNewSessionDialog()
      const removed = await page.request.delete('/router/api/providers/catalog-provider')
      expect(removed.ok()).toBe(true)

      expect(collector.clean()).toEqual([])
    })
  })
})

/** Short rail label for a session key (mirrors the backend elision). */
