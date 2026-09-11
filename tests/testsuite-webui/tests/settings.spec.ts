/**
 * Journey S.x — settings surface (phase-3 tasks S1–S6, new IA
 * fix-settings-menu-and-services-semantics; S5 rewritten for
 * make-core-own-provider-data 5.3 and again for
 * redesign-provider-models-settings 5.1).
 *
 * 功能：设置面 / 子功能：只读呈现、provider 管理旅程（core store 承载）
 *
 * New IA: default section Settings (overview shell), Services reads the
 * watchdog managed-service surface (/api/admin/services, response-driven —
 * the sandbox assembly is a variable, never enumerate concrete services).
 * Since redesign-provider-models-settings the Models section carries provider
 * management ONLY (no /api/router gateway card); router runtime state lives
 * in Services. Read-only sections are reconciled against their JSON API truth
 * with contains-assertions (never literals: listen addrs and uptime move with
 * the sandbox). Since make-core-own-provider-data the provider management
 * cluster (/router/api/providers*) is fulfilled by the webui backend from
 * the core-owned provider store: in this sandbox the core is live, so
 * create/edit/delete persist for real (S5a/S5b). The minimal preset/custom
 * forms follow redesign-provider-models-settings: preset create needs only
 * the preset + key (instance name defaults to the preset name); custom create
 * needs name + one base URL + protocol. Since add-fetch-models the
 * probe/fetch entry is live for providers with a usable base URL (covered in
 * models.spec.ts); S5b pins the honest no-entry rendering for a provider
 * without any base URL (the router proxy stays retired). The sandbox is bare
 * core (no
 * watchdog adapter), so /api/admin/services answers
 * `{adapter_ok: false, services: []}` — S1 pins the response-driven
 * reconciliation (empty-truth branch) and S6 pins the no-adapter banner +
 * disabled restart.
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  getAbout,
  getAdminServices,
  listRouterProviders,
  resetState,
  SettingsModal,
} from './helpers/index'

test.describe('设置面', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('只读呈现', () => {
    test('S1 services rows match /api/admin/services truth (response-driven)', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const truth = await getAdminServices(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Services')

      if (!truth.adapter_ok) {
        // Bare-core sandbox: honest degradation — banner, zero rows.
        await expect(settings.panel.locator('.services-banner')).toContainText(
          '无 watchdog 控制面',
          { timeout: 10_000 },
        )
        await expect(settings.panel.locator('.service-card')).toHaveCount(0)
      } else {
        // Watchdog assembly: two-way reconciliation — every rendered row
        // name is inside the response set, every response row is rendered.
        const cards = settings.panel.locator('.service-card')
        await expect(cards.first()).toBeVisible({ timeout: 10_000 })
        expect(await cards.count()).toBe(truth.services.length)
        for (const svc of truth.services) {
          await expect(cards.filter({ hasText: svc.name }).first()).toBeVisible()
        }
        const names = new Set(truth.services.map((s) => s.name))
        for (const id of await settings.panel
          .locator('.service-card .service-id')
          .allTextContents()) {
          expect(names.has(id.trim())).toBe(true)
        }
      }
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S2 about table matches /api/about truth', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const truth = await getAbout(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('About')

      const list = settings.panel.locator('dl.about-list')
      await expect(list).toBeVisible({ timeout: 10_000 })
      const row = (label: string) => list.locator('.kv', { hasText: label }).locator('dd')
      await expect(row('Version')).toContainText(truth.version)
      await expect(row('Providers')).toContainText(String(truth.provider_count))
      await expect(row('Router listen')).toContainText(truth.router_listen ?? '—')
      // Toolchain is empty in dev builds (an empty dd reads hidden) — pin
      // the row's attachment, not its visibility or value.
      await expect(row('Rust toolchain')).toBeAttached()
      await expect(row('Uptime')).not.toBeEmpty()
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S3 env table renders placeholder semantics', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Env')

      const table = settings.panel.locator('table.env-table')
      await expect(table).toBeVisible({ timeout: 10_000 })
      const row = table.locator('tr', { hasText: 'SEBAS_WEBUI_PASSWORD' })
      await expect(row).toBeVisible()
      // Placeholder semantics: presence is documented, values are not leaked.
      await expect(row.locator('.value')).toHaveText('managed by core config')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S6 bare core degrades: no-adapter banner, no rows, restart disabled', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      // Pin the precondition: the suite sandbox is bare core (no watchdog).
      const truth = await getAdminServices(page.request)
      expect(truth.adapter_ok).toBe(false)
      await page.goto('/')
      await settings.openViaSidebar()

      // Services: banner, zero rows, zero row actions.
      await settings.openSection('Services')
      await expect(settings.panel.locator('.services-banner')).toContainText(
        '无 watchdog 控制面',
        { timeout: 10_000 },
      )
      await expect(settings.panel.locator('.service-card')).toHaveCount(0)
      await expect(settings.panel.locator('.service-actions button')).toHaveCount(0)

      // Back to the Settings overview: the probe has now run, so the
      // restart-all action is disabled with the honest tooltip while the
      // adapter-independent reset stays enabled.
      await settings.openSection('Settings')
      const restart = settings.panel.locator('wa-button', { hasText: '全部进程重启' })
      await expect(restart).toHaveAttribute('disabled', '', { timeout: 10_000 })
      await expect(restart).toHaveAttribute('title', '无 watchdog 控制面')
      const reset = settings.panel.locator('wa-button', { hasText: '重置 Settings' })
      await expect(reset).not.toHaveAttribute('disabled', '')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('写降级', () => {
    test('S4 defaults read parity; set-default stays local, provider seeded via API', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      // workbench-agent-wire-fix 3.3：/api/agent-defaults 端点退役——读回
      // 404，诚实性改由 UI 的「no default set」与写降级路径共同承担。
      const gone = await page.request.get('/api/agent-defaults')
      expect(gone.status()).toBe(404)
      // make-core-own-provider-data：provider 列表来自 core 状态库（空库
      // 起）——为 default-dialog 旅程经 API 预置一行。
      const seededName = `spec-default-${Date.now()}`
      const seeded = await page.request.post('/router/api/providers', {
        data: { name: seededName, preset: 'deepseek', api_key: 'sk-spec' },
      })
      expect(seeded.status()).toBe(201)
      await page.goto('/')
      await settings.openViaSidebar()
      // New IA default section is Settings — defaults live under Models.
      await settings.openSection('Models')

      // Toolbar mirrors the null truth (span only — wa-button internals also
      // carry .label slots, so scope structurally).
      await expect(settings.panel.locator('.provider-toolbar span.label')).toHaveText(
        'no default set',
        { timeout: 10_000 },
      )

      // ★ opens the set-default dialog for the seeded provider row.
      const firstRow = settings.panel
        .locator('.provider-row')
        .filter({ hasText: seededName })
      const providerName = (await firstRow.locator('.provider-row-name').textContent())?.trim()
      expect(providerName).toBeTruthy()
      await firstRow.locator('button[title="Set as default for new sessions"]').click()
      const dialog = page.locator('sebas-settings-modal wa-dialog[label="Set default for new sessions"]')
      await expect(
        dialog.locator('.dialog-text').filter({ hasText: seededName }),
      ).toBeVisible()
      // Model choice is catalog-dependent: rows without a catalog honestly
      // say so, rows with one offer the select — either branch is legitimate.
      await expect(
        dialog
          .locator('.dialog-text', { hasText: 'This provider has no model catalog yet.' })
          .or(dialog.locator('wa-select[label="Default model"]')),
      ).toBeVisible()

      // workbench-agent-wire-fix 3.3：全局默认无服务端持久化面（端点已退
      // 录）。Save 只更新本地呈现（toolbar 反映新默认），wire 上无请求——
      // 默认 agent 的持久化由项目级 default_agent 承载。
      await dialog.locator('wa-button').filter({ hasText: 'Set default' }).click()
      await expect(
        settings.panel.locator('.provider-toolbar span.label'),
      ).toHaveText(`default: ${seededName.split(' ')[0]}`, { timeout: 10_000 })
      expect((await page.request.get('/api/agent-defaults')).status()).toBe(404)
      // 确认即收（defaultDraft 清空 = dialog 关闭）；无需再点 Cancel。
      await expect(dialog).toBeHidden({ timeout: 10_000 })
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S5a create/edit journeys persist through the core store (minimal forms)', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      // New IA default section is Settings — provider management lives under Models.
      await settings.openSection('Models')

      // Empty custom create is rejected client-side with zero network traffic
      // (custom minimal form requires the instance name).
      let postCalls = 0
      await page.route('**/router/api/providers', (route) => {
        if (route.request().method() === 'POST') postCalls += 1
        void route.continue()
      })
      await settings.panel.locator('wa-button').filter({ hasText: 'New (custom)' }).click()
      const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      // wa-dialog hosts read popover-hidden in the top layer — assert the
      // rendered footer action instead (same discipline as the rail dialogs).
      await expect(editor.locator('wa-button').filter({ hasText: 'Save' })).toBeVisible()
      await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editor.locator('.callout-error[role="alert"]')).toContainText('名称不能为空')
      expect(postCalls).toBe(0)

      // A named custom create now SUCCEEDS with the minimal input (name +
      // one base URL; the payload carries no advanced-only fields). Dialog
      // closes, list refreshes (make-core-own-provider-data 3.2).
      const probeName = `spec-create-${Date.now()}`
      const nameInput = editor.locator('wa-input[label="Name"] input')
      await nameInput.click()
      await nameInput.pressSequentially(probeName)
      await editor.locator('wa-input[label="Base URL (OpenAI-compatible)"] input').fill('http://127.0.0.1:9/v1')
      await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editor).toBeHidden({ timeout: 10_000 })
      // API truth: the new row is there immediately (no restart).
      await expect
        .poll(async () => (await listRouterProviders(page.request)).includes(probeName))
        .toBe(true)
      // UI parity: the row renders.
      await expect(
        settings.panel.locator('.provider-row').filter({ hasText: probeName }),
      ).toBeVisible({ timeout: 10_000 })

      // Persistence: the store is core-owned — a fresh page load still sees it.
      await page.reload()
      await settings.openViaSidebar()
      await settings.openSection('Models')
      await expect(
        settings.panel.locator('.provider-row').filter({ hasText: probeName }),
      ).toBeVisible({ timeout: 10_000 })

      // Edit-save on the new row persists over the same seam (empty key keeps
      // the stored key).
      await settings.panel
        .locator('.provider-row')
        .filter({ hasText: probeName })
        .locator('button[title="Edit"]')
        .click()
      const editDialog = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      await expect(editDialog.locator('wa-button').filter({ hasText: 'Save' })).toBeVisible()
      await editDialog.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editDialog).toBeHidden({ timeout: 10_000 })
      await expect(
        settings.panel.locator('.provider-row').filter({ hasText: probeName }),
      ).toBeVisible({ timeout: 10_000 })

      // Preset create needs no name input: the instance is stored under the
      // preset name (redesign-provider-models-settings D5) and its model
      // catalog follows the code table. The editor defaults to the first
      // code-table preset ("anthropic").
      await settings.panel.locator('wa-button').filter({ hasText: 'New (preset)' }).click()
      const presetEditor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      await expect(presetEditor.locator('wa-input[label="Name"]')).toHaveCount(0)
      await presetEditor.locator('wa-input[label="API key"] input').fill('sk-spec-preset')
      await presetEditor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(presetEditor).toBeHidden({ timeout: 10_000 })
      await expect(
        settings.panel.locator('.provider-row').filter({ hasText: 'anthropic' }),
      ).toBeVisible({ timeout: 10_000 })
      const presetResp = await page.request.get('/router/api/providers')
      const presetBody = (await presetResp.json()) as {
        providers?: Array<{ name: string; preset: string | null; models: Array<{ id: string }> }>
      }
      const presetRow = presetBody.providers?.find((p) => p.name === 'anthropic')
      expect(presetRow?.preset).toBe('anthropic')
      // Catalog materializes from the code table as entries (ids + tags).
      expect((presetRow?.models ?? []).map((m) => m.id)).toContain('claude-opus-4-20250514')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S5b delete persists; fetch entry hidden without a base URL', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      // Seed a row over the API so the delete journey owns a known target.
      const seededName = `spec-delete-${Date.now()}`
      const seeded = await page.request.post('/router/api/providers', {
        data: { name: seededName, preset: 'deepseek', api_key: 'sk-spec' },
      })
      expect(seeded.status()).toBe(201)
      await page.goto('/')
      await settings.openViaSidebar()
      // New IA default section is Settings — provider management lives under Models.
      await settings.openSection('Models')
      const seededRow = settings.panel.locator('.provider-row').filter({ hasText: seededName })
      await expect(seededRow).toBeVisible({ timeout: 10_000 })

      // Delete confirm now succeeds: dialog closes, the row disappears from
      // the UI and from the API truth.
      await seededRow.locator('button[title="Delete"]').click()
      const confirm = page.locator('sebas-settings-modal wa-dialog[label="Delete provider"]')
      await expect(confirm.locator('.dialog-text')).toBeVisible()
      await confirm.locator('wa-button').filter({ hasText: 'Delete' }).click()
      await expect(confirm).toBeHidden({ timeout: 10_000 })
      await expect(seededRow).toHaveCount(0)
      await expect
        .poll(async () => (await listRouterProviders(page.request)).includes(seededName))
        .toBe(false)

      // add-fetch-models: the fetch entry exists only for providers with a
      // usable base URL. A URL-less provider renders no entry at all (and the
      // API answers a typed 400 — never a fabricated success).
      await settings.close()
      const urllessName = `spec-nourl-${Date.now()}`
      const urlless = await page.request.post('/router/api/providers', {
        data: { name: urllessName },
      })
      expect(urlless.status()).toBe(201)
      // Reload so the SPA rebuilds and re-reads the provider list fresh.
      await page.reload()
      await settings.openViaSidebar()
      await settings.openSection('Models')
      const urllessRow = settings.panel.locator('.provider-row').filter({ hasText: urllessName })
      await expect(urllessRow).toBeVisible({ timeout: 10_000 })
      await expect(urllessRow.locator('button[data-testid="fetch-models"]')).toHaveCount(0)
      const probe = await page.request.post(
        `/router/api/providers/${encodeURIComponent(urllessName)}/probe`,
      )
      expect(probe.status()).toBe(400)
      const probeBody = (await probe.json()) as { error?: string }
      expect(probeBody.error ?? '').toContain('base URL')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
