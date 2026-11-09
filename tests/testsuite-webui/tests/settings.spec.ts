/**
 * Journey S.x — settings surface (phase-3 tasks S1–S6; rewritten across
 * fix-settings-menu-and-services-semantics, redesign-provider-models-settings,
 * and revamp-settings-nav-and-models-editor).
 *
 * 功能：设置面 / 子功能：只读呈现、provider 管理旅程（core store 承载）
 *
 * Since revamp-settings-nav-and-models-editor the sections are Generic →
 * Appearance → Services → Models → About: the former Settings overview shell
 * is gone (the read-only items moved into About's INSTANCE segment — since
 * preselect-last-used-model WITHOUT the default provider/model row, whose
 * data source retired to 404; creation preselect is last-used-first now,
 * above BUILD = /api/about; its restart-all / reset maintenance actions are
 * retired — per-service restart lives only in Services). Since
 * split-env-vars-settings-section the former Env table lives in its own
 * read-only「Env Vars」section in the bottom group (above About, pushed to
 * the bottom by the tail separator). Since add-about-build-info the BUILD
 * segment also carries the build-time row (UTC labelled) and a standalone git
 * row (branch@shorthash), reconciled to /api/about truth; missing build info
 * degrades to 'unknown' with the rows kept. Services reads the watchdog managed-service
 * surface (/api/admin/services, response-driven — the sandbox assembly is a
 * variable, never enumerate concrete services). Since
 * redesign-provider-models-settings the Models section carries provider
 * management ONLY (no /api/router gateway card); router runtime state lives
 * in Services. Read-only sections are reconciled against their JSON API truth
 * with contains-assertions (never literals: listen addrs and uptime move with
 * the sandbox). Since make-core-own-provider-data the provider management
 * cluster (/api/providers*) is fulfilled by the webui backend from
 * the core-owned provider store: in this sandbox the core is live, so
 * create/edit/delete persist for real (S5a/S5b). The minimal preset/custom
 * forms follow redesign-provider-models-settings: preset create needs only
 * the preset + key (instance name defaults to the preset name); custom create
 * needs name + one base URL + protocol. The probe/fetch entry lives inside the
 * provider editor (covered in models.spec.ts); S5b pins the honest no-entry
 * rendering for a provider without any base URL (the router proxy stays
 * retired). The sandbox is bare core (no watchdog adapter), so
 * /api/admin/services answers `{adapter_ok: false, services: []}` — S1 pins
 * the response-driven reconciliation (empty-truth branch) and S6 pins the
 * no-adapter banner plus the retirement of the maintenance actions.
 */
