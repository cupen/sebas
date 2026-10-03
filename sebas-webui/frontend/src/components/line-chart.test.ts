import { describe, expect, it } from 'vitest'
import './line-chart.js'
import {
  computeChartGeometry,
  formatCount,
  indexFromX,
  SERIES_COLORS,
  type SebasLineChart,
} from './line-chart.js'

// 与 DEFAULT_PADDING 同步（right=32 是 fix-webui-qa-round11 4.5 的右缘
// inset——最右 x 刻度的中心锚日期文本右半 ≈28px，必须留在 viewBox 内）。
const PAD = { top: 12, right: 32, bottom: 22, left: 46 }

describe('computeChartGeometry (data → points/paths pure functions)', () => {
  const W = 640
  const H = 260

  it('scales two multi-point series into the inner frame with paths', () => {
    const geo = computeChartGeometry(
      [
        { name: 'a', values: [0, 50, 100] },
        { name: 'b', values: [100, 50, 0] },
      ],
      W,
      H,
    )
    expect(geo.points).toHaveLength(2)
    for (const s of geo.points) {
      expect(s).toHaveLength(3)
      for (const p of s) {
        expect(p.x).toBeGreaterThanOrEqual(PAD.left)
        expect(p.x).toBeLessThanOrEqual(W - PAD.right)
        expect(p.y).toBeGreaterThanOrEqual(PAD.top)
        expect(p.y).toBeLessThanOrEqual(H - PAD.bottom)
      }
    }
    // path 是折线（M 起始 + L 续点）；单点 marker 为空。
    expect(geo.paths[0]).toMatch(/^M[\d.]+ [\d.]+ L/)
    expect(geo.paths[1]).toMatch(/^M[\d.]+ [\d.]+ L/)
    expect(geo.markers.every((m) => m.length === 0)).toBe(true)
    // 两条线在这组数据上必然相交错开：同一 x 上 y 互不相同。
    expect(geo.points[0]![0]!.y).not.toBe(geo.points[1]![0]!.y)
  })

  it('draws a marker, not a line, for a single-point series', () => {
    const geo = computeChartGeometry([{ name: 'a', values: [42] }], W, H)
    expect(geo.paths[0]).toBe('')
    expect(geo.markers[0]).toHaveLength(1)
    // 单点居中。
    expect(geo.markers[0]![0]!.x).toBeCloseTo(PAD.left + (W - PAD.left - PAD.right) / 2, 5)
  })

  it('renders an all-zero window as a flat line on the baseline', () => {
    const geo = computeChartGeometry([{ name: 'a', values: [0, 0, 0, 0] }], W, H)
    const ys = geo.points[0]!.map((p) => p.y)
    expect(new Set(ys).size).toBe(1, '全零 = 平线（所有点同一 y）')
    // 平线落在 0 刻度（基线）上。
    const baseline = geo.yTicks.find((t) => t.value === 0)!
    expect(ys[0]).toBeCloseTo(baseline.y, 5)
    expect(geo.yMax).toBe(1, '全零窗口 y 上限归一到 1')
  })

  it('emits ascending y ticks with a zero baseline', () => {
    const geo = computeChartGeometry([{ name: 'a', values: [10, 20, 35] }], W, H)
    const values = geo.yTicks.map((t) => t.value)
    expect(values[0]).toBe(0)
    expect(values[values.length - 1]).toBe(35)
    for (let i = 1; i < values.length; i++) {
      expect(values[i]).toBeGreaterThan(values[i - 1]!)
    }
    const ys = geo.yTicks.map((t) => t.y)
    for (let i = 1; i < ys.length; i++) {
      expect(ys[i]).toBeLessThan(ys[i - 1]!, '像素 y 自底向上递减')
    }
  })

  it('samples x labels within the cap and always includes both ends', () => {
    const values = Array.from({ length: 30 }, (_, i) => i)
    const geo = computeChartGeometry([{ name: 'a', values }], W, H)
    expect(geo.xLabels.length).toBeLessThanOrEqual(6)
    expect(geo.xLabels[0]!.index).toBe(0)
    expect(geo.xLabels[geo.xLabels.length - 1]!.index).toBe(29)
    // 短窗口逐点出刻度。
    const small = computeChartGeometry([{ name: 'a', values: [1, 2, 3] }], W, H)
    expect(small.xLabels.map((l) => l.index)).toEqual([0, 1, 2])
    // 单点窗口：单刻度居中。
    const single = computeChartGeometry([{ name: 'a', values: [1] }], W, H)
    expect(single.xLabels).toHaveLength(1)
  })

  // （fix-webui-qa-round11 4.5，A-5）右缘刻度完整可见：最右 x 刻度是中心
  // 锚的日期文本（天粒度 YYYY-MM-DD ≈56px，右半 ≈28px）——右 padding
  // 必须把它留在 viewBox 内，旧值 12 会裁成「2026-10」。
  it('rightmost x-axis label stays fully inside the viewBox at both granularities (round11 4.5)', () => {
    const W = 720 // usage 视图的图表宽度
    const HALF_DAY_LABEL = 28 // YYYY-MM-DD 中心锚的右半宽
    // 天粒度：14 桶（usage 视图近 14 天）。
    const day = computeChartGeometry(
      [{ name: 'a', values: Array.from({ length: 14 }, (_, i) => i) }],
      W,
      260,
    )
    const lastDay = day.xLabels[day.xLabels.length - 1]!
    expect(lastDay.x + HALF_DAY_LABEL).toBeLessThanOrEqual(W)
    // 小时粒度：24 桶（今天 0–23 时）；标签更短，同一 inset 自动覆盖。
    const hour = computeChartGeometry(
      [{ name: 'a', values: Array.from({ length: 24 }, (_, i) => i) }],
      W,
      260,
    )
    const lastHour = hour.xLabels[hour.xLabels.length - 1]!
    expect(lastHour.x + HALF_DAY_LABEL).toBeLessThanOrEqual(W)
  })

  it('ignores non-finite values instead of poisoning the scale', () => {
    const geo = computeChartGeometry([{ name: 'a', values: [Number.NaN, 10] }], W, H)
    expect(Number.isFinite(geo.yMax)).toBe(true)
    expect(Number.isFinite(geo.points[0]![0]!.y)).toBe(true)
  })
})

