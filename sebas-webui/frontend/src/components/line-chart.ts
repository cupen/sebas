/**
 * sebas-line-chart：手写 SVG 折线图（add-usage-statistics 4.1，design D6）。
 *
 * 零新依赖：「数据 → 点位/path」全部抽成纯函数（[`computeChartGeometry`] /
 * [`indexFromX`] / [`formatCount`]），组件只做渲染——纯函数进 vitest 钉形状
 * （多系列折线、单点画 marker 不画线、全零平线、坐标轴刻度）。
 *
 * 边界数据（design Risks）：单点画 marker 不画线；全零返回平线（y 轴按
 * max(1, …) 归一）；悬停读数由 view 以 `details`（逐 x 索引的行文本）传入，
 * 组件只负责呈现。颜色不是唯一通道：图例文字 + 悬停行文本都带模型名。
 */

import { LitElement, css, html, nothing, svg } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'

/** 一条折线系列：名字 + 与 x 轴索引等长的数值序列。 */
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
const DEFAULT_PADDING: ChartPadding = { top: 12, right: 32, bottom: 22, left: 46 }

/** 计数的人类可读形（轴标签用）：0 / 999 / 1.2k / 3.4M / 1.2G。 */
export function formatCount(n: number): string {
  if (!Number.isFinite(n)) return '0'
  const abs = Math.abs(n)
  if (abs >= 1_000_000_000) return `${trim(n / 1_000_000_000)}G`
  if (abs >= 1_000_000) return `${trim(n / 1_000_000)}M`
  if (abs >= 1_000) return `${trim(n / 1_000)}k`
  return String(Math.round(n))
}

