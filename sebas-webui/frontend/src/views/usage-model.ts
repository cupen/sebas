/**
 * Framework-agnostic usage model (3.3). Pure projections shared by the Lit
 * usage view and the React usage page; no Lit, DOM or network imports.
 */

import type { UsageTimeseries } from '../api/client.js'
import type { ChartSeries } from '../components/chart-geometry.js'

export type Granularity = 'day' | 'hour'
export type TokenDimension = 'total' | 'input' | 'output' | 'cache'
/** 数据口径（add-local-usage-statistics D4/D6）：全部 = 本地 + router 合并。 */
export type UsageSource = 'all' | 'router' | 'local'

/** 浏览器本机的分钟东偏（design D3：`tz_offset` 随查询下传）。 */
export function localTzOffsetMinutes(): number {
  return -new Date().getTimezoneOffset()
}

/**
 * （add-local-usage-statistics 5.1）数据源口径说明（usage-statistics spec
 * 「source selector」）：usage 页按来源出数——「全部」合并本地回合（不经
 * router 的 ACP/native 直连回合）与经 router 的流量；「router」只计经
 * router 的请求；「本地」只计不经 router 的回合。写入侧单源规则保证一条
 * 请求只被一个源计数，全部口径不重算。唯一出处在此常量，视图两处渲染
 * 同一份措辞。
 */
export const USAGE_SOURCE_NOTE =
  '「全部」合并两类流量：不经 router 的回合（ACP/native 直连，记本地账）与经 router 的请求；「router」与「本地」各自单看一侧。一条请求只被一个源计数。'

/** `source=all` 且 router 缺席时的局部警示条文案（cause 随行如实呈现）。 */
export function partialNoticeText(routerCause: string): string {
  return `router 部分不可达——当前「全部」口径只含本地数据（${routerCause}）`
}

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