describe('indexFromX', () => {
  const W = 640

  it('maps pointer x onto the nearest index and clamps', () => {
    // 5 个点均匀铺在 640 宽的内框。
    expect(indexFromX(0, 0, W, 5)).toBe(0)
    expect(indexFromX(W, 0, W, 5)).toBe(4)
    expect(indexFromX(W / 2, 0, W, 5)).toBe(2)
    expect(indexFromX(-999, 0, W, 5)).toBe(0, '越界左夹紧')
    expect(indexFromX(99999, 0, W, 5)).toBe(4, '越界右夹紧')
  })

  it('resolves a single-point chart to index 0 and empty data to -1', () => {
    expect(indexFromX(0, 0, W, 1)).toBe(0)
    expect(indexFromX(W / 2, 0, W, 1)).toBe(0)
    expect(indexFromX(10, 0, W, 0)).toBe(-1)
  })
})

describe('formatCount', () => {
  it('abbreviates axis numbers', () => {
    expect(formatCount(0)).toBe('0')
    expect(formatCount(999)).toBe('999')
    expect(formatCount(1000)).toBe('1k')
    expect(formatCount(1200)).toBe('1.2k')
    expect(formatCount(3_400_000)).toBe('3.4M')
    expect(formatCount(1_200_000_000)).toBe('1.2G')
    expect(formatCount(Number.NaN)).toBe('0')
  })
})

describe('sebas-line-chart rendering', () => {
  async function mount(series: { name: string; values: number[] }[], xLabels: string[]) {
    const el = document.createElement('sebas-line-chart') as SebasLineChart
    el.series = series
    el.xLabels = xLabels
    document.body.appendChild(el)
    await el.updateComplete
    return el
  }

  it('renders one polyline per series and a legend entry per model', async () => {
    const el = await mount(
      [
        { name: 'claude-sonnet', values: [1, 2, 3] },
        { name: 'gpt-4o-mini', values: [3, 2, 1] },
      ],
      ['2026-09-27', '2026-09-28', '2026-09-29'],
    )
    const svg = el.shadowRoot!.querySelector('svg')!
    expect(svg.querySelectorAll('polyline')).toHaveLength(2)
    const legend = el.shadowRoot!.textContent ?? ''
    expect(legend).toContain('claude-sonnet')
    expect(legend).toContain('gpt-4o-mini')
    // x 轴刻度带桶标签。
    expect(svg.textContent).toContain('2026-09-29')
    // 每条线用调色板里不同颜色（色非唯一通道，但线必须可区分）。
    const strokes = [...svg.querySelectorAll('polyline')].map((p) => p.getAttribute('stroke'))
    expect(new Set(strokes).size).toBe(2)
    expect(SERIES_COLORS).toContain(strokes[0])
    el.remove()
  })

  it('renders svg children in the SVG namespace (svg helper, not html)', async () => {
    const el = await mount(
      [
        { name: 'a', values: [1, 2] },
        { name: 'b', values: [2, 1] },
      ],
      ['2026-09-28', '2026-09-29'],
    )
    const svg = el.shadowRoot!.querySelector('svg')!
    // 回归钉子：<svg> 内部经 ${} 注入的子模板若走 html 助手会落在 XHTML
    // 命名空间（HTMLUnknownElement）——属性俱在但真浏览器整块不渲染。
    for (const child of svg.querySelectorAll('polyline, line, text')) {
      expect(child.namespaceURI).toBe('http://www.w3.org/2000/svg')
    }
    el.remove()
  })

  it('draws a marker circle instead of a polyline for a single point', async () => {
    const el = await mount([{ name: 'm', values: [7] }], ['2026-09-29'])
    const svg = el.shadowRoot!.querySelector('svg')!
    expect(svg.querySelectorAll('polyline')).toHaveLength(0)
    expect(svg.querySelectorAll('circle.marker')).toHaveLength(1)
    el.remove()
  })

  it('shows the hover readout with per-model detail rows at the hovered index', async () => {
    const el = await mount([{ name: 'm', values: [0, 9] }], ['00', '01'])
    el.details = [['m — 输入 1 · 输出 2 · 缓存 3']]
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.readout')).toBeNull()

    // 直接驱动指针处理函数（happy-dom 的 rect 尺寸不可靠，纯函数已单测）。
    ;(el as unknown as { onPointerMove: (e: PointerEvent) => void }).onPointerMove(
      new MouseEvent('pointermove', { clientX: 10 }) as PointerEvent,
    )
    await el.updateComplete
    const readout = el.shadowRoot!.querySelector('.readout')
    expect(readout).not.toBeNull()
    expect(readout!.textContent).toContain('m — 输入 1 · 输出 2 · 缓存 3')

    // 指针离开 → 读数消失。
    ;(el as unknown as { onPointerLeave: () => void }).onPointerLeave()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.readout')).toBeNull()
    el.remove()
  })
})
