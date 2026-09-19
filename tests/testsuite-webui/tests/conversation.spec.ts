/**
 * Journey — conversation view semantics (workbench-conversation-view 2.1–2.4,
 * 4.3/4.4 browser journeys for task 5.4).
 *
 * - 两侧交替: submissions and agent replies alternate in transcript order,
 *   each submission rendered as the operator's own turn bubble (2.3).
 * - 过程折叠: a gated tool call ("perm" trigger) folds into the turn's
 *   single process fold — collapsed by default, summary `process · N`
 *   (process-folds 2.1) — and expands level by level: second-level per-entry
 *   folds titled by the structured title (`Bash · rm -rf /` / `✓ Bash`),
 *   whose payload stays hidden until each is expanded (2.2).
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
        'sebas-dashboard sebas-transcript-view .turn-block',
      )
      await expect
        .poll(async () => await blocks.count(), { timeout: 20_000 })
        .toBeGreaterThanOrEqual(4)
      const count = await blocks.count()
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

  test.describe('过程折叠', () => {
    test('the gated tool call folds into the turn process fold and expands level by level', async ({
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

      // The turn's process entries collect into ONE outer fold — NOT
      // ordinary prose（workbench-natural-conversation-flow：折叠行三段式
      // = `process` 标签 + 尾条目结构化 title（实时「进行中工具」）+ 条目
      // 计数；collapsed by default, the invocation payload hidden until
      // expanded——展开体懒渲染，收起时不在 DOM）.
      const fold = detail.processFold().first()
      await expect(fold).toBeVisible({ timeout: 15_000 })
      const outerLink = fold.locator('[data-testid="process-fold-link"]')
      await expect(outerLink.locator('.label')).toHaveText('process')
      // 尾条目 = tool_result（结构化 title 同二级折叠），计数 = 2。
      await expect(outerLink.locator('.running')).toHaveText('✓ Bash')
      await expect(outerLink.locator('.fold-count')).toHaveText('2')
      await expect(outerLink).toHaveAttribute('aria-expanded', 'false')
      await expect(fold.locator('.fold-body')).toHaveCount(0)

      // Expand the outer fold（键盘用户对折叠行按 Enter 的同一动作）:
      // second-level per-entry folds appear, themselves collapsed by default
      // (2.2), titled by the backend's structured title.
      await outerLink.click()
      await expect(outerLink).toHaveAttribute('aria-expanded', 'true')
      const items = detail.processItems()
      await expect(items).toHaveCount(2)
      await expect(items.nth(0).locator('.item-title')).toHaveText('Bash · rm -rf /')
      await expect(items.nth(1).locator('.item-title')).toHaveText('✓ Bash')
      for (let i = 0; i < 2; i++) {
        await expect(items.nth(i).locator('[data-testid="process-item-link"]')).toHaveAttribute(
          'aria-expanded',
          'false',
        )
        await expect(items.nth(i).locator('.item-body')).toHaveCount(0)
      }

      // Second-level expand reveals the invocation detail (its result text).
      await items.nth(1).locator('[data-testid="process-item-link"]').click()
      await expect(items.nth(1).locator('.item-body')).toContainText('perm done')
      // The sibling fold stays collapsed — expansion is per entry.
      await expect(items.nth(0).locator('.item-body')).toHaveCount(0)

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
      await rail.ensureProjectExpanded(projectName)
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
        const modelSelect = rail
          .newSessionDialog()
          .locator('[data-testid="dialog-model-select"]')
        await expect(
          rail.newSessionDialog().locator('[data-testid="dialog-provider-select"]'),
        ).toBeVisible({ timeout: 15_000 })
        await expect(modelSelect).toContainText('mem-first')

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
        await expect(
          rail
            .newSessionDialog()
            .locator('[data-testid="dialog-provider-select"]'),
        ).toBeVisible({ timeout: 15_000 })
        await expect(modelSelect).toContainText('mem-second')

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
        await expect(
          rail
            .newSessionDialog()
            .locator('[data-testid="dialog-provider-select"]'),
        ).toBeVisible({ timeout: 15_000 })
        await expect(
          rail.newSessionDialog().locator('[data-testid="dialog-model-select"]'),
        ).toContainText('mem-other')
        await expect(
          rail.newSessionDialog().locator('[data-testid="dialog-model-select"]'),
        ).not.toContainText('mem-second')
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
