/**
 * Usage view（add-usage-statistics 4.2，design D5）：router 用量时序的独立
 * 看数面（`/usage`，可深链）。天/小时粒度切换、token 维度切换（总量/输入/
 * 输出/缓存）、窗口汇总数字、按模型多折线（`sebas-line-chart`，手写 SVG）。
 *
 * 两种空态**分开呈现**（spec 场景「router unreachable empty state」）：
 * - `ApiError.code === 'router_unreachable'` → 「router 不可达」通知（附
 *   cause，不是裸错误），提示与「没有数据」是两回事；
 * - 聚合成功但全零 → 「暂无用量数据」。
 *
 * 时区口径（design D3）：浏览器把本机偏移（分钟东偏）随请求下传，桶边界
 * 与操作者直觉一致；API 一次给全四类明细，维度切换零请求。
 */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { ApiError, api, type UsageTimeseries, errorText } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '../components/line-chart.js'
import type { ChartSeries } from '../components/line-chart.js'

type Granularity = 'day' | 'hour'
type TokenDimension = 'total' | 'input' | 'output' | 'cache'

const DIMENSIONS: { key: TokenDimension; label: string }[] = [
  { key: 'total', label: '总量' },
  { key: 'input', label: '输入' },
  { key: 'output', label: '输出' },
  { key: 'cache', label: '缓存' },
]

/** 浏览器本机的分钟东偏（design D3：`tz_offset` 随查询下传）。 */
function localTzOffsetMinutes(): number {
  return -new Date().getTimezoneOffset()
}

/**
 * （fix-webui-qa-round13 4.1，观察-1）数据源口径说明（usage-statistics spec
 * 「数据源口径 SHALL 钉死为 router 用量记录」）：usage 页只呈现经 router
 * 的流量聚合（usage.db 单写者归 router，架构不动）；ACP 直连回合不经
 * router、不产生用量记录，其 token 计数呈现于会话头部，不入本视图。
 * 有数据与「无用量数据」空态同带此行——纯 ACP 窗口的空态读作**口径
 * 事实**（这里本来只看 router）而非故障或覆盖缺口。唯一出处在此常量，
 * 视图两处渲染同一份措辞。
 */
export const USAGE_SOURCE_NOTE =
  '用量统计来自 router 流量；ACP 直连会话不经 router、不产生用量记录，其 token 计数见各会话头部，不计入本页。'

/** 桶标签的人类可读形：天粒度原样（`YYYY-MM-DD`），小时粒度补成 `HH:00`。 */
export function bucketLabel(bucket: string, granularity: Granularity): string {
  if (granularity === 'hour') return `${bucket}:00`
  return bucket
}

/** 从聚合载荷抽取「所选维度」的每模型数值序列（与桶序对齐，零填充已由
 * router 保证）。模型按名字字典序（router 输出序），稳定且可复现。 */
export function seriesFor(
  data: UsageTimeseries,
  dimension: TokenDimension,
): ChartSeries[] {
  const names = new Set<string>()
  for (const b of data.buckets) {
    for (const m of b.models) names.add(m.model)
  }
  const pick = (m: (typeof data.totals) | undefined): number => {
    if (!m) return 0
    switch (dimension) {
      case 'input':
        return m.input_tokens
      case 'output':
        return m.output_tokens
      case 'cache':
        return m.cache_read_tokens + m.cache_creation_tokens
      case 'total':
        return (
          m.input_tokens +
          m.output_tokens +
          m.cache_read_tokens +
          m.cache_creation_tokens
        )
    }
  }
  return [...names].sort().map((name) => ({
    name,
    values: data.buckets.map((b) => pick(b.models.find((m) => m.model === name))),
  }))
}

/** 窗口是否全零（「暂无用量数据」空态判别——聚合成功但什么都没发生）。 */
export function isAllZero(data: UsageTimeseries): boolean {
  const t = windowTotals(data)
  return (
    t.requests === 0 &&
    t.input_tokens === 0 &&
    t.output_tokens === 0 &&
    t.cache_read_tokens === 0 &&
    t.cache_creation_tokens === 0
  )
}

/**
 * （fix-webui-qa-round3 1.4 / D8）**窗口**合计（纯函数）：统计卡的数字从
 * 与图表**同一份 buckets** 现场累加——卡片口径按构造与图表窗口一致，切换
 * 粒度/窗口即随新响应重算（spec「summary cards follow the selected
 * window」）。不再信任载荷顶层 `totals`：它一旦与桶窗口漂移（旧聚合器、
 * 代理层缓存），卡片就会呈现「全量口径」与空图表同屏的矛盾（QA W3 实锤
 * 形态：请求数 1 常驻、按小时图表窗内全空）。
 */
