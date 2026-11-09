/**
 * Chart geometry model (3.3). Pure data-to-SVG projections extracted from the
 * Lit chart so the React usage page can draw the same chart without importing
 * Lit. `line-chart.ts` re-exports every symbol.
 */

export interface ChartSeries {
  name: string
  values: number[]
}

export interface ChartPoint {
  x: number
  y: number
}

export interface YTick {
  value: number
  y: number
}

/** x 轴刻度：命中的数据索引 + 像素位置。 */
export interface XLabel {
  index: number
  x: number
}

export interface ChartGeometry {
  /** 每系列的像素点列（折线顶点）。 */
  points: ChartPoint[][]
  /** 每系列的 SVG path `d`；单点系列为 `''`（画 marker 不画线）。 */
  paths: string[]
  /** 每系列的 marker 点（仅单点系列非空）。 */
  markers: ChartPoint[][]
  /** y 轴刻度（自底向上：0 基线在前）。 */
  yTicks: YTick[]
  /** x 轴刻度（均匀采样，含首尾）。 */
  xLabels: XLabel[]
  /** y 归一上限（全零窗口为 1——平线画在基线上）。 */
  yMax: number
}

export interface ChartPadding {
  top: number
  right: number
  bottom: number
  left: number
}

/**
 * 右缘 inset 32：最右 x 轴刻度是**中心锚**（text-anchor: middle）的日期文本
 * （天粒度 `YYYY-MM-DD` ≈ 56px，右半 ≈ 28px）——右 padding 12 时右半伸出
 * viewBox 被裁（QA A-5 实锤「2026-10」）。32 ≥ 28 保证最右刻度完整可见
 * （fix-webui-qa-round11 4.5，两种粒度：小时粒度 `HH:00` 更短，自动覆盖）。
 */
export const DEFAULT_PADDING: ChartPadding = { top: 12, right: 32, bottom: 22, left: 46 }

/** 计数的人类可读形（轴标签用）：0 / 999 / 1.2k / 3.4M / 1.2G。 */
export function formatCount(n: number): string {
  if (!Number.isFinite(n)) return '0'
  const abs = Math.abs(n)
  if (abs >= 1_000_000_000) return `${trim(n / 1_000_000_000)}G`
  if (abs >= 1_000_000) return `${trim(n / 1_000_000)}M`
  if (abs >= 1_000) return `${trim(n / 1_000)}k`
  return String(Math.round(n))
}

export function trim(v: number): string {
  return v.toFixed(v >= 10 ? 0 : 1).replace(/\.0$/, '')
}

/**
 * 「数据 → 点位/path」纯函数（task 4.1）：
 * - 多系列折线：每系列按值缩放到内框；
 * - 单点系列：不产出 path，只产出 marker（画点不画线）；
 * - 全零窗口：y 上限取 max(1, …)，所有系列平铺在基线（平线）；
 * - 刻度：y 轴 4 等分（0 基线在内），相邻等值标签去重（round13 观察-2：
 *   小值域下 1/1/1/0/0 形态绝迹）；x 轴均匀采样 ≤ maxXTicks 个（含首尾）。
 */
export function computeChartGeometry(
  series: ChartSeries[],
  width: number,
  height: number,
  opts?: { padding?: Partial<ChartPadding>; maxXTicks?: number },
): ChartGeometry {
  const pad: ChartPadding = { ...DEFAULT_PADDING, ...opts?.padding }
  const maxXTicks = opts?.maxXTicks ?? 6
  const innerW = Math.max(1, width - pad.left - pad.right)
  const innerH = Math.max(1, height - pad.top - pad.bottom)
  const n = series[0]?.values.length ?? 0

  const peak = Math.max(1, ...series.flatMap((s) => s.values.map((v) => (Number.isFinite(v) ? v : 0))))
  const xAt = (i: number): number =>
    n <= 1 ? pad.left + innerW / 2 : pad.left + (innerW * i) / (n - 1)
  const yAt = (v: number): number => pad.top + innerH * (1 - clamp(v) / peak)

  const points = series.map((s) => s.values.map((v, i) => ({ x: xAt(i), y: yAt(v) })))
  const paths = points.map((pts) =>
    pts.length > 1
      ? pts.map((p, i) => `${i === 0 ? 'M' : 'L'}${round(p.x)} ${round(p.y)}`).join(' ')
      : '',
  )
  const markers = points
    .map((pts, si) => (series[si]!.values.length === 1 ? pts : []))

  // y 刻度：0 基线起 4 等分（peak ≥ 1，全零窗口平线画在 0 刻度上）。
  // （fix-webui-qa-round13 2.1，观察-2）相邻等值标签去重：小值域下 4 等分
  // 产出 formatCount 取整后相同的相邻刻度（QA round13 观察 y 轴呈现
  // 1/1/1/0/0），读感像渲染瑕疵。按**渲染标签**判等（轴上的重复是标签级
  // 现象），等值相邻只保留首个（run 头）——0 基线刻度（run 头恒为首个）
  // 与轴的位置单调性保持，去重后仍自底向上递增；峰顶若与下方刻度同标签
  // 则不再重复标出。轴不换库、数据聚合不动。
  const rawTicks: YTick[] = [0, 1, 2, 3, 4].map((i) => {
    const value = (peak * i) / 4
    return { value, y: yAt(value) }
  })
  const yTicks: YTick[] = rawTicks.filter(
    (t, i) => i === 0 || formatCount(t.value) !== formatCount(rawTicks[i - 1]!.value),
  )

  // x 刻度：≤ maxXTicks 均匀采样，恒含首尾（n===1 时单刻度居中）。
  const xLabels: XLabel[] = sampleIndices(n, maxXTicks).map((i) => ({
    index: i,
    x: xAt(i),
  }))

  return { points, paths, markers, yTicks, xLabels, yMax: peak }
}

/** 悬停命中：指针 x → 最近的数据索引（组件把 getBoundingClientRect 的结果
 * 传入；纯函数便于单测）。空数据恒 -1。 */
export function indexFromX(
  clientX: number,
  rectLeft: number,
  rectWidth: number,
  count: number,
): number {
  if (count <= 0) return -1
  if (count === 1) return 0
  const pad = DEFAULT_PADDING.left
  const innerW = Math.max(1, rectWidth - pad - DEFAULT_PADDING.right)
  const ratio = (clientX - rectLeft - pad) / innerW
  const idx = Math.round(ratio * (count - 1))
  return Math.min(count - 1, Math.max(0, idx))
}

export function sampleIndices(n: number, maxTicks: number): number[] {
  if (n <= 0) return []
  if (n === 1) return [0]
  if (n <= maxTicks) return Array.from({ length: n }, (_, i) => i)
  const step = (n - 1) / (maxTicks - 1)
  const out = Array.from({ length: maxTicks - 1 }, (_, i) => Math.round(i * step))
  out.push(n - 1)
  return [...new Set(out)].sort((a, b) => a - b)
}

export function clamp(v: number): number {
  return Number.isFinite(v) ? Math.max(0, v) : 0
}

export function round(v: number): number {
  return Math.round(v * 100) / 100
}

/** 固定调色板（对比度与主题无关；图例/悬停行都带名字，色非唯一通道）。 */
export const SERIES_COLORS = [
  '#6366f1',
  '#38d1dd',
  '#f59e0b',
  '#34d399',
  '#f472b6',
  '#a78bfa',
  '#fb7185',
  '#84cc16',
]
