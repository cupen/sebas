/**
 * Usage model — pure helpers (migrated verbatim from the pure half of
 * `views/usage.test.ts`).
 *
 * The Lit `<sebas-usage>` element retired with the React cutover; the helpers
 * keep their coverage because `react/pages/UsagePage.tsx` imports
 * `views/usage-model.ts` directly. View orchestration (request shape, empty
 * states, granularity switching) is covered by `UsagePage.test.tsx` and the
 * Playwright usage journey.
 */

import { describe, expect, it } from 'vitest'
import type { UsageTimeseries } from '../api/client.js'
import {
  bucketLabel,
  detailRows,
  isAllZero,
  seriesFor,
  windowTotals,
} from './usage-model.js'
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

/** 全零窗口（D8 空态判别的桶形状）。 */
function zeroBuckets(): UsageTimeseries['buckets'] {
  return [
    { bucket: '2026-09-27', models: [] },
    { bucket: '2026-09-28', models: [] },
    { bucket: '2026-09-29', models: [] },
  ]
}

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
