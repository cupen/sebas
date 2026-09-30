// @vitest-environment happy-dom
/**
 * Usage view 测试（add-usage-statistics 4.2）：天/小时粒度切换、token 维度
 * 切换（零请求）、窗口汇总数字、以及两种空态**分开呈现**（router 不可达 vs
 * 无数据）。api client 全量 mock；「数据→点位」几何由 line-chart.test.ts
 * 单独钉，这里钉视图层的编排。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'
import './usage.js'
import type { SebasUsage } from './usage.js'
import {
  bucketLabel,
  detailRows,
  isAllZero,
  seriesFor,
  windowTotals,
} from './usage.js'
import { ApiError, type UsageTimeseries } from '../api/client.js'

const apiMocks = vi.hoisted(() => ({
  usageTimeseries: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  api: {
    usageTimeseries: apiMocks.usageTimeseries,
  },
  ApiError: class ApiError extends Error {
    readonly status: number
    readonly code: string | null
    constructor(status: number, message: string, code: string | null = null) {
      super(message)
      this.status = status
      this.code = code
    }
  },
}))

function fixture(overrides: Partial<UsageTimeseries> = {}): UsageTimeseries {
  return {
    granularity: 'day',
    days: 3,
    tz_offset: 480,
    buckets: [
      {
        bucket: '2026-09-27',
        models: [],
      },
      {
        bucket: '2026-09-28',
        models: [
          {
            model: 'claude-sonnet',
            requests: 2,
            input_tokens: 10,
            output_tokens: 50,
            cache_read_tokens: 5,
            cache_creation_tokens: 2,
          },
          {
            model: 'gpt-4o-mini',
            requests: 1,
            input_tokens: 4,
            output_tokens: 8,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
          },
        ],
      },
      {
        bucket: '2026-09-29',
        models: [],
      },
    ],
    totals: {
      model: 'total',
      requests: 3,
      input_tokens: 14,
      output_tokens: 58,
      cache_read_tokens: 5,
      cache_creation_tokens: 2,
    },
    ...overrides,
  }
}

async function mount(): Promise<SebasUsage> {
  const el = document.createElement('sebas-usage') as SebasUsage
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

/** 全零窗口（D8 空态判别的桶形状）。 */
function zeroBuckets(): UsageTimeseries['buckets'] {
  return [
    { bucket: '2026-09-27', models: [] },
    { bucket: '2026-09-28', models: [] },
    { bucket: '2026-09-29', models: [] },
  ]
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMocks.usageTimeseries.mockResolvedValue(fixture())
})

describe('usage view pure helpers', () => {
  it('seriesFor extracts the chosen dimension per model over all buckets', () => {
    const data = fixture()
    const input = seriesFor(data, 'input')
    expect(input.map((s) => s.name)).toEqual(['claude-sonnet', 'gpt-4o-mini'])
    expect(input[0]!.values).toEqual([0, 10, 0])

    const total = seriesFor(data, 'total')
    // claude 28 号总量 = 10 + 50 + 5 + 2 = 67。
    expect(total[0]!.values).toEqual([0, 67, 0])

    const cache = seriesFor(data, 'cache')
    // 缓存维度 = cache_read + cache_creation。
    expect(cache[0]!.values).toEqual([0, 7, 0])
    expect(cache[1]!.values).toEqual([0, 0, 0])
  })

  it('isAllZero judges the WINDOW (buckets), not the payload totals (D8)', () => {
    expect(isAllZero(fixture())).toBe(false)
    // 窗口内全空 = 无数据，即便载荷顶层 totals 带着别处的数（漂移防线）。
    expect(isAllZero(fixture({ buckets: zeroBuckets() }))).toBe(true)
    // totals 全零但桶里有用量 = 有数据（旧口径误判的反向形态）。
    expect(
      isAllZero(
        fixture({
          totals: {
            model: 'total',
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
          },
        }),
      ),
    ).toBe(false)
  })

  it('windowTotals sums the buckets the chart shows, ignoring drifted totals (D8)', () => {
    // 载荷顶层 totals 与桶窗口漂移（QA W3 实锤的「卡片全量、图表窗口」）：
    // 卡片取数必须等于桶和。
    const drifted = fixture({
      totals: {
        model: 'total',
        requests: 999,
        input_tokens: 99_999,
        output_tokens: 99_999,
        cache_read_tokens: 99_999,
        cache_creation_tokens: 99_999,
      },
    })
    const t = windowTotals(drifted)
    expect(t.requests).toBe(3)
    expect(t.input_tokens).toBe(14)
    expect(t.output_tokens).toBe(58)
    expect(t.cache_read_tokens).toBe(5)
    expect(t.cache_creation_tokens).toBe(2)
    // 空窗口 = 全零合计（空态判别与卡片同源）。
    const empty = windowTotals(fixture({ buckets: zeroBuckets() }))
    expect(empty.requests).toBe(0)
    expect(empty.input_tokens).toBe(0)
  })

  it('bucketLabel formats hour buckets as HH:00 and keeps dates as-is', () => {
    expect(bucketLabel('2026-09-29', 'day')).toBe('2026-09-29')
    expect(bucketLabel('07', 'hour')).toBe('07:00')
  })

  it('detailRows renders four-class detail per model at each point', () => {
    const rows = detailRows(fixture())
    expect(rows).toHaveLength(3)
    expect(rows[1]).toEqual([
      'claude-sonnet · 2 次 · 输入 10 · 输出 50 · 缓存 7',
      'gpt-4o-mini · 1 次 · 输入 4 · 输出 8 · 缓存 0',
    ])
    expect(rows[0]).toEqual([])
  })
})