function trim(v: number): string {
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

function sampleIndices(n: number, maxTicks: number): number[] {
  if (n <= 0) return []
  if (n === 1) return [0]
  if (n <= maxTicks) return Array.from({ length: n }, (_, i) => i)
  const step = (n - 1) / (maxTicks - 1)
  const out = Array.from({ length: maxTicks - 1 }, (_, i) => Math.round(i * step))
  out.push(n - 1)
  return [...new Set(out)].sort((a, b) => a - b)
}

function clamp(v: number): number {
  return Number.isFinite(v) ? Math.max(0, v) : 0
}

function round(v: number): number {
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

@customElement('sebas-line-chart')
export class SebasLineChart extends LitElement {
  /** 折线系列（每系列一条折线，长度恒等于桶数——含零填充桶）。 */
  @property({ type: Array }) series: ChartSeries[] = []
  /** x 轴刻度文本（与数据索引对齐的桶标签）。 */
  @property({ type: Array }) xLabels: string[] = []
  /** 悬停读数（add-usage-statistics 4.1）：逐 x 索引的行文本（该时点各模型
   * 四类明细，view 组装）；缺行时组件回退为各系列数值。 */
  @property({ type: Array }) details: string[][] = []
  @property({ type: Number }) width = 640
  @property({ type: Number }) height = 260

  /** 当前悬停的数据索引；`null` = 不显示读数。 */
  @state() private hoverIndex: number | null = null
  /** 悬停读数的像素 x（跟随指针，随侧翻转会自动换边）。 */
  @state() private hoverX = 0

  static styles = css`
    :host {
      display: block;
      position: relative;
      max-width: 100%;
    }
    svg {
      width: 100%;
      height: auto;
      display: block;
    }
    .gridline {
      stroke: var(--sebas-border, rgba(128, 138, 160, 0.25));
      stroke-width: 1;
      stroke-dasharray: 3 4;
    }
    .baseline {
      stroke: var(--sebas-border-strong, rgba(128, 138, 160, 0.5));
      stroke-width: 1;
    }
    .ytick,
    .xtick {
      fill: var(--sebas-text-faint, #8a92a6);
      font-size: 10px;
      font-variant-numeric: tabular-nums;
    }
    .xtick {
      text-anchor: middle;
    }
    .ytick {
      text-anchor: end;
    }
    polyline {
      fill: none;
      stroke-width: 2;
      stroke-linejoin: round;
      stroke-linecap: round;
    }
    circle.marker {
      stroke-width: 2;
      fill: var(--sebas-surface, #fff);
    }
    .legend {
      display: flex;
      flex-wrap: wrap;
      gap: var(--sebas-space-2, 8px);
      margin-top: var(--sebas-space-2, 8px);
    }
    .legend .item {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      font-size: 0.78rem;
      color: var(--sebas-text-dim, #aeb5c6);
    }
    .legend .swatch {
      width: 14px;
      height: 3px;
      border-radius: 2px;
      flex: 0 0 auto;
    }
    /* 悬停读数（tooltip）：跟随指针、贴边自动换向；行文本由 view 提供。 */
    .readout {
      position: absolute;
      top: 8px;
      z-index: 5;
      pointer-events: none;
      background: var(--sebas-surface, #161a24);
      border: 1px solid var(--sebas-border-strong, rgba(128, 138, 160, 0.5));
      border-radius: var(--sebas-radius-md, 10px);
      box-shadow: var(--sebas-shadow-2, 0 8px 24px rgba(8, 10, 18, 0.35));
      padding: 8px 10px;
      font-size: 0.78rem;
      color: var(--sebas-text, #e6e9f2);
      min-width: 180px;
      max-width: 260px;
    }
    .readout .head {
      color: var(--sebas-text-dim, #aeb5c6);
      font-weight: 600;
      margin-bottom: 4px;
    }
    .readout .row {
      display: flex;
      justify-content: space-between;
      gap: 12px;
      font-variant-numeric: tabular-nums;
      line-height: 1.5;
    }
    .readout .row .name {
      overflow-wrap: anywhere;
    }
    .hoverline {
      stroke: var(--sebas-text-faint, #8a92a6);
      stroke-width: 1;
      stroke-dasharray: 2 3;
    }
  `

  private color(i: number): string {
    return SERIES_COLORS[i % SERIES_COLORS.length]!
  }

  private onPointerMove = (e: PointerEvent): void => {
    const rect = this.renderRoot.querySelector('svg')?.getBoundingClientRect()
    const n = this.series[0]?.values.length ?? 0
    if (!rect || n === 0) {
      this.hoverIndex = null
      return
    }
    this.hoverIndex = indexFromX(e.clientX, rect.left, rect.width, n)
    // 读数定位：指针在 svg 内的相对位置（像素），贴边换向在 render 里做。
    this.hoverX = ((e.clientX - rect.left) / rect.width) * this.width
  }

  private onPointerLeave = (): void => {
    this.hoverIndex = null
  }

  render() {
    const n = this.series[0]?.values.length ?? 0
    if (n === 0 || this.series.length === 0) {
      return html`<div class="legend" aria-hidden="true"></div>`
    }
    const geo = computeChartGeometry(this.series, this.width, this.height)
    return html`
      <svg
        viewBox="0 0 ${this.width} ${this.height}"
        role="img"
        aria-label=${`折线图：${this.series.map((s) => s.name).join('、')}`}
        @pointermove=${this.onPointerMove}
        @pointerleave=${this.onPointerLeave}
      >
        ${geo.yTicks.map(
          // svg 助手而非 html：<svg> 内部经 ${} 注入的子模板若走 html 会解析成
          // XHTML 命名空间的 HTMLUnknownElement——属性俱在但整块不渲染（真浏览器
          // 才暴露，happy-dom 不敏感）；svg 助手强制子模板按 SVG 命名空间解析。
          (t) => svg`
            <line
              class=${t.value === 0 ? 'baseline' : 'gridline'}
              x1=${DEFAULT_PADDING.left}
              x2=${this.width - DEFAULT_PADDING.right}
              y1=${t.y}
              y2=${t.y}
            ></line>
            <text class="ytick" x=${DEFAULT_PADDING.left - 6} y=${t.y + 3}>
              ${formatCount(t.value)}
            </text>
          `,
        )}
        ${geo.xLabels.map((l) => svg`
          <text class="xtick" x=${l.x} y=${this.height - 6}>
            ${this.xLabels[l.index] ?? ''}
          </text>
        `)}
        ${this.hoverIndex !== null
          ? svg`<line
              class="hoverline"
              x1=${geo.points[0]![this.hoverIndex]!.x}
              x2=${geo.points[0]![this.hoverIndex]!.x}
              y1=${DEFAULT_PADDING.top}
              y2=${this.height - DEFAULT_PADDING.bottom}
            ></line>`
          : nothing}
        ${this.series.map((s, si) => {
          const color = this.color(si)
          return svg`
            ${geo.paths[si]
              ? svg`<polyline
                  points=${geo.points[si]!.map((p) => `${round(p.x)},${round(p.y)}`).join(' ')}
                  stroke=${color}
                ></polyline>`
              : nothing}
            ${geo.markers[si]!.map(
              (p) => svg`<circle
                class="marker"
                cx=${round(p.x)}
                cy=${round(p.y)}
                r="4"
                stroke=${color}
              ></circle>`,
            )}
          `
        })}
      </svg>
      <div class="legend" role="list">
        ${this.series.map(
          (s, si) => html`
            <span class="item" role="listitem">
              <span class="swatch" style=${`background: ${this.color(si)}`}></span>${s.name}
            </span>
          `,
        )}
      </div>
      ${this.hoverIndex !== null ? this.renderReadout(geo) : nothing}
    `
  }

  /** 悬停读数：head = 桶标签，行 = view 传入的四类明细（缺省回退系列值）。 */
  private renderReadout(geo: ChartGeometry) {
    const i = this.hoverIndex!
    const rows = this.details[i] ?? this.series.map((s) => `${s.name}: ${formatCount(s.values[i] ?? 0)}`)
    // 贴边换向：指针过半后读数挂到左侧。
    const flip = this.hoverX > this.width / 2
    const style = flip
      ? `right: ${Math.max(0, this.width - this.hoverX + 12)}px;`
      : `left: ${Math.max(0, this.hoverX + 12)}px;`
    return html`
      <div class="readout" style=${style} role="status">
        <div class="head">${this.xLabels[i] ?? `#${i + 1}`}</div>
        ${rows.map(
          (r) => html`<div class="row"><span class="name">${r}</span></div>`,
        )}
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-line-chart': SebasLineChart
  }
}