export function windowTotals(data: UsageTimeseries): UsageTimeseries['totals'] {
  const t = data.totals
  const acc = {
    requests: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  }
  for (const b of data.buckets) {
    for (const m of b.models) {
      acc.requests += m.requests
      acc.input_tokens += m.input_tokens
      acc.output_tokens += m.output_tokens
      acc.cache_read_tokens += m.cache_read_tokens
      acc.cache_creation_tokens += m.cache_creation_tokens
    }
  }
  // totals 里 model 字段等非数值形状保持载荷原样：数值五项以桶和为准。
  return { ...t, ...acc }
}

/** 悬停读数：该时点各模型的四类明细（task 4.1；API 一次给全，切换零请求）。 */
export function detailRows(data: UsageTimeseries): string[][] {
  return data.buckets.map((b) =>
    b.models.map(
      (m) =>
        `${m.model} · ${m.requests} 次 · 输入 ${m.input_tokens} · 输出 ${m.output_tokens} · 缓存 ${
          m.cache_read_tokens + m.cache_creation_tokens
        }`,
    ),
  )
}

@customElement('sebas-usage')
export class SebasUsage extends LitElement {
  @state() private granularity: Granularity = 'day'
  @state() private dimension: TokenDimension = 'total'
  @state() private data: UsageTimeseries | null = null
  /** 「router 不可达」的结构化 cause；`null` = 可达。 */
  @state() private unreachableCause: string | null = null
  @state() private loading = true
  @state() private reloadSeq = 0