describe('sebas-usage view', () => {
  it('renders summary numbers and per-model lines for the day window', async () => {
    const el = await mount()
    expect(apiMocks.usageTimeseries).toHaveBeenCalledWith(
      expect.objectContaining({ granularity: 'day' }),
    )
    const text = el.shadowRoot!.textContent ?? ''
    // 窗口汇总数字。
    expect(text).toContain('14', '输入 tokens 合计')
    expect(text).toContain('58', '输出 tokens 合计')
    const chart = el.shadowRoot!.querySelector('sebas-line-chart')!
    expect(chart.series).toHaveLength(2)
    // 图例带模型名（图例渲染在 chart 的 shadow 里）。
    await chart.updateComplete
    const chartText = chart.shadowRoot!.textContent ?? ''
    expect(chartText).toContain('claude-sonnet')
    expect(chartText).toContain('gpt-4o-mini')
    el.remove()
  })

  it('granularity toggle refetches with hour and flips the pressed state', async () => {
    const el = await mount()
    expect(apiMocks.usageTimeseries).toHaveBeenCalledTimes(1)
    const hourBtn = el.shadowRoot!.querySelector<HTMLButtonElement>(
      '[data-testid="granularity-hour"]',
    )!
    hourBtn.click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.usageTimeseries).toHaveBeenCalledTimes(2)
    expect(apiMocks.usageTimeseries).toHaveBeenLastCalledWith(
      expect.objectContaining({ granularity: 'hour', days: undefined }),
    )
    expect(hourBtn.getAttribute('aria-pressed')).toBe('true')
    expect(
      el.shadowRoot!
        .querySelector<HTMLButtonElement>('[data-testid="granularity-day"]')!
        .getAttribute('aria-pressed'),
    ).toBe('false')
    el.remove()
  })

  it('token dimension switch re-renders the series without a new request', async () => {
    const el = await mount()
    expect(apiMocks.usageTimeseries).toHaveBeenCalledTimes(1)
    el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="dimension-output"]')!.click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    // 零请求：API 一次给全四类明细，切换纯前端（design D4）。
    expect(apiMocks.usageTimeseries).toHaveBeenCalledTimes(1)
    const chart = el.shadowRoot!.querySelector('sebas-line-chart')!
    expect(chart.series[0]!.values).toEqual([0, 50, 0], '切换到「输出」维度')
    el.remove()
  })

  it('renders the router-unreachable empty state, distinct from no-data', async () => {
    apiMocks.usageTimeseries.mockRejectedValue(
      new ApiError(503, 'router_unreachable: connection refused', 'router_unreachable'),
    )
    const el = await mount()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('router 不可达')
    expect(text).toContain('router_unreachable: connection refused', 'cause 如实呈现')
    expect(text).not.toContain('暂无用量数据', '两种空态必须分开')
    expect(el.shadowRoot!.querySelector('sebas-line-chart')).toBeNull()
    el.remove()
  })

  it('renders the no-data empty state when aggregation succeeds all-zero', async () => {
    // （D8）空态判别走窗口桶（与卡片/图表同源）——顶层 totals 即便残留
    // 非零旧值，空窗口照样呈现「暂无用量数据」。
    apiMocks.usageTimeseries.mockResolvedValue(
      fixture({
        buckets: zeroBuckets(),
        totals: {
          model: 'total',
          requests: 7,
          input_tokens: 700,
          output_tokens: 700,
          cache_read_tokens: 7,
          cache_creation_tokens: 7,
        },
      }),
    )
    const el = await mount()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('暂无用量数据')
    expect(text).not.toContain('router 不可达', '两种空态必须分开')
    expect(el.shadowRoot!.querySelector('sebas-line-chart')).toBeNull()
    el.remove()
  })

  it('summary cards recompute from the new window after a granularity switch (D8)', async () => {
    // 切粒度 → 重取 → 卡片数字随新窗口重算（spec「summary cards follow
    // the selected window」）：日窗 14 输入 → 小时窗 mock 只剩 4。
    const el = await mount()
    expect(el.shadowRoot!.textContent).toContain('14', '日窗输入 tokens')
    apiMocks.usageTimeseries.mockResolvedValue(
      fixture({
        granularity: 'hour',
        days: 1,
        buckets: [
          {
            bucket: '07',
            models: [
              {
                model: 'claude-sonnet',
                requests: 1,
                input_tokens: 4,
                output_tokens: 8,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
              },
            ],
          },
        ],
        totals: {
          model: 'total',
          requests: 999,
          input_tokens: 99_999,
          output_tokens: 99_999,
          cache_read_tokens: 99_999,
          cache_creation_tokens: 99_999,
        },
      }),
    )
    el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="granularity-hour"]')!.click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('4', '小时窗输入 tokens 重算')
    expect(text).not.toContain('14', '旧窗口数字不再驻留')
    expect(text).not.toContain('999', '漂移的顶层 totals 不上卡片')
    el.remove()
  })
})
