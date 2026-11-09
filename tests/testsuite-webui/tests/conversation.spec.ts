/**
 * Journey — conversation view semantics (workbench-conversation-view 2.1–2.4,
 * 4.3/4.4 browser journeys for task 5.4).
 *
 * - 两侧交替: submissions and agent replies alternate in transcript order,
 *   each submission rendered as the operator's own turn bubble (2.3).
 * - 过程折叠: a gated tool call ("perm" trigger) lands inside the turn's
 *   single process fold — collapsed by default, summary `过程 · N` — and its
 *   invocation and result merge into ONE collapsed tool call block titled by
 *   the structured title (`Bash · rm -rf /`) with the outcome marker on the
 *   collapsed row; expanding the block reveals arguments and result together
 *   (fold-tool-calls-into-process-tree; D-C3's nested dead-end must not
 *   regress: toggling the sub-block never touches its parent).
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
      await detail.expectStatus('done', 20000)

      // Alternation: operator turn q1 < agent reply < operator turn q2 in
      // the rendered conversation stream. badge 先到、转录气泡后落——
      // 用收敛式等待等第 4 块（q2 的回复）上屏，不钉计数瞬帧。
      const blocks = page.locator(
        ':is([data-testid="workbench-session"],[data-testid="workbench-home"]) [data-testid="react-transcript"] [data-unit-key]',
      )
      await expect
        .poll(async () => await blocks.count(), { timeout: 20_000 })
        .toBeGreaterThanOrEqual(4)
      const count = await blocks.count()
      const texts = await blocks.allTextContents()
      const q1Idx = texts.findIndex((s) => s.includes(q1))
      // React units expose their side through the visible role label rather
      // than a Lit styling class: operator = 你, agent = agent.
      const firstAgentIdx = texts.findIndex((s) => s.includes('agent'))
      const q2Idx = texts.findIndex((s) => s.includes(q2))
      expect(q1Idx).toBeGreaterThanOrEqual(0)
      expect(firstAgentIdx).toBeGreaterThan(q1Idx)
      expect(q2Idx).toBeGreaterThan(firstAgentIdx)
      // The operator turns really are the operator's own bubbles (两侧交替).
      for (const [i, text] of texts.entries()) {
        if (text.includes(q1) || text.includes(q2)) {
          expect(text.trimStart().startsWith('你'), `turn ${i} must be operator-styled`).toBe(true)
        }
      }

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('过程折叠', () => {
    test('the gated tool call merges into one tool call block inside the turn process fold', async ({
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

      // Resolve the gate; the turn completes with the process fold inside it.
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await cards.allowOnce().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })

      // React renders the turn's process run as one native <details> fold:
      // summary = structured tail title (tool · args) + item count, body =
      // every process item (call arguments and result together). The fold is
      // collapsed by default and has no nested details (the Lit nested
      // tool-result block was retired).
      const fold = detail.processFold().first()
      await expect(fold).toBeVisible({ timeout: 15_000 })
      const summary = fold.locator('summary')
      await expect(summary).toContainText('Bash')
      await expect(summary).toContainText('2 项')
      await expect(fold).not.toHaveAttribute('open', '')
      await expect(fold.locator('details')).toHaveCount(0)

      // Expand: arguments and result are readable in the same fold body.
      await summary.click()
      await expect(fold).toHaveAttribute('open', '')
      const body = fold.locator('pre')
      await expect(body.filter({ hasText: 'rm -rf /' }).first()).toBeVisible()
      await expect(body.filter({ hasText: 'perm done' }).first()).toBeVisible()
      await expect(fold.locator('details')).toHaveCount(0)

      // Collapse again: the single process fold stays the only layer.
      await summary.click()
      await expect(fold).not.toHaveAttribute('open', '')

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
      await expect(workbench.host).toBeVisible()
      await rail.ensureProjectExpanded(projectName)
      // A document-level probe proves the focus happens inside the SPA (the
      // React shell routes to /sessions/<key> without a full page load).
      await page.evaluate(() => {
        ;(window as unknown as { __focusProbe?: boolean }).__focusProbe = true
      })
      await rail.sessionItem(`inplace-${t}`).click()

      // The conversation renders IN PLACE on the same shell surface.
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(detail.userTurn(`inplace-${t}`)).toBeVisible()
      expect(page.url()).toContain('/sessions/')
      expect(
        await page.evaluate(
          () => (window as unknown as { __focusProbe?: boolean }).__focusProbe,
        ),
      ).toBe(true)
      await expect(page.getByTestId('react-shell')).toBeVisible()

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
      const emptyDialog = rail.newSessionDialog()
      // React states catalog unavailability on the single combined model
      // select's extra line; the select itself stays disabled (no fabricated
      // or empty list).
      await expect(emptyDialog.getByText(/模型目录不可用/).first()).toBeVisible({
        timeout: 15_000,
      })
      await expect(emptyDialog.getByRole('combobox', { name: '模型' })).toBeDisabled()
      await rail.cancelNewSessionDialog()

      // Configure a catalog (the same write path the Settings Models editor
      // uses) — the dialog must reflect it without any session existing.
      // Re-run safety: drop a leftover from an earlier journey first.
      await page.request.delete('/api/providers/catalog-provider')
      const created = await page.request.post('/api/providers', {
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
      // React presents the provider and model as one combined pair option
      // (`provider / model`) instead of two dependent selects.
      const modelSelect = rail.newSessionDialog().getByRole('combobox', { name: '模型' })
      await expect(modelSelect).toBeVisible({ timeout: 15_000 })
      // AntD 6 的选中标签是 .ant-select-content 的文本（input 恒空）。
      await expect(
        rail.newSessionDialog().locator('.ant-select-content').filter({ hasText: 'catalog-provider / cat-small' }),
      ).toBeVisible()

      // Clean up: the provider lives in the SHARED core store — leaving it
      // behind would flip later journeys' honest "no provider configured"
      // first-paint assertions.
      await rail.cancelNewSessionDialog()
      const removed = await page.request.delete('/api/providers/catalog-provider')
      expect(removed.ok()).toBe(true)

      expect(collector.clean()).toEqual([])
    })

    test('confirming a creation remembers the chosen pair; reopening preselects it (preselect-last-used-model)', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)

      // Seed a catalog with two models (same write path as Settings Models).
      // Re-run safety: drop a leftover from an earlier journey first.
      await page.request.delete('/api/providers/memory-provider')
      const created = await page.request.post('/api/providers', {
        data: {
          name: 'memory-provider',
          base_url_openai_chat: 'http://127.0.0.1:9/v1',
          models: [
            { id: 'mem-first', tags: [] },
            { id: 'mem-second', tags: [] },
          ],
        },
      })
      expect(created.ok()).toBe(true)

      try {
        await page.goto('/')
        await rail.openNewSessionDialog(projectName)
        // First pair preselects with no memory (catalog order): mem-first.
        const modelSelect = rail.newSessionDialog().getByRole('combobox', { name: '模型' })
        await expect(modelSelect).toBeVisible({ timeout: 15_000 })
        const selectedContent = (text: string) =>
          rail.newSessionDialog().locator('.ant-select-content').filter({ hasText: text })
        await expect(selectedContent('mem-first')).toBeVisible()

        // Choose the SECOND model and confirm — the confirmation is the only
        // write point of the last-used memory (localStorage).
        await rail.pickDialogModel('mem-second')
        await rail.confirmNewSessionDialog()
        await expect(rail.newSessionDialog()).toBeHidden({ timeout: 10_000 })
        const remembered = await page.evaluate(
          () => localStorage.getItem('lastUsedModelPair'),
        )
        expect(JSON.parse(remembered ?? 'null')).toEqual({
          provider: 'memory-provider',
          model: 'mem-second',
        })

        // Reopen (same page, rail reuses the dialog): the remembered pair
        // wins the preselection over the catalog's first pair.
        await rail.openNewSessionDialog(projectName)
        await expect(modelSelect).toBeVisible({ timeout: 15_000 })
        await expect(selectedContent('mem-second')).toBeVisible()

        // A stale pair (model no longer in the catalog) must NOT preselect:
        // swap the catalog and the first pair takes over, without surfacing
        // the vanished model as an option.
        await rail.cancelNewSessionDialog()
        const updated = await page.request.put('/api/providers/memory-provider', {
          data: {
            name: 'memory-provider',
            base_url_openai_chat: 'http://127.0.0.1:9/v1',
            models: [{ id: 'mem-other', tags: [] }],
          },
        })
        expect(updated.ok()).toBe(true)
        await rail.openNewSessionDialog(projectName)
        await expect(modelSelect).toBeVisible({ timeout: 15_000 })
        await expect(selectedContent('mem-other')).toBeVisible()
        await expect(selectedContent('mem-second')).toHaveCount(0)
        await rail.cancelNewSessionDialog()
      } finally {
        // Hygiene: the shared core store must not leak into later journeys.
        await page.request.delete('/api/providers/memory-provider')
      }

      expect(collector.clean()).toEqual([])
    })
  })
})

/** Short rail label for a session key (mirrors the backend elision). */