import { expect, test } from '@playwright/test'
import {
  cjk,
  ErrorCollector,
  getAbout,
  getAdminServices,
  listRouterProviders,
  pickSelectOption,
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
      await settings.openSection('服务')

      if (!truth.adapter_ok) {
        // Bare-core sandbox: honest degradation — banner, zero rows.
        await expect(settings.panel.getByText(/无 watchdog 控制面/)).toBeVisible({
          timeout: 10_000,
        })
        await expect(settings.panel.locator('strong')).toHaveCount(0)
      } else {
        // Watchdog assembly: two-way reconciliation — every rendered row
        // name is inside the response set, every response row is rendered.
        const rows = settings.panel.locator('strong')
        await expect(rows.first()).toBeVisible({ timeout: 10_000 })
        expect(await rows.count()).toBe(truth.services.length)
        const names = new Set(truth.services.map((s) => s.name))
        for (const name of await rows.allTextContents()) {
          expect(names.has(name.trim())).toBe(true)
        }
      }
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S2 about section shows INSTANCE segment above BUILD, both reconciled to API truth', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      const truth = await getAbout(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('关于')

      // React About 把实例与构建事实合并为一个 dl.facts；逐项对账 /api/about
      // 真值（不锚绝对值：监听地址与运行时随沙箱移动）。
      const facts = settings.panel.locator('dl')
      await expect(facts).toBeVisible({ timeout: 10_000 })
      const row = (label: string) => facts.locator('dt', { hasText: label }).locator('xpath=following-sibling::dd[1]')
      await expect(row('默认 agent')).toContainText(truth.default_agent_kind)
      await expect(row('provider 数')).toContainText(String(truth.provider_count))
      await expect(row('router')).toContainText(truth.router_listen ?? '未监听')
      await expect(row('版本')).toContainText(truth.version)
      await expect(row('运行时长')).not.toBeEmpty()
      await expect(row('构建时间')).toContainText(truth.build_time)
      await expect(row('Git')).toContainText(
        `${truth.git_branch} · ${truth.git_hash}`,
      )
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S3 env table renders placeholder semantics under Env Vars', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      // split-env-vars-settings-section D4：Env 表自 Generic 迁出，住底部
      // 只读组「Env Vars」（与 About 同组，压底分隔线）。
      await settings.openSection('环境变量')

      const table = settings.panel.locator('table')
      await expect(table).toBeVisible({ timeout: 10_000 })
      const row = table.locator('tr', { hasText: 'SEBAS_WEBUI_PASSWORD' })
      await expect(row).toBeVisible()
      // Placeholder semantics (add-webui-multiuser-rbac 后该条目是服务端策划
      // 的 set_unset 敏感项): presence is documented, values are not leaked —
      // 沙箱未设置该变量 → 如实呈现「未设置」，绝不展示/编造值。
      await expect(row.locator('td').nth(2)).toHaveText('未设置')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S6 bare core degrades: no-adapter banner, no rows, no actions — and the retired maintenance actions stay gone', async ({
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
      await settings.openSection('服务')
      await expect(settings.panel.getByText(/无 watchdog 控制面/)).toBeVisible({
        timeout: 10_000,
      })
      await expect(settings.panel.locator('strong')).toHaveCount(0)
      await expect(
        settings.panel.getByRole('button', { name: /停用|启用|重启/ }),
      ).toHaveCount(0)

      // revamp-settings-nav-and-models-editor: the former Settings overview's
      // maintenance actions are retired everywhere — no restart-all, no reset,
      // in any section (per-service restart lives only in Services).
      for (const section of ['通用', '外观', '模型', '关于'] as const) {
        await settings.openSection(section)
        await expect(
          settings.panel.locator('button', { hasText: '全部进程重启' }),
        ).toHaveCount(0)
        await expect(
          settings.panel.locator('button', { hasText: '重置 Settings' }),
        ).toHaveCount(0)
      }
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('分区导航 IA', () => {
    test('nav order Generic→Appearance→Services→Models→Aliases→Agents→Env Vars→About(pinned), default focus, memory, stale-value fallback', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()

      // revamp-settings-nav-and-models-editor：无历史记忆时缺省聚焦 Generic。
      const navItem = (label: string) => settings.panel.getByRole('button', { name: label, exact: true })
      await expect(navItem('通用')).toHaveAttribute('aria-current', 'page', {
        timeout: 10_000,
      })

      // 分区顺序即规约（SETTINGS_SECTIONS）：通用 → 外观 → 服务 → 模型 →
      // 别名 → Agent → 技能 → 环境变量 → 关于。免登录沙箱（role = null）
      // 下 Users 分区按角色裁剪隐藏。React 导航列表没有组间分隔线元素。
      const signature = await settings.panel
        .getByRole('complementary', { name: '设置分区' })
        .locator('nav button')
        .allInnerTexts()
      expect(signature).toEqual([
        '通用',
        '外观',
        '服务',
        '模型',
        '别名',
        'Agent',
        '技能',
        '环境变量',
        '关于',
      ])
      await expect(navItem('用户')).toHaveCount(0)

      // 历史记忆：切到 Models 后关闭再打开，缺省直接回到 Models（无点击）。
      await settings.openSection('模型')
      await settings.close()
      await settings.openViaSidebar()
      await expect(navItem('模型')).toHaveAttribute('aria-current', 'page', {
        timeout: 10_000,
      })
      await expect(navItem('通用')).not.toHaveAttribute('aria-current', 'page')
      await settings.close()

      // 旧值回退：记忆值 `settings`（本变更前的合法分区名）按非法值处理，
      // 重载后打开聚焦 Generic 而非报错或空白。
      await page.evaluate(() => localStorage.setItem('lastSettingsSection', 'settings'))
      await page.reload()
      await settings.openViaSidebar()
      await expect(navItem('通用')).toHaveAttribute('aria-current', 'page', {
        timeout: 10_000,
      })
      await expect(navItem('模型')).not.toHaveAttribute('aria-current', 'page')
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
      const seeded = await page.request.post('/api/providers', {
        data: { name: seededName, preset: 'deepseek', api_key: 'sk-spec' },
      })
      expect(seeded.status()).toBe(201)
      await page.goto('/')
      await settings.openViaSidebar()
      // Default section is Generic — defaults live under Models.
      await settings.openSection('模型')

      // React Models 面板把默认值呈现为只读文案（默认来自 /api/provider-defaults，
      // 新建会话预选走 last-used；旧 set-default 对话框已随蓝图退役）。
      await expect(settings.panel.getByText(/未设置默认|默认：/)).toBeVisible({
        timeout: 10_000,
      })
      // provider 行仍在列表中（真值来自 core store）。
      const firstRow = settings.panel
        .getByText(seededName, { exact: true })
        .locator('xpath=../..')
      await expect(firstRow).toBeVisible({ timeout: 10_000 })
      // 旧的全局默认写入口已退役：面板不提供「设为新建会话的默认」。
      await expect(
        settings.panel.getByRole('button', { name: /设为新建会话的默认|设为默认/ }),
      ).toHaveCount(0)
      // 全局默认无服务端持久化面（端点已退役）。
      expect((await page.request.get('/api/agent-defaults')).status()).toBe(404)
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
      // Default section is Generic — provider management lives under Models.
      await settings.openSection('模型')

      // Empty create is rejected client-side with zero network traffic (the
      // editor requires the instance name).
      let postCalls = 0
      await page.route('**/api/providers', (route) => {
        if (route.request().method() === 'POST') postCalls += 1
        void route.continue()
      })
      await settings.panel.getByRole('button', { name: '新建 provider' }).click()
      const editor = page.getByRole('dialog', { name: '新建 provider' })
      await expect(editor.getByRole('button', { name: cjk('创建') })).toBeVisible()
      await editor.getByRole('button', { name: cjk('创建') }).click()
      await expect(editor.getByRole('alert')).toContainText('provider 名称必填')
      expect(postCalls).toBe(0)

      // A named create now SUCCEEDS with the minimal input (name + one base
      // URL); dialog closes, list refreshes.
      const probeName = `spec-create-${Date.now()}`
      await editor.getByLabel('provider 名称').fill(probeName)
      await editor.getByLabel('OpenAI base URL').fill('http://127.0.0.1:9/v1')
      await editor.getByRole('button', { name: cjk('创建') }).click()
      await expect(editor).toBeHidden({ timeout: 10_000 })
      await expect
        .poll(async () => (await listRouterProviders(page.request)).includes(probeName))
        .toBe(true)
      const probeRow = () =>
        settings.panel.getByText(probeName, { exact: true }).locator('xpath=../..')
      await expect(probeRow()).toBeVisible({ timeout: 10_000 })

      // Persistence: the store is core-owned — a fresh page load still sees it.
      await page.reload()
      await settings.openViaSidebar()
      await settings.openSection('模型')
      await expect(probeRow()).toBeVisible({ timeout: 10_000 })

      // Edit-save on the new row persists over the same seam (empty key keeps
      // the stored key).
      await probeRow().getByRole('button', { name: /编\s*辑/ }).click()
      const editDialog = page.getByRole('dialog', { name: /编辑 provider：/ })
      await expect(editDialog.getByRole('button', { name: cjk('保存') })).toBeVisible()
      await editDialog.getByRole('button', { name: cjk('保存') }).click()
      await expect(editDialog).toBeHidden({ timeout: 10_000 })
      await expect(probeRow()).toBeVisible({ timeout: 10_000 })

      // Preset create: selecting a built-in preset seeds name/base URLs/key env
      // from the code table; saving persists the provider under that name.
      await settings.panel.getByRole('button', { name: '新建 provider' }).click()
      const presetEditor = page.getByRole('dialog', { name: '新建 provider' })
      await pickSelectOption(
        page,
        presetEditor.getByRole('combobox', { name: '内置 preset' }),
        'anthropic',
      )
      await expect(presetEditor.getByLabel('provider 名称')).toHaveValue('anthropic')
      await presetEditor.getByRole('button', { name: cjk('创建') }).click()
      await expect(presetEditor).toBeHidden({ timeout: 10_000 })
      const presetResp = await page.request.get('/api/providers')
      const presetBody = (await presetResp.json()) as {
        providers?: Array<{ name: string; preset: string | null; models: Array<{ id: string }> }>
      }
      const presetRow = presetBody.providers?.find((p) => p.name === 'anthropic')
      expect(presetRow?.preset).toBe('anthropic')
      expect((presetRow?.models ?? []).map((m) => m.id)).toContain('claude-opus-4-20250514')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('S5b delete persists; fetch entry hidden without a base URL', async ({ page }) => {
      const settings = new SettingsModal(page)

      await resetState(page.request)
      // Seed a row over the API so the delete journey owns a known target.
      const seededName = `spec-delete-${Date.now()}`
      const seeded = await page.request.post('/api/providers', {
        data: { name: seededName, preset: 'deepseek', api_key: 'sk-spec' },
      })
      expect(seeded.status()).toBe(201)
      await page.goto('/')
      await settings.openViaSidebar()
      // Default section is Generic — provider management lives under Models.
      await settings.openSection('模型')
      const seededRow = settings.panel
        .getByText(seededName, { exact: true })
        .locator('xpath=../..')
      await expect(seededRow).toBeVisible({ timeout: 10_000 })

      // React provider rows delete directly (no confirm dialog); the row and
      // API truth must converge.
      await seededRow.getByRole('button', { name: /删\s*除/ }).click()
      await expect(seededRow).toHaveCount(0, { timeout: 10_000 })
      await expect
        .poll(async () => (await listRouterProviders(page.request)).includes(seededName))
        .toBe(false)

      // The fetch entry lives inside the provider editor and exists only for
      // providers with a usable base URL. A URL-less provider's editor renders
      // no entry at all (and the API answers a typed 400 — never a fabricated
      // success).
      await settings.close()
      const urllessName = `spec-nourl-${Date.now()}`
      const urlless = await page.request.post('/api/providers', {
        data: { name: urllessName },
      })
      expect(urlless.status()).toBe(201)
      await page.reload()
      await settings.openViaSidebar()
      await settings.openSection('模型')
      const urllessRow = settings.panel
        .getByText(urllessName, { exact: true })
        .locator('xpath=../..')
      await expect(urllessRow).toBeVisible({ timeout: 10_000 })
      await urllessRow.getByRole('button', { name: /编\s*辑/ }).click()
      const urllessEditor = page.getByRole('dialog', { name: /编辑 provider：/ })
      await expect(urllessEditor.getByRole('button', { name: cjk('保存') })).toBeVisible()
      await expect(
        urllessEditor.getByRole('button', { name: '抓取模型列表' }),
      ).toHaveCount(0)
      await urllessEditor.getByRole('button', { name: cjk('取消') }).click()
      await expect(urllessEditor).toBeHidden({ timeout: 10_000 })
      const probe = await page.request.post(
        `/api/providers/${encodeURIComponent(urllessName)}/probe`,
      )
      expect(probe.status()).toBe(400)
      const probeBody = (await probe.json()) as { error?: string }
      expect(probeBody.error ?? '').toContain('base URL')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
