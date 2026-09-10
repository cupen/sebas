/**
 * Journey 3.x — model面诚实语义 (phase-2 task 3.2, fallback C +
 * cover-core-channel-test-gaps B2.2).
 *
 * 功能：模型管理覆盖 / 子功能：无模型诚实缺省、settings provider 只读、
 * 有模型面的正向切换与 typed rejection
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
    test('3.2 set_model on a model-less session fails non-terminally and honestly', async ({
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
      const detail = new SessionDetailPage(page)

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
      const detail = new SessionDetailPage(page)

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

  test.describe('settings provider 只读', () => {
    test('3.2 settings provider list matches API, zero probe traffic', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaComposer()
      // New IA default section is Settings — the provider list lives under Models.
      await settings.openSection('Models')

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

  test.describe('agent 不可变的锁定提示（workbench-agent-wire-fix）', () => {
    test('detail head shows the bound agent with the lock affordance', async ({ page }) => {
      const detail = new SessionDetailPage(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'lock check' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // Agent 不可变的 UI 承诺：🔒 + tooltip（spec scenario "UI communicates
      // immutability"）；不渲染任何 agent 切换控件。
      const lock = page.locator('sebas-session-detail [data-testid="agent-lock"]')
      await expect(lock).toContainText('🔒')
      await expect(lock).toHaveAttribute('title', /immutable — chosen when the session was created/)

      expect(collector.clean()).toEqual([])
    })
  })

})