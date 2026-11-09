/**
 * Deterministic performance fixture (4.6). Dev-only entry (`perf.html`, never a
 * build input) that renders the SAME 500-entry transcript the budget's mixed
 * fixture describes. No API, no provider: every entry is generated here.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import type { ConversationEntryView, SessionDetail } from '../api/client.js'
import { setMarkdownObserver } from '../components/markdown.js'
import { TranscriptView } from '../react/conversation/TranscriptView.js'
import { Composer } from '../react/conversation/Composer.js'
import { ThemeProvider, useThemeMode } from '../react/theme/ThemeProvider.js'
import '../styles/tokens.css'
import '../styles/app.css'

const BASE_COUNT = 500
const P95 = (values: number[], q: number): number => {
  if (values.length === 0) return 0
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * q))]!
}

const CJK =
  '这是一个用于性能验收的中文长句：流式增量到达时历史转录不得重新解析，滚动位置不得跳动，审批与工具折叠保持稳定。'
const LONG_URL =
  'https://example.test/a/very/long/path/with/many/segments/that/should/wrap/instead/of/overflowing/the/viewport?token=abcdefghijklmnopqrstuvwxyz0123456789&more=0123456789abcdef'
const CODE = '```rust\nfn work(n: usize) -> usize {\n    (0..n).map(|i| i * i).sum()\n}\n```'
const TABLE = '| 列 | 值 |\n| --- | --- |\n| 延迟 | 50ms |\n| 预算 | 硬门 |'
const LONG = `${CJK}\n\n${'性能验收长文。'.repeat(1200)}`

function entry(
  position: number,
  kind: 'prompt' | 'content',
  element_type: ConversationEntryView['element_type'],
  content: string,
  extra: Partial<ConversationEntryView> = {},
): ConversationEntryView {
  return { position, kind, element_type, content, created_at_unix: 1_800_000_000 + position, ...extra }
}

/** Exactly 500 entries with the budget's mixed shapes. */
export function fixture(): ConversationEntryView[] {
  const entries: ConversationEntryView[] = []
  let block = 0
  while (entries.length < BASE_COUNT) {
    const p = entries.length
    entries.push(entry(p, 'prompt', 'markdown', `第 ${block + 1} 轮：${CJK}`))
    entries.push(entry(p + 1, 'content', 'markdown', `${CJK}\n\n${TABLE}`))
    entries.push(
      entry(p + 2, 'content', 'thinking', block % 2 === 0 ? `思考：${CJK}` : `thinking ${block}`),
    )
    const id = `toolu_perf_${block}`
    entries.push(
      entry(p + 3, 'content', 'tool', `bash · echo ${block}\n${CJK}`, {
        title: `bash · echo ${block}`,
        tool_use_id: id,
      }),
    )
    entries.push(
      entry(p + 4, 'content', 'tool', `done ${block}`, {
        title: '✓ bash',
        tool_use_id: id,
      }),
    )
    entries.push(entry(p + 5, 'content', 'markdown', CODE))
    entries.push(entry(p + 6, 'content', 'markdown', `${CJK}\n\n${LONG_URL}`))
    entries.push(
      entry(
        p + 7,
        'content',
        'markdown',
        block % 5 === 0 ? LONG : `${CJK} ${'增量。'.repeat(30)}`,
      ),
    )
    entries.push(
      block % 20 === 19
        ? entry(p + 8, 'content', 'notice', '**回合已结束且无输出**：本轮回合未产生任何可见输出。')
        : entry(p + 8, 'content', 'markdown', `${CJK}\n\n- 项目 ${block}\n- 预算 50ms`),
    )
    entries.push(
      block % 25 === 24
        ? entry(p + 9, 'content', 'error', `spawn failed: fixture ${block}`, {
            failure_class: 'spawn',
          })
        : entry(p + 9, 'content', 'markdown', `${CJK} 收尾 ${block}。`),
    )
    block += 1
  }
  return entries.slice(0, BASE_COUNT)
}

interface Counts {
  parse: number
  sanitize: number
  highlight: number
  unit: Record<string, number>
}

interface PerfApi {
  static(): Promise<void>
  stream(seconds: number, rate: number): Promise<void>
  burst(): Promise<void>
  settle(): Promise<void>
  snapshot(): Record<string, unknown>
  raw(): { raf: number[]; longTasks: number[] }
  resetCounters(): void
  scrollToBottom(): Promise<void>
  scrollUp(): Promise<void>
  jumpToLatest(): Promise<void>
  toggleFirstFold(): Promise<boolean>
  measure(promise: Promise<unknown>): Promise<number>
}

declare global {
  interface Window {
    __perf?: PerfApi
    __perfCounts?: Counts
    __perfInitial?: Counts
  }
}

