/**
 * Journey 3.x — model面诚实语义 (phase-2 task 3.2, fallback C +
 * cover-core-channel-test-gaps B2.2 + redesign-provider-models-settings 5.1 +
 * revamp-settings-nav-and-models-editor).
 *
 * 功能：模型管理覆盖 / 子功能：无模型诚实缺省、settings provider 可编辑可
 * 抓取（模型条目 + 能力标记可编辑并持久；抓取入口在编辑器内、成功整单替换
 * 草稿列表、保存才落库、取消即丢弃；无 URL 不渲染入口）、有模型面的正向
 * 切换与 typed rejection
 *
 * redesign-provider-models-settings：provider 模型列表是条目列表（id + 能力
 * 标记），WebUI 编辑器可增删条目、勾选 vision/audio/video（browsing 仍
 * 然零 probe 流量）。
 *
 * revamp-settings-nav-and-models-editor：抓取入口从 provider 行内挪进编辑器
 * 「Models」区块标题旁；抓取成功直接整单替换编辑器草稿模型列表（同 id 保留
 * 人工 capability tags、按 id 去重），保存与否走普通编辑流；行内 🔍 与
 * 「只读结果列表 + 逐条挑选」UI 已删除。
 *
 * ## 两条拒绝路径的区分（B2.2 要求先写清；7f1d7c9 后 SetModel 一律非终态）
 *
 * 判别器是 **agent 是否通告模型面（configOptions.model）**，两者不共享会话、
 * 不共享后端，互不为 flake：
 * - 3.2（默认 claude 会话）：claude 驱动不暴露 configOptions、模型面为空 →
 *   SetModel 得到驱动的 NON-terminal 错误 → 会话存活、模型不变（7f1d7c9 起
 *   不再 teardown）。
 * - 本组新增用例（`fakeacp` agent 会话）：沙箱通过
 *   `[acp.agents.fakeacp]`（generic-ACP 驱动 + fake-acp-agent）通告
 *   `ok-model`/`bad-model` 两个模型 id（初值 = 列表首位 bad-model），
 *   并对 `bad-model` 的 set_config_option 回 RPC 错误：
 *   - `ok-model` POST → agent 接受 → ModelChanged → current_model 同步；
 *   - `bad-model` POST → agent typed rejection → 会话存活、current_model 不变。
 * 两条路径共同契约：拒绝不销毁会话、不伪造成功。
 */
import { expect, test } from '@playwright/test'
import http from 'node:http'
import type { AddressInfo } from 'node:net'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  listSessions,
  resetState,
  FocusedSession,
  ProjectRail,
  SettingsModal,
  waitStatus,
} from './helpers/index'