  static styles = [
    viewStyles,
    css`
      .controls {
        display: flex;
        gap: var(--sebas-space-3);
        align-items: center;
        flex-wrap: wrap;
        margin-bottom: var(--sebas-space-4);
      }
      .seg {
        display: inline-flex;
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-md);
        overflow: hidden;
      }
      .seg button {
        border: none;
        background: none;
        color: var(--sebas-text-dim);
        font: inherit;
        font-size: 0.82rem;
        font-weight: 550;
        padding: 6px 14px;
        cursor: pointer;
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .seg button + button {
        border-left: 1px solid var(--sebas-border);
      }
      .seg button[aria-pressed='true'] {
        background: var(--sebas-accent-soft);
        color: var(--sebas-accent);
      }
      .seg button:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: -2px;
      }
      .seg .dim-label {
        font-size: 0.72rem;
        color: var(--sebas-text-faint);
        text-transform: uppercase;
        letter-spacing: 0.07em;
        align-self: center;
        padding: 0 var(--sebas-space-2);
      }
      .panel {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
      }
      sebas-line-chart {
        width: 100%;
      }
      .summary {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
        gap: var(--sebas-space-3);
        margin-bottom: var(--sebas-space-4);
      }
      .stat {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-3) var(--sebas-space-4);
      }
      .stat .num {
        font-size: 1.15rem;
        font-weight: 700;
        font-variant-numeric: tabular-nums;
        color: var(--sebas-text-bright);
      }
      .stat .label {
        font-size: 0.74rem;
        color: var(--sebas-text-dim);
        text-transform: uppercase;
        letter-spacing: 0.07em;
      }
      .meta {
        color: var(--sebas-text-faint);
        font-size: 0.76rem;
        margin-top: var(--sebas-space-2);
      }
      /* （fix-webui-qa-round13 4.1）数据源口径说明行：图表态贴摘要区上方
         （.meta 的上边距让位给下边距），空态面板内与成因 hint 拉开一行。 */
      .source-note {
        margin: 0 0 var(--sebas-space-3);
      }
      .empty .source-note {
        margin: var(--sebas-space-2) 0 0;
      }
      .reload {
        margin-left: auto;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    void this.load()
  }

  private async load(): Promise<void> {
    const seq = ++this.reloadSeq
    this.loading = true
    try {
      const data = await api.usageTimeseries({
        granularity: this.granularity,
        days: this.granularity === 'day' ? 14 : undefined,
        tzOffset: localTzOffsetMinutes(),
      })
      if (seq !== this.reloadSeq) return // 过期应答：已有更新的请求在途
      this.data = data
      this.unreachableCause = null
    } catch (e) {
      if (seq !== this.reloadSeq) return
      this.data = null
      this.unreachableCause =
        e instanceof ApiError && e.code === 'router_unreachable'
          ? e.message
          : null
      // 其它失败（网络断 / 未登录跳转等）也按不可达呈现 cause，但没有
      // 结构化 cause 时用通用文案。
      if (this.unreachableCause === null && !(e instanceof ApiError && e.status === 401)) {
        this.unreachableCause = errorText(e)
      }
    } finally {
      if (seq === this.reloadSeq) this.loading = false
    }
  }

  private setGranularity(g: Granularity): void {
    if (this.granularity === g) return
    this.granularity = g
    void this.load()
  }

  private setDimension(d: TokenDimension): void {
    // 维度切换零请求：API 一次给全四类明细（design D4）。
    this.dimension = d
  }

  private renderUnreachable() {
    // 两种失败文案分开：结构化 router_unreachable cause = 「router 不可达」；
    // 其它失败（webui 自身不可达等）= 通用「加载失败」，不冒充 router 不可达。
    const isRouter = (this.unreachableCause ?? '').startsWith('router_unreachable')
    return html`
      <section class="panel">
        <div class="empty" role="status">
          <span class="glyph">${icon('alert', 20)}</span>
          <span class="title">${isRouter ? 'router 不可达' : '用量加载失败'}</span>
          <p class="hint">
            ${
              isRouter
                ? '用量数据由 router 记录与聚合；当前 router 未在运行或无法连接，历史数据无从读取。'
                : ''
            }
            <br />
            <span class="mono">${this.unreachableCause ?? ''}</span>
          </p>
        </div>
      </section>
    `
  }

  private renderNoData() {
    return html`
      <section class="panel">
        <div class="empty" role="status">
          <span class="glyph">${icon('usage', 20)}</span>
          <span class="title">暂无用量数据</span>
          <p class="hint">
            ${
              this.granularity === 'hour'
                ? '今天（按本机时区）还没有任何经 router 的请求。'
                : '窗口内没有任何经 router 的请求。'
            }
          </p>
          <!-- （fix-webui-qa-round13 4.1）口径说明随空态在场：纯 ACP 窗口
               （router 用量记录为零）读作数据源口径事实，不伪装成 router
               故障，也不渲染空图（spec 场景「纯 ACP 流量窗口呈现无数据
               空态」）。 -->
          <p class="hint source-note" data-testid="usage-source-note">${USAGE_SOURCE_NOTE}</p>
        </div>
      </section>
    `
  }

  render() {
    // （fix-webui-qa-round3 1.4 / D8）卡片数字 = 窗口合计（与图表同一份
    // buckets 现场累加），不再读载荷顶层 totals——口径一致性由构造保证。
    const t = this.data ? windowTotals(this.data) : null
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">用量</h1>
          <p class="page-sub">Token 消耗时序：按模型分列，天 / 小时两种粒度。</p>
        </div>
      </header>

      <div class="controls">
        <div class="seg" role="group" aria-label="粒度">
          <button
            aria-pressed=${this.granularity === 'day'}
            data-testid="granularity-day"
            @click=${() => this.setGranularity('day')}
          >
            按天
          </button>
          <button
            aria-pressed=${this.granularity === 'hour'}
            data-testid="granularity-hour"
            @click=${() => this.setGranularity('hour')}
          >
            按小时（今天）
          </button>
        </div>
        <div class="seg" role="group" aria-label="Token 维度">
          <span class="dim-label">维度</span>
          ${DIMENSIONS.map(
            (d) => html`
              <button
                aria-pressed=${this.dimension === d.key}
                data-testid=${`dimension-${d.key}`}
                @click=${() => this.setDimension(d.key)}
              >
                ${d.label}
              </button>
            `,
          )}
        </div>
        <button
          class="retry-btn reload"
          data-testid="usage-reload"
          ?disabled=${this.loading}
          @click=${() => void this.load()}
        >
          刷新
        </button>
      </div>

      ${this.unreachableCause !== null
        ? this.renderUnreachable()
        : this.data === null
          ? html`<section class="panel"><div class="empty"><span class="glyph">${icon('usage', 20)}</span><span class="title">加载中…</span></div></section>`
          : isAllZero(this.data)
            ? this.renderNoData()
            : html`
                <!-- （fix-webui-qa-round13 4.1，观察-1）数据源口径说明：
                     摘要区上方一行，钉死「本页 = router 用量记录」。 -->
                <p class="meta source-note" data-testid="usage-source-note">${USAGE_SOURCE_NOTE}</p>
                <div class="summary" data-testid="usage-summary">
                  <div class="stat">
                    <div class="num">${t!.requests}</div>
                    <div class="label">请求数</div>
                  </div>
                  <div class="stat">
                    <div class="num">${t!.input_tokens}</div>
                    <div class="label">输入 tokens</div>
                  </div>
                  <div class="stat">
                    <div class="num">${t!.output_tokens}</div>
                    <div class="label">输出 tokens</div>
                  </div>
                  <div class="stat">
                    <div class="num">${t!.cache_read_tokens + t!.cache_creation_tokens}</div>
                    <div class="label">缓存 tokens</div>
                  </div>
                </div>
                <section class="panel">
                  <sebas-line-chart
                    .series=${seriesFor(this.data, this.dimension)}
                    .xLabels=${this.data.buckets.map((b) =>
                      bucketLabel(b.bucket, this.data!.granularity),
                    )}
                    .details=${detailRows(this.data)}
                    .width=${720}
                    .height=${260}
                  ></sebas-line-chart>
                  <p class="meta">
                    ${
                      this.data.granularity === 'day'
                        ? `近 ${this.data.days} 天 · 桶边界按本机时区（UTC${
                            this.data.tz_offset >= 0 ? '+' : ''
                          }${this.data.tz_offset / 60}）切分`
                        : `今天 0–23 时 · 桶边界按本机时区（UTC${
                            this.data.tz_offset >= 0 ? '+' : ''
                          }${this.data.tz_offset / 60}）切分`
                    }
                  </p>
                </section>
              `}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-usage': SebasUsage
  }
}