const perfDetail = {
  chat_id: 'perf',
  thread_id: null,
  session_id: 'perf',
  status: 'active',
  status_label: 'Done',
  status_slug: 'done',
  status_glyph: '✓',
  entries: [],
  msg_id: null,
  last_active: 'now',
  encoded_key: 'perf',
  msg_count: 0,
  current_model: 'perf',
  available_models: ['perf'],
  desired_mode: 'ask',
  effective_mode: 'ask',
  agent_kind: 'perf',
  usage: null,
  project_id: null,
  remote: null,
  pending: [],
  available_commands: [],
} as unknown as SessionDetail

const rafSamples: number[] = []
const longTasks: number[] = []
let sampling = false
let lastFrame = 0
let running = false

function sampleFrame(now: number): void {
  if (sampling) {
    if (lastFrame !== 0) rafSamples.push(now - lastFrame)
    lastFrame = now
  }
  requestAnimationFrame(sampleFrame)
}
requestAnimationFrame(sampleFrame)
new PerformanceObserver((list) => {
  for (const item of list.getEntries()) longTasks.push(item.duration)
}).observe({ type: 'longtask', buffered: true })

function resetTiming(): void {
  rafSamples.length = 0
  longTasks.length = 0
  lastFrame = 0
  sampling = true
}