test.describe('模型管理覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('无模型诚实缺省', () => {
    test('3.2 set_model on a model-less session fails non-terminally and honestly', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'modeless' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // No picker anywhere (D4 presentation, re-pinned here as the pre-state).
      await expect(detail.modelPick).toHaveCount(0)

      // The webui delivers the command (HTTP 200). The Claude driver answers
      // SetModel with a NON-terminal error (7f1d7c9): the session survives,
      // the model is unchanged, and no fake success is recorded.
      const resp = await page.request.post(`/api/sessions/${key}/model`, {
        data: { model_id: 'no-such-model-3.2' },
      })
      expect(resp.ok()).toBe(true)

      // Session survives the rejected switch: still listed, detail resolvable.
      await expect
        .poll(async () =>
          (await listSessions(page.request)).some((r) => r.encoded_key === key),
        )
        .toBe(true)
      const after = await getSession(page.request, key)
      expect(after.status).toBe(200)
      // Honest absence: no fabricated model surface after the rejection.
      expect(after.detail?.current_model).toBeNull()
      expect(after.detail?.available_models ?? []).toEqual([])

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('有模型面的正向切换（acp:fakeacp，B2.2）', () => {
    test('set_session_model happy path — POST ok-model → ModelChanged → current_model 同步 + 选择器呈现', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, {
        prompt: 'model-switch',
        agent: 'fakeacp',
      })
      // The generic-ACP turn completes its echo response but the session
      // stays `working` (pre-existing: that driver emits no Finished event).
      // Wait only for the model surface to appear after session/new.
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.current_model, {
          timeout: 15_000,
          intervals: [250],
        })
        .toBe('bad-model')
      expect((await getSession(page.request, key)).detail?.available_models).toContain('ok-model')

      // The route is POST (the webui delivers; the agent accepts/rejects
      // over the event stream).
      const resp = await page.request.post(`/api/sessions/${key}/model`, {
        data: { model_id: 'ok-model' },
      })
      expect(resp.ok()).toBe(true)

      // Snapshot syncs via the agent's ModelChanged event.
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.current_model, {
          timeout: 15_000,
          intervals: [250],
        })
        .toBe('ok-model')

      // The UI renders the model picker for this session and shows the new
      // model selected.
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await expect(detail.modelPick).toBeVisible()
      await expect(detail.modelPick).toContainText('ok-model', { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })

    test('set_session_model rejects unknown model — typed rejection 到达但 webui 无内联错误面（observed product gap），会话存活、current_model 不变', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, {
        prompt: 'model-reject',
        agent: 'fakeacp',
      })
      // Same pre-existing no-Finished trait: the model surface appears once
      // session/new lands.
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.current_model, {
          timeout: 15_000,
          intervals: [250],
        })
        .toBe('bad-model')

      const resp = await page.request.post(`/api/sessions/${key}/model`, {
        data: { model_id: 'bad-model' },
      })
      // The webui only delivers (HTTP 200); the AGENT rejects over the event
      // stream — a typed, NON-terminal rejection (vs 3.2's terminal teardown).
      expect(resp.ok()).toBe(true)

      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // Observed product gap (same style as the allow-session journey in
      // permission.spec): the agent's typed rejection arrives as a
      // non-terminal Error event, but the webui has no inline surface for
      // agent-level Error events yet — nothing renders it. The honest
      // contract that IS observable: no fake success (the model truth does
      // NOT move) and the session survives untouched.
      await expect
        .poll(async () => (await listSessions(page.request)).some((r) => r.encoded_key === key), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(true)
      const after = await getSession(page.request, key)
      expect(after.status).toBe(200)
      expect(after.detail?.current_model).toBe('bad-model')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('settings provider 编辑与编辑器内抓取（redesign-provider-models-settings + revamp-settings-nav-and-models-editor）', () => {
    test('3.2 providers are editable: model entries with capability tags persist; browsing stays probe-free', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      // Seed a custom provider over the API; the UI edit journey then owns it.
      const created = await page.request.post('/router/api/providers', {
        data: { name: 'editable', base_url_openai_chat: 'http://127.0.0.1:9/v1' },
      })
      expect(created.ok()).toBe(true)

      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Models')

      // Browsing the list stays read-only on the network: zero fetch traffic
      // (each fetch dials the provider's upstream from core).
      let fetchCalls = 0
      await page.route('**/probe*', (route) => {
        fetchCalls += 1
        void route.continue()
      })

      const row = settings.panel.locator('.provider-row', { hasText: 'editable' })
      await expect(row).toBeVisible({ timeout: 10_000 })
      // revamp…4.1: the provider row no longer carries a fetch button.
      await expect(row.locator('button[data-testid="fetch-models"]')).toHaveCount(0)

      // Model entries are editable: add two entries, one tagged vision.
      await row.locator('button[title="Edit"]').click()
      const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      const add = editor.locator('button[data-testid="add-model-entry"]')
      await expect(add).toBeVisible()
      await add.click()
      const entry0 = editor.locator('[data-testid="model-entry"]').first()
      await entry0.locator('wa-input input').fill('entry-a')
      await entry0.locator('input[data-testid="tag-vision"]').check()
      await add.click()
      const entry1 = editor.locator('[data-testid="model-entry"]').nth(1)
      await entry1.locator('wa-input input').fill('entry-b')
      await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editor).toBeHidden({ timeout: 10_000 })

      // API truth: the entries persisted with their capability tags (entry
      // objects; text implicit and never stored).
      const resp = await page.request.get('/router/api/providers')
      expect(resp.ok()).toBe(true)
      const body = (await resp.json()) as {
        providers?: Array<{ name: string; models: Array<{ id: string; tags: string[] }> }>
      }
      const stored = body.providers?.find((p) => p.name === 'editable')
      expect(stored?.models).toEqual([
        { id: 'entry-a', tags: ['vision'] },
        { id: 'entry-b', tags: [] },
      ])

      // UI parity: the row lists the entries and the vision tag.
      await expect(row.locator('.model-chip', { hasText: 'entry-a' })).toContainText('vision')
      await expect(row.locator('.model-chip', { hasText: 'entry-b' })).toBeVisible()

      // Browsing + editing never dialed an upstream.
      expect(fetchCalls).toBe(0)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('editor fetch replaces the draft wholesale (tags preserved), storage moves only on save; cancel discards', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      // Node-local fake upstream (never a real provider host): serves an
      // openai-style /v1/models envelope to whatever path core dials.
      const upstreamModels = ['fetch-m-1', 'fetch-m-2']
      const upstream = http.createServer((_req, res) => {
        res.setHeader('content-type', 'application/json')
        res.end(
          JSON.stringify({
            object: 'list',
            data: upstreamModels.map((id) => ({ id, object: 'model' })),
          }),
        )
      })
      await new Promise<void>((resolve) => upstream.listen(0, '127.0.0.1', resolve))
      const port = (upstream.address() as AddressInfo).port

      /** Stored catalog entries (ids + tags) for one provider. */
      const storedEntries = async (): Promise<Array<{ id: string; tags: string[] }>> => {
        const resp = await page.request.get('/router/api/providers')
        expect(resp.ok()).toBe(true)
        const body = (await resp.json()) as {
          providers?: Array<{ name: string; models: Array<{ id: string; tags: string[] }> }>
        }
        return body.providers?.find((p) => p.name === 'fetchable')?.models ?? []
      }

      try {
        await resetState(page.request)
        // Ordinary edit path: create a custom provider pointed at the fake
        // upstream (core dials 127.0.0.1 only — sandbox-safe), pre-seeded with
        // one manually tagged entry whose id the upstream also serves.
        const created = await page.request.post('/router/api/providers', {
          data: {
            name: 'fetchable',
            base_url_openai_chat: `http://127.0.0.1:${port}/v1`,
            models: [{ id: 'fetch-m-2', tags: ['vision'] }],
          },
        })
        expect(created.ok()).toBe(true)
        expect(await storedEntries()).toEqual([{ id: 'fetch-m-2', tags: ['vision'] }])

        await page.goto('/')
        await settings.openViaSidebar()
        await settings.openSection('Models')

        const row = settings.panel.locator('.provider-row', { hasText: 'fetchable' })
        await expect(row).toBeVisible({ timeout: 10_000 })

        // ── Cancel-discard phase: fetch replaces the DRAFT only; cancelling
        // the editor leaves the stored catalog untouched.
        await row.locator('button[title="Edit"]').click()
        let editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
        await expect(
          editor.locator('button[data-testid="fetch-models"]'),
        ).toBeVisible({ timeout: 10_000 })
        await editor.locator('button[data-testid="fetch-models"]').click()

        // The draft now lists the upstream ids — the surviving id keeps its
        // manual vision tag, the new one starts text-only.
        const entry0 = editor.locator('[data-testid="model-entry"]').nth(0)
        const entry1 = editor.locator('[data-testid="model-entry"]').nth(1)
        await expect(entry0.locator('wa-input input')).toHaveValue('fetch-m-1', {
          timeout: 10_000,
        })
        await expect(entry1.locator('wa-input input')).toHaveValue('fetch-m-2')
        await expect(entry0.locator('input[data-testid="tag-vision"]')).not.toBeChecked()
        await expect(entry1.locator('input[data-testid="tag-vision"]')).toBeChecked()

        // Fetch persists nothing by itself.
        expect(await storedEntries()).toEqual([{ id: 'fetch-m-2', tags: ['vision'] }])

        await editor.locator('wa-button').filter({ hasText: 'Cancel' }).click()
        await expect(editor).toBeHidden({ timeout: 10_000 })
        expect(await storedEntries()).toEqual([{ id: 'fetch-m-2', tags: ['vision'] }])

        // ── Save phase: reopen, fetch again, save through the ordinary edit
        // flow — the wholesale replacement (dedup by id) reaches the store.
        await row.locator('button[title="Edit"]').click()
        editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
        await expect(
          editor.locator('button[data-testid="fetch-models"]'),
        ).toBeVisible({ timeout: 10_000 })
        await editor.locator('button[data-testid="fetch-models"]').click()
        // fetch-m-1 只在抓取完成后出现在草稿里（存量目录只有 fetch-m-2）——
        // 以它为同步锚点。
        await expect(editor.locator('[data-testid="model-entry"]').nth(0).locator('wa-input input')).toHaveValue(
          'fetch-m-1',
          { timeout: 10_000 },
        )
        await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
        await expect(editor).toBeHidden({ timeout: 10_000 })
        await expect.poll(storedEntries, { timeout: 10_000, intervals: [250] }).toEqual([
          { id: 'fetch-m-1', tags: [] },
          { id: 'fetch-m-2', tags: ['vision'] },
        ])
      } finally {
        await new Promise<void>((resolve) => upstream.close(() => resolve()))
      }

      expect(collector.clean()).toEqual([])
    })

    test('a fetched-and-saved catalog reaches the creation dialog without a restart', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      const rail = new ProjectRail(page)

      // Node-local fake upstream (never a real provider host), same discipline
      // as the editor-fetch journey above.
      const upstreamModels = ['dlg-m-1', 'dlg-m-2']
      const upstream = http.createServer((_req, res) => {
        res.setHeader('content-type', 'application/json')
        res.end(
          JSON.stringify({
            object: 'list',
            data: upstreamModels.map((id) => ({ id, object: 'model' })),
          }),
        )
      })
      await new Promise<void>((resolve) => upstream.listen(0, '127.0.0.1', resolve))
      const port = (upstream.address() as AddressInfo).port

      try {
        await resetState(page.request)
        const { name: projectName } = await ensureSceneProject(page.request)
        // Re-run safety: the provider lives in the SHARED core store.
        await page.request.delete('/router/api/providers/dialog-seam')
        const created = await page.request.post('/router/api/providers', {
          data: { name: 'dialog-seam', base_url_openai_chat: `http://127.0.0.1:${port}/v1` },
        })
        expect(created.ok()).toBe(true)

        // The real Settings write path: editor fetch replaces the draft, save
        // lands it in the core store.
        await page.goto('/')
        await settings.openViaSidebar()
        await settings.openSection('Models')
        const row = settings.panel.locator('.provider-row', { hasText: 'dialog-seam' })
        await expect(row).toBeVisible({ timeout: 10_000 })
        await row.locator('button[title="Edit"]').click()
        const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
        await editor.locator('button[data-testid="fetch-models"]').click()
        await expect(
          editor.locator('[data-testid="model-entry"]').nth(0).locator('wa-input input'),
        ).toHaveValue('dlg-m-1', { timeout: 10_000 })
        await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
        await expect(editor).toBeHidden({ timeout: 10_000 })

        // API truth: the fetched ids persisted through the ordinary save.
        const resp = await page.request.get('/router/api/providers')
        const body = (await resp.json()) as {
          providers?: Array<{ name: string; models: Array<{ id: string }> }>
        }
        expect(body.providers?.find((p) => p.name === 'dialog-seam')?.models.map((m) => m.id)).toEqual(
          upstreamModels,
        )
        await settings.close()

        // The seam: WITHOUT any reload the creation dialog's catalog (same
        // shared store) offers the fetched provider → models — the dialog
        // reflects catalog changes live (spec: reflected without a restart).
        await rail.expandProject(projectName)
        await rail.openNewSessionDialog(projectName)
        const dialog = rail.newSessionDialog()
        const providerSelect = dialog.locator('[data-testid="dialog-provider-select"]')
        await expect(providerSelect).toBeVisible({ timeout: 15_000 })
        await expect(providerSelect).toContainText('dialog-seam')
        await expect(dialog.locator('[data-testid="dialog-model-select"]')).toContainText(
          'dlg-m-1',
        )
        await rail.cancelNewSessionDialog()

        // Hygiene: drop the provider so later journeys' honest "no provider
        // configured" assertions stay valid.
        const removed = await page.request.delete('/router/api/providers/dialog-seam')
        expect(removed.ok()).toBe(true)
      } finally {
        await new Promise<void>((resolve) => upstream.close(() => resolve()))
      }

      expect(collector.clean()).toEqual([])
    })

    test('a provider without any usable base URL renders no fetch entry in its editor', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const created = await page.request.post('/router/api/providers', {
        data: { name: 'urlless' },
      })
      expect(created.ok()).toBe(true)

      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Models')

      const row = settings.panel.locator('.provider-row', { hasText: 'urlless' })
      await expect(row).toBeVisible({ timeout: 10_000 })
      await row.locator('button[title="Edit"]').click()
      const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      await expect(editor.locator('wa-button').filter({ hasText: 'Save' })).toBeVisible()
      await expect(editor.locator('button[data-testid="fetch-models"]')).toHaveCount(0)
      await editor.locator('wa-button').filter({ hasText: 'Cancel' }).click()
      await expect(editor).toBeHidden({ timeout: 10_000 })
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('agent 不可变的锁定提示（workbench-agent-wire-fix）', () => {
    test('detail head shows the bound agent with the lock affordance', async ({ page }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'lock check' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // Agent 不可变的 UI 承诺：🔒 + tooltip（spec scenario "UI communicates
      // immutability"）；不渲染任何 agent 切换控件。（session-detail 视图已
      // 退休——锁提示现在住在工作台的聚焦会话头里，3.3。）
      // 会话头与跟随模式 composer 各有一枚锁提示——限定会话头那枚。
      const lock = page.locator('sebas-dashboard .session-head [data-testid="agent-lock"]')
      await expect(lock).toContainText('🔒')
      await expect(lock).toHaveAttribute('title', /immutable — chosen when the session was created/)

      expect(collector.clean()).toEqual([])
    })
  })

})