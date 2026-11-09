/**
 * Journey SV.x — settings Services 分区在 watchdog 形载荷下的行契约
 * (status-driven-service-rows)。
 *
 * 套件沙箱（9899）按设计是裸 core（无 watchdog adapter）——settings.spec.ts
 * 的 S1/S6 在那里钉「无 watchdog 控制面」横幅的诚实退化。本 spec 钉的是
 * 行契约的另一侧：`page.route` 安装一份 watchdog 形的真实载荷
 * （adapter_ok: true + core/webui/router/im，并故意混入后端已不再产生的
 * watchdog/updater 泄漏行），所有断言都是真实浏览器里的 DOM/布局断言：
 *   - SV1 只渲染受管行：即使响应泄漏 watchdog/updater 合成行也不渲染
 *     （渲染侧兜底，与后端列表收窄互为纵深）；
 *   - SV2 动作按钮随 actual status 互斥（running 只停止；stopped/disabled 只
 *     启用；starting/restarting 及未知 status 不渲染可点动作；degraded/
 *     failed-startup 显停止+重启）；
 *   - SV3 core 行纯只读（零按钮，连重启也没有）；
 *   - SV4 行布局对齐（真实布局：名字/状态列同 x、动作区右缘对齐）；
 *   - SV5 router 停止被拒（active_routed_sessions + 计数）→ 强制停止出口，
 *     Force stop 以 force: true 重发（网络层记录请求体）且列表刷新。
 *
 * 选择器纪律：React 面没有 Lit 的 `.service-*` class 结构，行按可见内部名
 * （`<strong>` 文本）锚定，动作按 accessible name 锚定；旧实现里的状态圆点
 * 与过渡占位若有对应可访问面则按其断言，否则断言「不产生可点动作」的
 * 行为面（点击副作用为零），而不回退到已退役的 Lit 标记。
 */
import { expect, test, type Locator, type Page, type Route } from '@playwright/test'
import { ErrorCollector, SettingsModal } from './helpers/index'

interface ServiceRow {
  name: string
  status: string
  desired: string
  uptime_secs: number | null
}

/** 安装 /api/admin/services(+events) 拦截；返回当前载荷句柄与 mutation 记录。 */
function interceptServices(page: Page, initial: ServiceRow[]) {
  const state = {
    services: initial,
    disableCalls: [] as (Record<string, unknown> | null)[],
  }
  const serve = (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ adapter_ok: true, services: state.services }),
    })
  void page.route('**/api/admin/services', (route) =>
    route.request().method() === 'GET' ? serve(route) : route.fallback(),
  )
  void page.route('**/api/admin/events', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ adapter_ok: true, events: [] }),
    }),
  )
  void page.route('**/api/admin/services/router/disable', async (route) => {
    const raw = route.request().postDataJSON() as Record<string, unknown> | null
    // 无 body 的 POST 解析为 {}（非 null）——归一化成 force 视角下的形态。
    const body = raw && Object.keys(raw).length > 0 ? raw : null
    state.disableCalls.push(body)
    if (body && body.force === true) {
      // 强制停止放行：翻转载荷（下次 GET 反映 stopped），与真实 watchdog
      // 的停止后刷新形态一致。
      state.services = state.services.map((s) =>
        s.name === 'router' ? { ...s, status: 'stopped' } : s,
      )
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ status: 'accepted', operation_id: 'op-force', message: 'accepted' }),
      })
      return
    }
    // 首次（无 force）拒绝：活跃 routed 会话计数载荷（wire 合同）。
    await route.fulfill({
      status: 400,
      contentType: 'application/json',
      body: JSON.stringify({
        error: 'router has active routed sessions',
        code: 'active_routed_sessions',
        count: 3,
      }),
    })
  })
  return state
}

/** 打开 Settings → Services 并等行渲染完成。 */
async function openServices(settings: SettingsModal): Promise<void> {
  await settings.page.goto('/')
  await settings.openViaSidebar()
  await settings.openSection('服务')
}

