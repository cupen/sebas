/**
 * Journey US.x — /usage 视图（add-usage-statistics 5.1 浏览器级呈现）。
 *
 * 套件沙箱（9899）按设计是裸 core（无 router、无 usage 数据）——视图的三种
 * 数据形态用 `page.route` 安装 router 聚合端点的**真实形状载荷**（与
 * services.spec.ts 同款手法），所有断言都是真实浏览器里的 DOM/SVG 断言：
 *   - US1 侧栏「用量」入口进入 /usage，天粒度按模型双折线 + 图例点名 +
 *     窗口汇总数字（spec 场景「day view renders per-model lines」+ 需求
 *     「reachable from the sidebar」）；
 *   - US2 粒度切换 day → hour：重发请求（granularity=hour、无 days）、
 *     24 桶折线重渲染（spec 场景「granularity toggle switches the series」）；
 *   - US3 维度切换零请求：API 一次给全四类明细，切换纯前端（design D4）；
 *   - US4 router 不可达空态：503 + code=router_unreachable → 「router 不可达」
 *     通知带 cause，无图表（spec 场景「router unreachable empty state」）；
 *   - US5 无数据空态：聚合成功全零 → 「暂无用量数据」，与 US4 互斥呈现
 *     （两种空态分开，spec 需求「distinguish the honest empty states」）。
 */
import { expect, test, type Page, type Route } from '@playwright/test'
import { AppShell, ErrorCollector } from './helpers/index'

/** router 聚合载荷的真实形状（sebas-router usage_query Timeseries）。 */
interface UsagePayload {
  granularity: 'day' | 'hour'
  days: number
  tz_offset: number
  buckets: { bucket: string; models: UsageModelRow[] }[]
  totals: UsageModelRow
}

interface UsageModelRow {
  model: string
  requests: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_creation_tokens: number
}

const CLAUDE: UsageModelRow = {
  model: 'claude-sonnet',
  requests: 2,
  input_tokens: 10,
  output_tokens: 50,
  cache_read_tokens: 5,
  cache_creation_tokens: 2,
}
const GPT: UsageModelRow = {
  model: 'gpt-4o-mini',
  requests: 1,
  input_tokens: 4,
  output_tokens: 8,
  cache_read_tokens: 0,
  cache_creation_tokens: 0,
}

/** 天粒度：3 天窗口，中间一天双模型（窗口其余日期零填充——真实端点同款）。 */
const DAY_PAYLOAD: UsagePayload = {
  granularity: 'day',
  days: 3,
  tz_offset: 480,
  buckets: [
    { bucket: '2026-09-27', models: [] },
    { bucket: '2026-09-28', models: [CLAUDE, GPT] },
    { bucket: '2026-09-29', models: [] },
  ],
  totals: {
    model: 'total',
    requests: 3,
    input_tokens: 14,
    output_tokens: 58,
    cache_read_tokens: 5,
    cache_creation_tokens: 2,
  },
}

/** 小时粒度：当天 0–23 全 24 桶，只有 04 时带数据（真实端点同款零填充）。 */
const HOUR_PAYLOAD: UsagePayload = {
  granularity: 'hour',
  days: 1,
  tz_offset: 480,
  buckets: Array.from({ length: 24 }, (_, h) => ({
    bucket: String(h).padStart(2, '0'),
    models: h === 4 ? [CLAUDE, GPT] : [],
  })),
  totals: DAY_PAYLOAD.totals,
}

/** 聚合成功的全零窗口（「暂无用量数据」空态的输入）。 */
const ALL_ZERO_PAYLOAD: UsagePayload = {
  granularity: 'day',
  days: 3,
  tz_offset: 0,
  buckets: [
    { bucket: '2026-09-27', models: [] },
    { bucket: '2026-09-28', models: [] },
    { bucket: '2026-09-29', models: [] },
  ],
  totals: {
    model: 'total',
    requests: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  },
}

/** 安装 /api/usage/timeseries 拦截；返回请求 URL 记录（参数透传断言用）。 */
function interceptUsage(page: Page, payload: UsagePayload) {
  const requests: string[] = []
  const serve = (route: Route) => {
    requests.push(route.request().url())
    void route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(payload),
    })
  }
  void page.route(/\/api\/usage\/timeseries/, (route) =>
    route.request().method() === 'GET' ? serve(route) : route.fallback(),
  )
  return requests
}

/** 安装 503 router_unreachable 拦截（webui 反代不可达形态，webui api.rs 同款体）。 */
function interceptUsageUnreachable(page: Page) {
  const requests: string[] = []
  void page.route(/\/api\/usage\/timeseries/, (route) => {
    requests.push(route.request().url())
    void route.fulfill({
      status: 503,
      contentType: 'application/json',
      body: JSON.stringify({
        error: 'router_unreachable: connection refused',
        code: 'router_unreachable',
        cause: 'router_unreachable',
      }),
    })
  })
  return requests
}

