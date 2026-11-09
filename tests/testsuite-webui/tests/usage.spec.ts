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
      await shell.nav.getByRole('button', { name: '用量' }).click()
      await expect(page).toHaveURL(/\/usage$/)
      const view = page.getByTestId('usage-page')
      await expect(view).toBeVisible()

      // 请求口径：day 粒度 + 30 天窗口 + 本机时区偏移（分钟东偏）。
      expect(requests).toHaveLength(1)
      const params = new URL(requests[0]!).searchParams
      expect(params.get('granularity')).toBe('day')
      expect(params.get('days')).toBe('30')
      const tz = Number(params.get('tz_offset'))
      expect(Number.isFinite(tz)).toBe(true)

      // 窗口汇总数字（AntD Statistic 卡片：标题 + 数值同卡）。
      const stat = (title: string) => view.locator('.ant-statistic').filter({ hasText: title })
      await expect(stat('请求数')).toContainText('3')
      await expect(stat('输入 tokens')).toContainText('14')
      await expect(stat('输出 tokens')).toContainText('58')
      await expect(stat('缓存 tokens')).toContainText('7')

      // 双模型折线（每系列一条 path）+ 图例点名。
      const chart = view.locator('svg[role="img"]')
      await expect(chart.locator('path')).toHaveCount(2)
      const legend = view.getByRole('group', { name: '模型图例' })
      await expect(legend.getByRole('button')).toHaveCount(2)
      await expect(legend.getByRole('button', { name: 'claude-sonnet' })).toHaveCount(1)
      await expect(legend.getByRole('button', { name: 'gpt-4o-mini' })).toHaveCount(1)

      expect(collector.clean()).toEqual([])
    })

    test('US2 granularity toggle refetches hour and re-renders the 24 buckets of today', async ({
      page,
    }) => {
      const shell = new AppShell(page)
      const requests = interceptUsage(page, HOUR_PAYLOAD)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      await shell.nav.getByRole('button', { name: '用量' }).click()
      const view = page.getByTestId('usage-page')
      await expect(view).toBeVisible()
      const chart = view.locator('svg[role="img"]')
      await expect(chart).toBeVisible()

      // 切到「按小时」：重新请求（granularity=hour，窗口固定为 1 天）。
      await view.getByRole('radiogroup', { name: '粒度' }).getByText('小时', { exact: true }).click()
      await expect
        .poll(() => requests.length, '粒度切换必须重发请求')
        .toBe(2)
      const hourUrl = new URL(requests[1]!)
      expect(hourUrl.searchParams.get('granularity')).toBe('hour')
      expect(hourUrl.searchParams.get('days')).toBe('1')

      // 24 小时桶重渲染：折线 path 覆盖 24 个零填充桶（用圆点 title 计数）。
      await expect(chart.locator('circle')).toHaveCount(48)
      // 副标题点明小时面口径（图表 title 点名 24 小时粒度）。
      await expect(chart.locator('title').first()).toContainText('小时粒度')
      // 选态翻转（AntD Segmented 的可见 label 反映当前项）。
      const hourLabel = view
        .getByRole('radiogroup', { name: '粒度' })
        .locator('.ant-segmented-item-selected')
      await expect(hourLabel).toContainText('小时')

      expect(collector.clean()).toEqual([])
    })

    test('US3 token dimension switch re-renders series without a refetch', async ({ page }) => {
      const shell = new AppShell(page)
      const requests = interceptUsage(page, DAY_PAYLOAD)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      await shell.nav.getByRole('button', { name: '用量' }).click()
      const view = page.getByTestId('usage-page')
      await expect(view.locator('svg[role="img"]')).toBeVisible()
      expect(requests).toHaveLength(1)

      // 切到「输出」维度：零请求（design D4），图表改用输出序列 [0, 50, 0]。
      const dimension = view.getByRole('radiogroup', { name: '维度' })
      await dimension.getByText('输出', { exact: true }).click()
      await expect(dimension.locator('.ant-segmented-item-selected')).toContainText('输出')
      expect(requests).toHaveLength(1)
      const chart = view.locator('svg[role="img"]')
      // gpt 的输出序列在 09-28 桶 = 50；切维度后点值随数据重算。
      await expect(chart.locator('circle title', { hasText: '· 50' })).toHaveCount(1)

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
      const view = page.getByTestId('usage-page')
      await expect(view).toBeVisible()

      // 「router 不可达」通知（非裸错误）+ cause 如实呈现；无图表。
      const alert = view.getByRole('alert')
      await expect(alert).toContainText('router 不可达')
      await expect(alert).toContainText('router_unreachable: connection refused')
      await expect(view.locator('svg[role="img"]')).toHaveCount(0)
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
      const view = page.getByTestId('usage-page')
      await expect(view).toBeVisible()

      await expect(view.getByText('暂无用量数据')).toBeVisible()
      await expect(view.getByText('router 不可达')).toHaveCount(0)
      await expect(view.locator('svg[role="img"]')).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })
})
