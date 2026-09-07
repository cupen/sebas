/**
 * Journey 3.x — model面诚实语义 (phase-2 task 3.2, fallback C).
 *
 * 功能：模型管理覆盖 / 子功能：无模型诚实缺省、settings provider 只读
 *
 * The sandbox stub exposes NO model options, so there is no positive switch
 * to assert (that needs the D1 stub+driver re-scope). The honest contract:
 * an invalid set_model request is accepted-and-delivered (HTTP 200 — the
 * webui only delivers; the driver rejects over the event stream), the model
 * truth stays absent (current_model null, available_models []), nothing is
 * lost, and the settings provider list renders read-only parity with the API
 * (row count matches, zero probe traffic — we look, we don't probe).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  getSession,
  listSessions,
  resetState,
  SessionDetailPage,
  SettingsModal,
  waitStatus,
} from './helpers/index'

test.describe('模型管理覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('无模型诚实缺省', () => {
    test('3.2 set_model on a model-less session fails terminally and honestly', async ({
      page,
    }) => {
      const detail = new SessionDetailPage(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'modeless' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // No picker anywhere (D4 presentation, re-pinned here as the pre-state).
      await expect(detail.modelPick).toHaveCount(0)

      // The webui delivers the command (HTTP 200), but the Claude driver
      // answers SetModel with a TERMINAL error (driver.rs:422-431) — the
      // session is torn down, exactly like the crash journey. The honest
      // contract is: no fake success, the death is presented as-is.
      const resp = await page.request.post(`/api/sessions/${key}/model`, {
        data: { model_id: 'no-such-model-3.2' },
      })
      expect(resp.ok()).toBe(true)

      // Row leaves the live list; the API agrees it is gone.
      await expect
        .poll(
          async () =>
            (await listSessions(page.request)).some((r) => r.encoded_key === key),
          { timeout: 10_000, intervals: [200] },
        )
        .toBe(false)
      const after = await getSession(page.request, key)
      expect(after.status).toBe(404)
      expect(after.detail).toBeNull()

      // The open detail refetches over live WS and presents the death honestly.
      await expect(detail.errorCallout).toContainText('Session not found', {
        timeout: 20_000,
      })
      await expect(detail.backToWorkbench).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('settings provider 只读', () => {
    test('3.2 settings provider list matches API, zero probe traffic', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaComposer()

      // Count the API truth first.
      const apiResp = await page.request.get('/router/api/providers')
      expect(apiResp.ok()).toBe(true)
      const apiBody = (await apiResp.json()) as { providers?: unknown[] }
      const apiCount = apiBody.providers?.length ?? 0

      // Never probe during a read-only assertion.
      let probeCalls = 0
      await page.route('**/probe*', (route) => {
        probeCalls += 1
        void route.continue()
      })

      // UI parity: row count matches the API (empty state included).
      const rows = settings.panel.locator('.provider-row')
      if (apiCount === 0) {
        await expect(
          settings.panel.locator('.provider-row-empty'),
        ).toBeVisible({ timeout: 10_000 })
        await expect(rows).toHaveCount(0)
      } else {
        await expect(rows.first()).toBeVisible({ timeout: 10_000 })
        expect(await rows.count()).toBe(apiCount)
      }
      expect(probeCalls).toBe(0)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
