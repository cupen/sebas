/**
 * Journey S.x — settings surface (phase-3 tasks S1–S5).
 *
 * 功能：设置面 / 子功能：只读呈现、写降级
 *
 * Read-only sections (Services/About/Env) are reconciled against their JSON
 * API truth with contains-assertions (never literals: listen addrs and
 * uptime move with the sandbox). The sandbox has no SEBAS_CONTROL_SECRET,
 * so every router mutation deterministically answers 503 — the contract
 * under test is honest degradation: the failure surfaces inline
 * (`.callout-error`), server-side lists/defaults stay unchanged, and the
 * dialogs remain interactive. Write persistence is explicitly NOT asserted
 * (needs a control-secret sandbox shape, separate item).
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  getAbout,
  getAgentDefaults,
  getRouterInfo,
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
    test('S1 services cards match /api/router truth', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const truth = await getRouterInfo(page.request)
      await page.goto('/')
      await settings.openViaComposer()
      await settings.openSection('Services')

      const cards = settings.panel.locator('.service-card')
      await expect(cards.first()).toBeVisible({ timeout: 10_000 })
      // Router card: listen addr comes from the API, never a literal.
      await expect(cards.nth(0).locator('.service-desc')).toContainText(truth.listen ?? '—')
      await expect(cards.nth(0).locator('.service-status')).toContainText('Running')
      // Routing card: count/auth/debug mirror the API truth.
      await expect(cards.nth(1).locator('.service-desc')).toContainText(
        `${truth.provider_count} provider(s)`,
      )
      await expect(cards.nth(1).locator('.service-desc')).toContainText(
        `auth ${truth.has_auth ? 'configured' : 'none'} · debug ${truth.debug ? 'on' : 'off'}`,
      )
      await expect(cards.nth(1).locator('.service-status')).toContainText(
        truth.provider_count > 0 ? 'Configured' : 'Idle',
      )
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S2 about table matches /api/about truth', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const truth = await getAbout(page.request)
      await page.goto('/')
      await settings.openViaComposer()
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
      await settings.openViaComposer()
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
  })

  test.describe('写降级', () => {
    test('S4 defaults read parity; sandbox write fails honestly', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const before = await getAgentDefaults(page.request)
      expect(before).toEqual({ provider: null, model: null })
      await page.goto('/')
      await settings.openViaComposer()

      // Toolbar mirrors the null truth (span only — wa-button internals also
      // carry .label slots, so scope structurally).
      await expect(settings.panel.locator('.provider-toolbar span.label')).toHaveText(
        'no default set',
        { timeout: 10_000 },
      )

      // ★ opens the set-default dialog for the first provider row.
      const firstRow = settings.panel.locator('.provider-row').first()
      const providerName = (await firstRow.locator('.provider-row-name').textContent())?.trim()
      expect(providerName).toBeTruthy()
      await firstRow.locator('button[title="Set as default for new sessions"]').click()
      const dialog = page.locator('sebas-settings-modal wa-dialog[label="Set default for new sessions"]')
      await expect(
        dialog.locator('.dialog-text').filter({ hasText: providerName! }),
      ).toBeVisible()
      // Model choice is catalog-dependent: rows without a catalog honestly
      // say so, rows with one offer the select — either branch is legitimate.
      await expect(
        dialog
          .locator('.dialog-text', { hasText: 'This provider has no model catalog yet.' })
          .or(dialog.locator('wa-select[label="Default model"]')),
      ).toBeVisible()

      // Save hits the 503 mutation wall: inline error, truth unchanged,
      // dialog stays interactive (cancellable, not torn down).
      await dialog.locator('wa-button').filter({ hasText: 'Set default' }).click()
      await expect(settings.panel.locator('.callout-error[role="alert"]')).toBeVisible({
        timeout: 10_000,
      })
      expect(await getAgentDefaults(page.request)).toEqual({ provider: null, model: null })
      await expect(
        settings.panel.locator('.provider-toolbar span.label'),
      ).toHaveText('no default set')
      await dialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S5a create/edit mutations: client validation + honest 503', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const providersBefore = await listRouterProviders(page.request)
      await page.goto('/')
      await settings.openViaComposer()

      // Empty name is rejected client-side with zero network traffic.
      let postCalls = 0
      await page.route('**/router/api/providers', (route) => {
        if (route.request().method() === 'POST') postCalls += 1
        void route.continue()
      })
      await settings.panel.locator('wa-button').filter({ hasText: 'New (preset)' }).click()
      const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      // wa-dialog hosts read popover-hidden in the top layer — assert the
      // rendered footer action instead (same discipline as the rail dialogs).
      await expect(editor.locator('wa-button').filter({ hasText: 'Save' })).toBeVisible()
      await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editor.locator('.callout-error[role="alert"]')).toContainText('名称不能为空')
      expect(postCalls).toBe(0)

      // A named create reaches the server and meets the 503 wall honestly.
      const probeName = `spec-probe-${Date.now()}`
      const nameInput = editor.locator('wa-input[label="Name"] input')
      await nameInput.click()
      await nameInput.pressSequentially(probeName)
      await editor.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editor.locator('.callout-error[role="alert"]')).not.toContainText(
        '名称不能为空',
        { timeout: 10_000 },
      )
      await expect(editor.locator('.callout-error[role="alert"]')).toBeVisible()
      expect(await listRouterProviders(page.request)).toEqual(providersBefore)
      await editor.locator('wa-button').filter({ hasText: 'Cancel' }).click()

      // Edit-save on an existing row meets the same wall; the list is untouched.
      await settings.panel
        .locator('.provider-row')
        .first()
        .locator('button[title="Edit"]')
        .click()
      const editDialog = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      await expect(editDialog.locator('wa-button').filter({ hasText: 'Save' })).toBeVisible()
      await editDialog.locator('wa-button').filter({ hasText: 'Save' }).click()
      await expect(editDialog.locator('.callout-error[role="alert"]')).toBeVisible({
        timeout: 10_000,
      })
      expect(await listRouterProviders(page.request)).toEqual(providersBefore)
      await editDialog.locator('wa-button').filter({ hasText: 'Cancel' }).click()
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S5b delete/probe mutations fail honestly, list unchanged', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const providersBefore = await listRouterProviders(page.request)
      expect(providersBefore.length).toBeGreaterThan(0)
      await page.goto('/')
      await settings.openViaComposer()
      await expect(settings.panel.locator('.provider-row').first()).toBeVisible({
        timeout: 10_000,
      })

      // Delete confirm meets the 503 wall: error surfaces, dialog stays open.
      await settings.panel
        .locator('.provider-row')
        .first()
        .locator('button[title="Delete"]')
        .click()
      const confirm = page.locator('sebas-settings-modal wa-dialog[label="Delete provider"]')
      await expect(confirm.locator('.dialog-text')).toBeVisible()
      await confirm.locator('wa-button').filter({ hasText: 'Delete' }).click()
      await expect(settings.panel.locator('.callout-error[role="alert"]')).toBeVisible({
        timeout: 10_000,
      })
      expect(await listRouterProviders(page.request)).toEqual(providersBefore)
      await confirm.locator('wa-button').filter({ hasText: 'Cancel' }).click()

      // Probe meets the same wall: no success result, error surfaces instead.
      await settings.panel
        .locator('.provider-row')
        .first()
        .locator('button[title="Probe model list"]')
        .click()
      await expect(settings.panel.locator('.callout-error[role="alert"]')).toBeVisible({
        timeout: 10_000,
      })
      await expect(settings.panel.locator('.callout[role="status"]')).toHaveCount(0)
      expect(await listRouterProviders(page.request)).toEqual(providersBefore)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