/**
 * 按内部名锚定一行：React 行的名字是 `<strong>{name}</strong>`，其父容器
 * 同时承载 meta 文案，祖父节点即整行（名字 + 动作）。
 */
const rowById = (settings: SettingsModal, id: string): Locator =>
  settings.panel.getByText(id, { exact: true }).locator('xpath=../..')

/** 行内动作按钮（AntD 双汉字自动插空格，用正则容纳）。 */
const action = (row: Locator, verb: '停用' | '启用' | '重启'): Locator =>
  row.getByRole('button', { name: new RegExp(verb.split('').join('\\s*')) })

test.describe('设置面 Services 分区', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('受管行与合成行', () => {
    test('SV1 renders managed rows only — leaked watchdog/updater entries are not rendered', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 60 },
        { name: 'webui', status: 'running', desired: 'enabled', uptime_secs: 61 },
        { name: 'router', status: 'running', desired: 'enabled', uptime_secs: 62 },
        { name: 'im', status: 'disabled', desired: 'disabled', uptime_secs: null },
        // 后端已不再产生（status-driven-service-rows 1.1）；渲染侧兜底必须丢掉。
        { name: 'watchdog', status: 'running', desired: 'enabled', uptime_secs: null },
        { name: 'updater', status: 'idle', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      // React 行的内部名是行首的唯一 <strong>：四个受管行、顺序稳定。
      const names = settings.panel.locator('strong')
      await expect(names).toHaveCount(4, { timeout: 10_000 })
      const ids = (await names.allTextContents()).map((s) => s.trim())
      expect(ids).toEqual(['core', 'webui', 'router', 'im'])
      const text = (await settings.panel.textContent()) ?? ''
      expect(text).not.toContain('watchdog')
      expect(text).not.toContain('updater')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('动作按钮随 actual status 互斥', () => {
    test('SV2 running 只 stopped 动作；stopped 只启用；starting 不提供可点动作', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      const state = interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 1 },
        { name: 'webui', status: 'running', desired: 'enabled', uptime_secs: 2 },
        { name: 'router', status: 'stopped', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'starting', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      const webui = rowById(settings, 'webui')
      const router = rowById(settings, 'router')
      const im = rowById(settings, 'im')
      await expect(webui).toBeVisible({ timeout: 10_000 })

      // running：只停止；不得出现启用。
      await expect(action(webui, '停用')).toHaveCount(1)
      await expect(action(webui, '启用')).toHaveCount(0)
      // stopped：只启用。
      await expect(action(router, '启用')).toHaveCount(1)
      await expect(action(router, '停用')).toHaveCount(0)
      // starting：过渡态不提供可点动作——点击不得发任何请求。
      await expect(im.getByRole('button')).toHaveCount(0)
      await settings.page.waitForTimeout(300)
      expect(state.disableCalls).toEqual([])
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('SV2b degraded/failed-startup 显停止+重启；disabled 只启用', async ({ page }) => {
      const settings = new SettingsModal(page)
      interceptServices(page, [
        { name: 'webui', status: 'degraded', desired: 'enabled', uptime_secs: null },
        { name: 'router', status: 'failed-startup', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'disabled', desired: 'disabled', uptime_secs: null },
      ])

      await openServices(settings)
      const webui = rowById(settings, 'webui')
      const router = rowById(settings, 'router')
      const im = rowById(settings, 'im')
      await expect(webui).toBeVisible({ timeout: 10_000 })

      // degraded / failed-startup：停止 + 重启两钮（旧 ▮+⟳ 的语义面）。
      for (const row of [webui, router]) {
        await expect(action(row, '停用')).toHaveCount(1)
        await expect(action(row, '重启')).toHaveCount(1)
      }
      // disabled：只启用。
      await expect(action(im, '启用')).toHaveCount(1)
      await expect(action(im, '停用')).toHaveCount(0)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('SV2c restarting 与未知 status 都不提供可点动作（fail-safe）', async ({ page }) => {
      const settings = new SettingsModal(page)
      interceptServices(page, [
        { name: 'webui', status: 'restarting', desired: 'enabled', uptime_secs: null },
        { name: 'router', status: 'some-future-state', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      for (const id of ['webui', 'router']) {
        const row = rowById(settings, id)
        await expect(row).toBeVisible({ timeout: 10_000 })
        await expect(row.getByRole('button')).toHaveCount(0)
      }
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('core 只读与纵向对齐', () => {
    test('SV3/SV4 core row renders zero buttons; name column and action edge align across rows', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      // 三种动作区形态并置：core 只读、running 单钮、stopped 单钮。
      interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 10 },
        { name: 'webui', status: 'running', desired: 'enabled', uptime_secs: 11 },
        { name: 'router', status: 'stopped', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'starting', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      const core = rowById(settings, 'core')
      await expect(core).toBeVisible({ timeout: 10_000 })

      // SV3：core 行零按钮（连重启也没有），只读语义可见。
      await expect(core.getByRole('button')).toHaveCount(0)
      await expect(core.getByText('只读', { exact: true })).toBeVisible()

      // SV4：真实布局断言（公共语义可表达的部分）——四行的内部名同 x，
      // 状态文案（desired/actual）同 x；有动作的行其动作区右缘对齐。
      const rows = ['core', 'webui', 'router', 'im'].map((id) => rowById(settings, id))
      const nameXs: number[] = []
      const metaXs: number[] = []
      for (const row of rows) {
        const name = await row.locator('strong').boundingBox()
        const meta = await row.getByText(/desired=/, { exact: false }).boundingBox()
        expect(name).not.toBeNull()
        expect(meta).not.toBeNull()
        nameXs.push(name!.x)
        metaXs.push(meta!.x)
      }
      const sameX = (vals: number[]) => Math.max(...vals) - Math.min(...vals) < 1
      expect(sameX(nameXs)).toBe(true)
      expect(sameX(metaXs)).toBe(true)
      // 动作区（最右按钮）右缘对齐：core 行没有按钮，跳过。
      const rights: number[] = []
      for (const row of [rows[1]!, rows[2]!]) {
        const buttons = row.getByRole('button')
        await expect(buttons).toHaveCount(1)
        const box = await buttons.last().boundingBox()
        expect(box).not.toBeNull()
        rights.push(box!.x + box!.width)
      }
      expect(sameX(rights)).toBe(true)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('router 停止保护', () => {
    test('SV5 rejected stop opens the force dialog; Force stop resends with force: true', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      const state = interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 1 },
        { name: 'router', status: 'running', desired: 'enabled', uptime_secs: 2 },
      ])

      await openServices(settings)
      const router = rowById(settings, 'router')
      await expect(router).toBeVisible({ timeout: 10_000 })

      // running 行只提供停止：点它 → 首次拒绝（400 + active_routed_sessions）。
      await action(router, '停用').click()
      // 首次请求不带 force。
      await expect.poll(() => state.disableCalls.length).toBe(1)
      expect(state.disableCalls).toEqual([null])

      // 拒绝后进入强制停止出口，计数原文可读。
      const force = page.getByRole('dialog', { name: '强制停止服务' })
      await expect(force).toBeVisible({ timeout: 10_000 })
      await expect(force).toContainText('3')

      // Force stop：以 force: true 重发同一请求 → 放行 → 列表刷新为 stopped。
      await force.getByRole('button', { name: /强制停止/ }).click()
      await expect(force).toBeHidden({ timeout: 10_000 })
      expect(state.disableCalls).toEqual([null, { force: true }])
      await expect(router).toContainText('actual=stopped', { timeout: 10_000 })
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