test.describe('用量视图 /usage', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('折线呈现与切换', () => {
    test('US1 sidebar entry opens /usage with per-model lines, legend and window summary', async ({
      page,
    }) => {
      const shell = new AppShell(page)
      const requests = interceptUsage(page, DAY_PAYLOAD)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      // 侧栏「用量」入口（spec：reachable from the sidebar）→ SPA 导航。
      await page.locator('sebas-app .usage-link', { hasText: '用量' }).click()
      await expect(page).toHaveURL(/\/usage$/)
      const view = page.locator('sebas-usage')
      await expect(view).toBeVisible()

      // 请求口径：day 粒度 + 14 天窗口 + 本机时区偏移（分钟东偏）。
      expect(requests).toHaveLength(1)
      const params = new URL(requests[0]!).searchParams
      expect(params.get('granularity')).toBe('day')
      expect(params.get('days')).toBe('14')
      const tz = Number(params.get('tz_offset'))
      expect(Number.isFinite(tz)).toBe(true)

      // 窗口汇总数字（totals 原样呈现）。
      const summary = view.locator('[data-testid="usage-summary"]')
      await expect(summary.locator('.stat', { hasText: '请求数' })).toContainText('3')
      await expect(summary.locator('.stat', { hasText: '输入 tokens' })).toContainText('14')
      await expect(summary.locator('.stat', { hasText: '输出 tokens' })).toContainText('58')
      await expect(summary.locator('.stat', { hasText: '缓存 tokens' })).toContainText('7')

      // 双模型折线（每系列一条 polyline，3 桶 > 1 画线不画 marker）+ 图例点名。
      const chart = view.locator('sebas-line-chart')
      await expect(chart.locator('svg polyline')).toHaveCount(2)
      const legend = chart.locator('.legend .item')
      await expect(legend).toHaveCount(2)
      await expect(legend.filter({ hasText: 'claude-sonnet' })).toHaveCount(1)
      await expect(legend.filter({ hasText: 'gpt-4o-mini' })).toHaveCount(1)

      expect(collector.clean()).toEqual([])
    })

    test('US2 granularity toggle refetches hour and re-renders the 24 buckets of today', async ({
      page,
    }) => {
      const shell = new AppShell(page)
      const requests = interceptUsage(page, HOUR_PAYLOAD)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      await page.locator('sebas-app .usage-link', { hasText: '用量' }).click()
      const view = page.locator('sebas-usage')
      await expect(view).toBeVisible()
      const chart = view.locator('sebas-line-chart')
      await expect(chart).toBeVisible()

      // 切到「按小时」：重新请求（granularity=hour，hour 忽略 days → 无 days 参）。
      await view.locator('[data-testid="granularity-hour"]').click()
      await expect
        .poll(() => requests.length, '粒度切换必须重发请求')
        .toBe(2)
      const hourUrl = new URL(requests[1]!)
      expect(hourUrl.searchParams.get('granularity')).toBe('hour')
      expect(hourUrl.searchParams.get('days')).toBeNull()

      // 24 小时桶重渲染：polyline 顶点数 = 24（零填充全窗口）。
      const points = await chart.locator('svg polyline').first().getAttribute('points')
      expect(points!.trim().split(/\s+/)).toHaveLength(24)
      // 副标题点明小时面口径。
      await expect(view.locator('.meta')).toContainText('今天 0–23 时')
      // 按下态翻转。
      await expect(view.locator('[data-testid="granularity-hour"]')).toHaveAttribute(
        'aria-pressed',
        'true',
      )
      await expect(view.locator('[data-testid="granularity-day"]')).toHaveAttribute(
        'aria-pressed',
        'false',
      )

      expect(collector.clean()).toEqual([])
    })

    test('US3 token dimension switch re-renders series without a refetch', async ({ page }) => {
      const shell = new AppShell(page)
      const requests = interceptUsage(page, DAY_PAYLOAD)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      await page.locator('sebas-app .usage-link', { hasText: '用量' }).click()
      const view = page.locator('sebas-usage')
      await expect(view.locator('sebas-line-chart')).toBeVisible()
      expect(requests).toHaveLength(1)

      // 切到「输出」维度：零请求（design D4），第一系列变为输出序列 [0, 50, 0]。
      await view.locator('[data-testid="dimension-output"]').click()
      await expect(view.locator('[data-testid="dimension-output"]')).toHaveAttribute(
        'aria-pressed',
        'true',
      )
      expect(requests).toHaveLength(1)
      const values = await page.evaluate(() => {
        // 原生 DOM API 不穿 shadow root：sebas-usage 挂在 sebas-app 的
        // shadowRoot 里，逐层下探（Playwright 定位器才自动穿透）。
        const view = document.querySelector('sebas-app')?.shadowRoot?.querySelector(
          'sebas-usage',
        ) as undefined | { shadowRoot?: ShadowRoot | null }
        const chart = view?.shadowRoot?.querySelector(
          'sebas-line-chart',
        ) as undefined | { series?: { values: number[] }[] }
        return chart?.series?.[0]?.values
      })
      expect(values).toEqual([0, 50, 0])

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('诚实空态', () => {
    test('US4 router unreachable renders the unreachable notice with its cause, no chart', async ({
      page,
    }) => {
      const shell = new AppShell(page)
      interceptUsageUnreachable(page)

      // 深链直入（路由可深链，design D5）。
      await page.goto('/usage')
      await expect(shell.brand).toBeVisible()
      const view = page.locator('sebas-usage')
      await expect(view).toBeVisible()

      // 「router 不可达」通知（非裸错误）+ cause 如实呈现；无图表。
      await expect(view.getByText('router 不可达')).toBeVisible()
      await expect(view.locator('.mono', { hasText: 'router_unreachable: connection refused' })).toBeVisible()
      await expect(view.locator('sebas-line-chart')).toHaveCount(0)
      await expect(view.getByText('暂无用量数据')).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })

    test('US5 all-zero success renders the no-data empty state, distinct from unreachable', async ({
      page,
    }) => {
      const shell = new AppShell(page)
      interceptUsage(page, ALL_ZERO_PAYLOAD)

      await page.goto('/usage')
      await expect(shell.brand).toBeVisible()
      const view = page.locator('sebas-usage')
      await expect(view).toBeVisible()

      await expect(view.getByText('暂无用量数据')).toBeVisible()
      await expect(view.getByText('router 不可达')).toHaveCount(0)
      await expect(view.locator('sebas-line-chart')).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
