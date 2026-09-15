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
 *   - SV2 动作按钮随 actual status 互斥（running 只 ■；stopped/disabled 只
 *     ▶；starting/restarting 及未知 status 渲染不可点过渡占位；
 *     degraded/failed-startup 显 ■+⟳）；
 *   - SV3 core 行纯只读（零按钮，连 ⟳ 也没有）；
 *   - SV4 状态列纵向对齐（真实布局：定宽动作区 + 占位填充——各行
 *     .service-status 与 .dot 的 x 一致、.service-actions 等宽）；
 *   - SV5 router 停止被拒（active_routed_sessions + 计数）→ 强制出口对话
 *     框，Force stop 以 force: true 重发（网络层记录请求体）且列表刷新。
 */
import { expect, test, type Page, type Route } from '@playwright/test'
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
  await settings.openSection('Services')
}

/** 精确按内部名锚定一行（避免子串误配，如 uptime 里的 "im"）。 */
const rowById = (settings: SettingsModal, page: Page, id: string) =>
  settings.panel.locator('.service-card', {
    has: page.locator(`.service-id:text-is("${id}")`),
  })

/** 行内按钮文本（trim 掉模板缩进空白），如 ['■'] / ['■', '⟳']。 */
const buttonLabels = async (row: ReturnType<typeof rowById>): Promise<string[]> =>
  (await row.locator('button').allTextContents()).map((t) => t.trim())

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
      const cards = settings.panel.locator('.service-card')
      await expect(cards).toHaveCount(4, { timeout: 10_000 })
      const ids = (await settings.panel.locator('.service-card .service-id').allTextContents()).map(
        (s) => s.trim(),
      )
      expect(ids).toEqual(['core', 'webui', 'router', 'im'])
      const text = (await settings.panel.textContent()) ?? ''
      expect(text).not.toContain('watchdog')
      expect(text).not.toContain('updater')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('动作按钮随 actual status 互斥', () => {
    test('SV2 running 只 ■；stopped 只 ▶；starting 过渡占位不可点', async ({ page }) => {
      const settings = new SettingsModal(page)
      const state = interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 1 },
        { name: 'webui', status: 'running', desired: 'enabled', uptime_secs: 2 },
        { name: 'router', status: 'stopped', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'starting', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      const webui = rowById(settings, page, 'webui')
      const router = rowById(settings, page, 'router')
      const im = rowById(settings, page, 'im')
      await expect(webui).toBeVisible({ timeout: 10_000 })

      // running：只 ■。
      expect(await buttonLabels(webui)).toEqual(['■'])
      await expect(webui.locator('button[title="Disable service"]')).toHaveCount(1)
      await expect(webui.locator('button[title="Enable service"]')).toHaveCount(0)
      // stopped：只 ▶。
      expect(await buttonLabels(router)).toEqual(['▶'])
      await expect(router.locator('button[title="Enable service"]')).toHaveCount(1)
      // starting：过渡占位（span，非按钮）——点击不发任何请求。
      await expect(im.locator('button')).toHaveCount(0)
      const placeholder = im.locator('.service-actions .service-transition')
      await expect(placeholder).toHaveCount(1)
      await expect(placeholder).toHaveAttribute('aria-hidden', 'true')
      await placeholder.click()
      await settings.page.waitForTimeout(300)
      expect(state.disableCalls).toEqual([])
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('SV2b degraded/failed-startup 显 ■+⟳；disabled 只 ▶', async ({ page }) => {
      const settings = new SettingsModal(page)
      interceptServices(page, [
        { name: 'webui', status: 'degraded', desired: 'enabled', uptime_secs: null },
        { name: 'router', status: 'failed-startup', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'disabled', desired: 'disabled', uptime_secs: null },
      ])

      await openServices(settings)
      const webui = rowById(settings, page, 'webui')
      const router = rowById(settings, page, 'router')
      const im = rowById(settings, page, 'im')
      await expect(webui).toBeVisible({ timeout: 10_000 })

      for (const row of [webui, router]) {
        expect(await buttonLabels(row)).toEqual(['■', '⟳'])
        await expect(row.locator('button[title="Restart service"]')).toHaveCount(1)
      }
      expect(await buttonLabels(im)).toEqual(['▶'])
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('SV2c restarting 与未知 status 都渲染过渡占位（fail-safe）', async ({ page }) => {
      const settings = new SettingsModal(page)
      interceptServices(page, [
        { name: 'webui', status: 'restarting', desired: 'enabled', uptime_secs: null },
        { name: 'router', status: 'some-future-state', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      for (const id of ['webui', 'router']) {
        const row = rowById(settings, page, id)
        await expect(row).toBeVisible({ timeout: 10_000 })
        await expect(row.locator('button')).toHaveCount(0)
        await expect(row.locator('.service-actions .service-transition')).toHaveCount(1)
      }
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('core 只读与纵向对齐', () => {
    test('SV3/SV4 core row renders zero buttons; status dots align to one x across rows', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)
      // 三种动作区形态并置：core 只读、running 两钮位（■ 单钮）、过渡占位。
      interceptServices(page, [
        { name: 'core', status: 'running', desired: 'enabled', uptime_secs: 10 },
        { name: 'webui', status: 'running', desired: 'enabled', uptime_secs: 11 },
        { name: 'router', status: 'stopped', desired: 'enabled', uptime_secs: null },
        { name: 'im', status: 'starting', desired: 'enabled', uptime_secs: null },
      ])

      await openServices(settings)
      const core = rowById(settings, page, 'core')
      await expect(core).toBeVisible({ timeout: 10_000 })

      // SV3：core 行零按钮（连 ⟳ 也没有）；动作区容器仍渲染（定宽占位）。
      await expect(core.locator('button')).toHaveCount(0)
      await expect(core.locator('.service-actions')).toHaveCount(1)

      // SV4：真实布局断言——四行的状态列与状态圆点落在同一 x，动作区等宽。
      const rows = ['core', 'webui', 'router', 'im'].map((id) => rowById(settings, page, id))
      const xs: number[] = []
      const dots: number[] = []
      const widths: number[] = []
      for (const row of rows) {
        const status = await row.locator('.service-status').boundingBox()
        const dot = await row.locator('.service-status .dot').boundingBox()
        const actions = await row.locator('.service-actions').boundingBox()
        expect(status).not.toBeNull()
        expect(dot).not.toBeNull()
        expect(actions).not.toBeNull()
        xs.push(status!.x)
        dots.push(dot!.x)
        widths.push(actions!.width)
      }
      const sameX = (vals: number[]) => Math.max(...vals) - Math.min(...vals) < 1
      expect(sameX(xs)).toBe(true)
      expect(sameX(dots)).toBe(true)
      // 动作区定宽（--service-actions-w = 56px）：core 只读行与占位行同宽。
      expect(sameX(widths)).toBe(true)
      for (const w of widths) {
        expect(Math.abs(w - 56)).toBeLessThan(1)
      }
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
      const router = rowById(settings, page, 'router')
      await expect(router).toBeVisible({ timeout: 10_000 })

      // running 行只 ■：点它 → confirm 弹窗（不预查会话数，拒绝驱动）。
      await router.locator('button[title="Disable service"]').click()
      const confirm = page.locator('sebas-settings-modal wa-dialog.service-action-confirm')
      await expect(confirm.locator('.dialog-text')).toBeVisible()
      await confirm.locator('wa-button[variant="danger"]').click()

      // 拒绝（400 + active_routed_sessions + 3）→ 二层强制出口对话框。
      const force = page.locator('sebas-settings-modal wa-dialog.service-force-stop')
      await expect(force.locator('.dialog-text')).toContainText('3', { timeout: 10_000 })
      // 首次请求不带 force。
      expect(state.disableCalls).toEqual([null])

      // Force stop：以 force: true 重发同一请求 → 放行 → 列表刷新为 stopped。
      await force.locator('wa-button[variant="danger"]').click()
      await expect(force).toBeHidden({ timeout: 10_000 })
      expect(state.disableCalls).toEqual([null, { force: true }])
      await expect(
        router.locator('.service-desc'),
      ).toContainText('status stopped', { timeout: 10_000 })
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
