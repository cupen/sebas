import { useEffect, useId, useMemo, useState } from 'react'
import { Alert, Button, Card, Empty, Segmented, Spin, Statistic, Typography } from 'antd'
import { ApiError, api, errorText, type UsageTimeseries } from '../../api/client.js'
import { SERIES_COLORS, computeChartGeometry, formatCount } from '../../components/chart-geometry.js'
import {
  USAGE_SOURCE_NOTE,
  bucketLabel,
  detailRows,
  isAllZero,
  localTzOffsetMinutes,
  partialNoticeText,
  seriesFor,
  windowTotals,
  type Granularity,
  type TokenDimension,
  type UsageSource,
} from '../../views/usage-model.js'
import styles from './usage.module.css'

export interface UsagePageProps {
  load?: (params: {
    granularity: Granularity
    days?: number
    tzOffset?: number
    source?: UsageSource
  }) => Promise<UsageTimeseries>
}

const GRANULARITY = [
  { label: '天', value: 'day' },
  { label: '小时', value: 'hour' },
]
const SOURCE = [
  { label: '全部', value: 'all' },
  { label: 'router', value: 'router' },
  { label: '本地', value: 'local' },
]
const DIMENSION = [
  { label: '总量', value: 'total' },
  { label: '输入', value: 'input' },
  { label: '输出', value: 'output' },
  { label: '缓存', value: 'cache' },
]

const WIDTH = 760
const HEIGHT = 260