function counts(): Counts {
  window.__perfCounts ??= { parse: 0, sanitize: 0, highlight: 0, unit: {} }
  return window.__perfCounts
}
setMarkdownObserver({
  parse: () => (counts().parse += 1),
  sanitize: () => (counts().sanitize += 1),
  highlight: () => (counts().highlight += 1),
})

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function PerfApp() {
  const [entries, setEntries] = useState<ConversationEntryView[]>(() => fixture())
  const [live, setLive] = useState(false)
  const [status, setStatus] = useState('idle')
  const [detached, setDetached] = useState(false)
  const liveRef = useRef(false)
  const statusRef = useRef(status)
  const entriesRef = useRef(entries)
  const scrollHost = useRef<HTMLDivElement | null>(null)
  const follow = useRef(true)

  entriesRef.current = entries
  liveRef.current = live
  statusRef.current = status

  // Same bottom-follow contract as ConversationPane, exercised under the
  // 500-entry fixture so the scroll budget is measured on a real layout.
  useEffect(() => {
    const host = scrollHost.current
    if (host === null) return
    if (!follow.current) return
    host.scrollTop = host.scrollHeight
  }, [entries.length])

  function onScroll(): void {
    const host = scrollHost.current
    if (host === null) return
    const distance = host.scrollHeight - host.scrollTop - host.clientHeight
    follow.current = distance <= 48
    setDetached(!follow.current)
  }

  function jumpNow(): void {
    const host = scrollHost.current
    if (host === null) return
    follow.current = true
    setDetached(false)
    host.scrollTop = host.scrollHeight
  }

  const onUnitRender = useCallback((key: string) => {
    counts().unit[key] = (counts().unit[key] ?? 0) + 1
  }, [])

  useEffect(() => {
    const api: PerfApi = {
      async static() {
        resetTiming()
        setLive(false)
        setEntries(fixture())
        setStatus('static')
        await sleep(0)
      },
      async stream(seconds, rate) {
        resetTiming()
        const base = fixture()
        setEntries(base)
        setLive(true)
        setStatus('streaming')
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        )
        const initial = structuredClone(counts())
        window.__perfCounts = { parse: 0, sanitize: 0, highlight: 0, unit: {} }
        resetTiming()
        running = true
        const total = Math.round(seconds * rate)
        const startedAt = performance.now()
        for (let i = 0; i < total && running; i += 1) {
          const due = startedAt + (i * 1000) / rate
          const wait = due - performance.now()
          if (wait > 0) await sleep(wait)
          const chunk = 'x'.repeat(1 + (i % 40))
          setEntries((current) => [
            ...current,
            entry(current.length, 'content', 'markdown', `${chunk} `),
          ])
        }
        running = false
        window.__perfInitial = initial
        setStatus('streamed')
      },
      async burst() {
        resetTiming()
        const base = fixture()
        setEntries(base)
        setLive(true)
        setStatus('burst')
        // Four parallel tool events (independent ids) precede the 200 appends.
        const toolEntries: ConversationEntryView[] = [0, 1, 2, 3].flatMap((n) => [
          entry(base.length + n * 2, 'content', 'tool', `web_fetch ${n}`, {
            title: `web_fetch · ${n}`,
            tool_use_id: `toolu_burst_${n}`,
          }),
          entry(base.length + n * 2 + 1, 'content', 'tool', `fetched ${n}`, {
            title: '✓ web_fetch',
            tool_use_id: `toolu_burst_${n}`,
          }),
        ])
        setEntries([...base, ...toolEntries])
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        )
        const initial = structuredClone(counts())
        window.__perfCounts = { parse: 0, sanitize: 0, highlight: 0, unit: {} }
        resetTiming()
        running = true
        for (let i = 0; i < 200 && running; i += 1) {
          await sleep(10)
          setEntries((current) => [
            ...current,
            entry(current.length, 'content', 'markdown', `burst${i} `),
          ])
        }
        running = false
        window.__perfInitial = initial
        setStatus('burst-done')
      },
      async settle() {
        setLive(false)
        setStatus('settled')
        await sleep(0)
      },
      snapshot() {
        const host = scrollHost.current
        const unitKeys = [...document.querySelectorAll('[data-unit-key]')].map(
          (el) => el.getAttribute('data-unit-key') ?? '',
        )
        const duplicateKeys = unitKeys.filter((key, index) => unitKeys.indexOf(key) !== index)
        const rendered = counts().unit
        const streamingKeys = Object.keys(rendered).filter((key) => key !== '')
        return {
          status: statusRef.current,
          initial: window.__perfInitial ?? null,
          live: liveRef.current,
          entryCount: entriesRef.current.length,
          domUnits: unitKeys.length,
          raf: {
            count: rafSamples.length,
            p50: P95(rafSamples, 0.5),
            p95: P95(rafSamples, 0.95),
            max: rafSamples.length ? Math.max(...rafSamples) : 0,
          },
          longTasks: [...longTasks],
          markdown: {
            parse: counts().parse,
            sanitize: counts().sanitize,
            highlight: counts().highlight,
          },
          unitRenders: rendered,
          unitKeys: streamingKeys.length,
          lastUnitKey: unitKeys.at(-1) ?? null,
          duplicateKeys,
          heap: (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory
            ?.usedJSHeapSize ?? null,
          overflow: {
            document:
              document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
            scroller: host ? host.scrollWidth <= host.clientWidth + 1 : true,
          },
          scroll: host
            ? {
                top: host.scrollTop,
                height: host.scrollHeight,
                client: host.clientHeight,
                distance: host.scrollHeight - host.scrollTop - host.clientHeight,
              }
            : null,
          ts: Date.now(),
        }
      },
      resetCounters() {
        window.__perfCounts = { parse: 0, sanitize: 0, highlight: 0, unit: {} }
        resetTiming()
      },
      raw() {
        return { raf: [...rafSamples], longTasks: [...longTasks] }
      },
      async scrollToBottom() {
        jumpNow()
        await sleep(0)
      },
      async scrollUp() {
        const host = scrollHost.current
        if (host === null) return
        follow.current = false
        setDetached(true)
        host.scrollTop = Math.max(0, host.scrollHeight / 2)
        await sleep(0)
      },
      async jumpToLatest() {
        jumpNow()
        await sleep(0)
      },
      async toggleFirstFold() {
        const react = scrollHost.current?.querySelector('details > summary')
        if (react instanceof HTMLElement) {
          react.click()
          await sleep(0)
          return true
        }
        return false
      },
      async measure(promise) {
        const started = performance.now()
        await promise
        await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
        return performance.now() - started
      },
    }
    window.__perf = api
    return () => {
      running = false
      if (window.__perf === api) delete window.__perf
    }
  }, [])

  return (
    <div>
      <div className="perf-toolbar">
        <strong>sebas perf fixture</strong>
        <span data-testid="perf-status">{status}</span>
        <span data-testid="perf-count">{entries.length}</span>
      </div>
      <div className="perf-approval" data-testid="perf-approval">
        <span>待审批</span> <strong>bash</strong> <code>mkdir -p .sebas-probe</code>
        <button type="button">允许一次</button>
      </div>
      <div
        className="perf-scroller"
        ref={scrollHost}
        style={{ height: 700, overflowY: 'auto', position: 'relative' }}
        data-testid="perf-scroller"
        onScroll={onScroll}
      >
        <TranscriptView entries={entries} live={live} onUnitRender={onUnitRender} />
        {detached ? (
          <button
            type="button"
            data-testid="perf-jump"
            onClick={jumpNow}
            style={{ position: 'sticky', left: '100%', bottom: 12, width: 88, height: 44 }}
          >
            跳到最新
          </button>
        ) : null}
      </div>
      <div style={{ padding: '8px 12px' }}>
        <Composer
          sessionKey="perf"
          detail={perfDetail}
          reachability={{ ok: true }}
          onRefresh={async () => {}}
        />
      </div>
    </div>
  )
}

export function mountPerf(root: HTMLElement): void {
  const theme = new URLSearchParams(window.location.search).get('theme')
  createRoot(root).render(
    <ThemeProvider initialMode={theme === 'dark' || theme === 'light' ? theme : undefined}>
      <ThemeBridge />
      <PerfApp />
    </ThemeProvider>,
  )
}

/** Deterministic theme switching for the 4.6 responsive/theme runs. */
function ThemeBridge() {
  const { setMode } = useThemeMode()
  useEffect(() => {
    ;(window as Window & { __perfSetTheme?: (mode: 'light' | 'dark') => void }).__perfSetTheme =
      setMode
  }, [setMode])
  return null
}