export function UsagePage({ load = api.usageTimeseries }: UsagePageProps) {
  const [granularity, setGranularity] = useState<Granularity>('day')
  const [source, setSource] = useState<UsageSource>('all')
  const [dimension, setDimension] = useState<TokenDimension>('total')
  const [data, setData] = useState<UsageTimeseries | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)
  const [loading, setLoading] = useState(true)
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set())
  const chartId = useId()

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError(null)
    load({
      granularity,
      tzOffset: localTzOffsetMinutes(),
      source,
      days: granularity === 'day' ? 30 : 1,
    })
      .then((next) => {
        if (cancelled) return
        setData(next)
      })
      .catch((failure: unknown) => {
        if (cancelled) return
        setData(null)
        if (failure instanceof ApiError && failure.code === 'router_unreachable') {
          setError(`router 不可达：${failure.message}`)
        } else if (failure instanceof ApiError && failure.status === 401) {
          setError('登录已失效，请重新登录')
        } else {
          setError(`用量加载失败：${errorText(failure)}`)
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [granularity, source, reload, load])

  const series = useMemo(() => (data ? seriesFor(data, dimension) : []), [data, dimension])
  const visible = useMemo(() => series.filter((item) => !hidden.has(item.name)), [series, hidden])
  const geometry = useMemo(
    () => computeChartGeometry(visible, WIDTH, HEIGHT, { maxXTicks: granularity === 'hour' ? 8 : 6 }),
    [visible, granularity],
  )
  const totals = data ? windowTotals(data) : null
  const empty = data !== null && isAllZero(data)
  const rows = data ? detailRows(data) : []

  function toggle(name: string): void {
    setHidden((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  return (
    <div className={styles.page} data-testid="usage-page">
      <header className={styles.head}>
        <div>
          <Typography.Title level={3} className={styles.title}>
            用量
          </Typography.Title>
          <Typography.Paragraph type="secondary" className={styles.sub}>
            Token 消耗时序：按模型分列，天 / 小时两种粒度。
          </Typography.Paragraph>
        </div>
        <Button onClick={() => setReload((value) => value + 1)} loading={loading}>
          刷新
        </Button>
      </header>

      <div className={styles.controls}>
        <div className={styles.seg}>
          <span className={styles.segLabel}>来源</span>
          <Segmented
            aria-label="数据来源"
            value={source}
            options={SOURCE}
            onChange={(value) => setSource(value as UsageSource)}
          />
        </div>
        <div className={styles.seg}>
          <span className={styles.segLabel}>粒度</span>
          <Segmented
            aria-label="粒度"
            value={granularity}
            options={GRANULARITY}
            onChange={(value) => setGranularity(value as Granularity)}
          />
        </div>
        <div className={styles.seg}>
          <span className={styles.segLabel}>维度</span>
          <Segmented
            aria-label="维度"
            value={dimension}
            options={DIMENSION}
            onChange={(value) => setDimension(value as TokenDimension)}
          />
        </div>
      </div>
      <Typography.Paragraph className={styles.sourceNote}>{USAGE_SOURCE_NOTE}</Typography.Paragraph>

      {data?.router_cause ? (
        <Alert
          className={styles.notice}
          type="warning"
          showIcon
          title={partialNoticeText(data.router_cause)}
        />
      ) : null}
      {error !== null ? (
        <Alert
          className={styles.notice}
          type="error"
          showIcon
          title={error}
          action={<Button size="small" onClick={() => setReload((value) => value + 1)}>重试</Button>}
          role="alert"
        />
      ) : null}

      {loading && data === null ? (
        <div className={styles.loading}>
          <Spin description="加载用量…" />
        </div>
      ) : empty ? (
        <Card variant="borderless" className={styles.card}>
          <Empty description="暂无用量数据" />
        </Card>
      ) : data !== null && error === null ? (
        <>
          <div className={styles.stats}>
            <Card size="small" variant="borderless" className={styles.card}>
              <Statistic title="请求数" value={totals?.requests ?? 0} />
            </Card>
            <Card size="small" variant="borderless" className={styles.card}>
              <Statistic title="输入 tokens" value={totals?.input_tokens ?? 0} />
            </Card>
            <Card size="small" variant="borderless" className={styles.card}>
              <Statistic title="输出 tokens" value={totals?.output_tokens ?? 0} />
            </Card>
            <Card size="small" variant="borderless" className={styles.card}>
              <Statistic
                title="缓存 tokens"
                value={(totals?.cache_read_tokens ?? 0) + (totals?.cache_creation_tokens ?? 0)}
              />
            </Card>
          </div>

          <Card variant="borderless" className={`${styles.card} ${styles.chartCard}`}>
            <div className={styles.legend} role="group" aria-label="模型图例">
              {series.map((item, index) => {
                const off = hidden.has(item.name)
                return (
                  <button
                    key={item.name}
                    type="button"
                    className={styles.legendItem}
                    aria-pressed={!off}
                    onClick={() => toggle(item.name)}
                  >
                    <span
                      className={styles.swatch}
                      style={{ background: SERIES_COLORS[index % SERIES_COLORS.length] }}
                      aria-hidden="true"
                    />
                    {item.name}
                  </button>
                )
              })}
            </div>
            <svg
              className={styles.chart}
              viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
              role="img"
              aria-labelledby={chartId}
            >
              <title id={chartId}>
                按模型分列的 token 用量折线图，{granularity === 'day' ? '天' : '小时'}粒度
              </title>
              <line
                x1="46"
                x2={WIDTH - 32}
                y1={HEIGHT - 22}
                y2={HEIGHT - 22}
                className={styles.axis}
              />
              {geometry.yTicks.map((tick) => (
                <g key={`y-${tick.value}`}>
                  <line x1="46" x2={WIDTH - 32} y1={tick.y} y2={tick.y} className={styles.grid} />
                  <text x="40" y={tick.y + 4} textAnchor="end" className={styles.tick}>
                    {formatCount(tick.value)}
                  </text>
                </g>
              ))}
              {geometry.xLabels.map((label) => (
                <text
                  key={`x-${label.index}`}
                  x={label.x}
                  y={HEIGHT - 6}
                  textAnchor="middle"
                  className={styles.tick}
                >
                  {bucketLabel(data.buckets[label.index]?.bucket ?? '', granularity)}
                </text>
              ))}
              {visible.map((item, seriesIndex) => {
                const color = SERIES_COLORS[seriesIndex % SERIES_COLORS.length]
                const points = geometry.points[seriesIndex] ?? []
                return (
                  <g key={item.name}>
                    {geometry.paths[seriesIndex] ? (
                      <path d={geometry.paths[seriesIndex]} fill="none" stroke={color} strokeWidth="2" />
                    ) : null}
                    {points.map((point, pointIndex) => (
                      <circle key={pointIndex} cx={point.x} cy={point.y} r="2.5" fill={color}>
                        <title>
                          {`${item.name} · ${bucketLabel(data.buckets[pointIndex]?.bucket ?? '', granularity)} · ${formatCount(item.values[pointIndex] ?? 0)}`}
                        </title>
                      </circle>
                    ))}
                  </g>
                )
              })}
            </svg>
          </Card>

          <Card variant="borderless" className={styles.card} title="逐时段读数">
            <ul className={styles.readings}>
              {rows.map((entries, index) => (
                <li key={data.buckets[index]?.bucket ?? index}>
                  <span className={styles.readingBucket}>
                    {bucketLabel(data.buckets[index]?.bucket ?? '', granularity)}
                  </span>
                  {entries.length === 0 ? (
                    <span className={styles.readingEmpty}>无请求</span>
                  ) : (
                    <ul>
                      {entries.map((row) => (
                        <li key={row}>{row}</li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          </Card>
        </>
      ) : null}
    </div>
  )
}
