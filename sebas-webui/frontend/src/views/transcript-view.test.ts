// @vitest-environment jsdom
/**
 * sebas-transcript-view — the conversation view
 * (workbench-natural-conversation-flow 1.1–2.3; former workbench-
 * conversation-view 2.1–2.5 and workbench-agent-identity-and-process-folds
 * 2.1–3.2 keep their coverage here, updated to the run model).
 *
 * The component groups the ordered entry sequence into turns (a prompt
 * opens an operator turn; agent chunks until the next prompt form ONE
 * turn), splits each agent turn into ALTERNATING text/process runs, and
 * runs a turn-counting seen-boundary seam. Scenarios:
 *
 *   1.1  splitAgentRuns: contiguous same-kind entries merge into one run;
 *        kind changes alternate text/process runs at their arrival
 *        positions; errors never join a run (standalone counted bubbles)
 *   1.2  process run id = the run's first entry position, stable across
 *        regrouping; fold open state tracked per id in a local Map and
 *        restored after re-render (D2)
 *   2.1  one collapsed-by-default fold per process run; second-level
 *        per-entry folds inside (structured titles, generic fallback);
 *        summary row shows the running entry title + entry count and
 *        updates live; an expanded fold appends streamed entries in place
 *        without collapsing (D3)
 *   2.2  agent side carries no card chrome (no .bubble); user side keeps
 *        the tinted block; error bubbles keep their counted card
 *   2.3  "已收到" receipt badge while the prompt is still the newest entry
 *        (pure entry-sequence); gone once the agent reply arrives
 *   ws   turn.append frames (shared-ws mock) drive the streaming faces at
 *        unit level: position dedup (incremental-sync cursor), session
 *        filtering, live fold summary, live receipt clearing, and the
 *        queued-submission turn boundary
 *   (seam / seen-boundary / fill-mode / author-label coverage from the
 *   former changes keeps running unchanged)
 *
 * The localStorage polyfill below replaces whatever jsdom ships so the
 * tests stay deterministic across environments and so the production
 * code path (which reads/writes through the global) is exercised
 * verbatim.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConversationEntryView } from '../api/client.js'
import {
  ERROR_MERGE_WINDOW_SECS,
  TRUNCATE_CHARS,
  TRUNCATE_LINES,
  awaitingReceipt,
  deniedDetailContent,
  deniedLabel,
  errorEntryLabel,
  groupConversation,
  mergeSpawnErrors,
  middleTruncate,
  processItemLabel,
  processRunDenied,
  processRunSummary,
  registerEmptyStreamSession,
  resolveAgentDisplay,
  splitAgentRuns,
  toolResultDenied,
  truncateHtml,
  unitMaxTs,
  unitSegmentCount,
} from './transcript-view.js'
import type { ProcessItem, ProcessRun } from './transcript-view.js'
import type { SebasTranscriptView } from './transcript-view.js'

// ---- markdown mock（4.3 计数面）-----------------------------------------
// 全文件以可计数的替身替换 markdown 管线：正文断言只依赖 textContent（
// `<p>${source}</p>` 足够），渲染成本断言（流式帧期间调用次数持平）依赖
// mock 的调用记录。
vi.mock('../components/markdown.js', () => ({
  renderMarkdown: vi.fn((source: string) => `<p>${source}</p>`),
}))
import { renderMarkdown } from '../components/markdown.js'

// ---- localStorage polyfill --------------------------------------------
// A tiny in-memory Map-shaped object replaces the host's `localStorage`.
// Clearing per-test is the responsibility of the `beforeEach` below; we
// don't auto-clear on each get/set so individual tests can assert on
// values that survive across renders.

const store = new Map<string, string>()
beforeEach(() => store.clear())

const ls = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => {
    store.set(k, v)
  },
  removeItem: (k: string) => {
    store.delete(k)
  },
  clear: () => store.clear(),
  key: () => null,
  get length() {
    return store.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })

// ---- shared-ws mock -----------------------------------------------------
// The component subscribes to the shared WS client for `turn.append`
// frames (the incremental-sync path). The mock replaces the real client
// (which would dial a socket from jsdom) with a handler sink so tests can
// emit streaming frames deterministically: position dedup (sync cursor),
// session filtering, live fold summary, live receipt clearing, and the
// queued-submission turn boundary all ride this path in production.
const wsMock = vi.hoisted(() => {
  const handlers = new Set<(event: unknown) => void>()
  return {
    handlers,
    subscribe: (handler: (event: unknown) => void) => {
      handlers.add(handler)
      return () => handlers.delete(handler)
    },
  }
})
vi.mock('../api/shared-ws.js', () => ({ sharedWs: { subscribe: wsMock.subscribe } }))

/** Emit a `turn.append` frame to every live subscriber (mounted views). */
function emitTurnAppend(sessionId: string, entries: ConversationEntryView[]): void {
  const seq = entries.reduce((m, e) => Math.max(m, e.position), 0)
  for (const handler of wsMock.handlers) {
    handler({ type: 'turn.append', session_id: sessionId, entries, seq })
  }
}

// ---- component import -------------------------------------------------
// Importing the module side-effect registers `<sebas-transcript-view>`
// via the @customElement decorator and pulls in renderMarkdown (which
// reaches for `document.createElement` — that's fine in jsdom).

import './transcript-view.js'

// ---- helpers ----------------------------------------------------------

async function mount(opts: {
  entries: ConversationEntryView[]
  sessionKey?: string
  msgCount?: number
  agentDisplay?: string | null
  turnLive?: boolean
}): Promise<SebasTranscriptView> {
  const el = document.createElement('sebas-transcript-view') as SebasTranscriptView
  el.entries = opts.entries
  el.sessionKey = opts.sessionKey ?? 'oc_test'
  if (opts.msgCount !== undefined) el.msgCount = opts.msgCount
  if (opts.agentDisplay !== undefined) el.agentDisplay = opts.agentDisplay
  if (opts.turnLive !== undefined) el.turnLive = opts.turnLive
  document.body.appendChild(el)
  // Lit schedules its first update asynchronously; then the
  // component's `requestAnimationFrame(() => applyAutoScroll())` runs
  // after the layout flush. We yield a few times so both settle.
  await el.updateComplete
  await new Promise((r) => requestAnimationFrame(() => r(null)))
  await el.updateComplete
  return el
}

afterEach(() => {
  document.body.innerHTML = ''
})

const FIXED_DATES: Record<string, number> = {
  // 2025-01-01T12:00:00Z, 2025-01-01T12:00:01Z, ...
  T1: 1735732800,
  T2: 1735732801,
  T3: 1735732802,
  T4: 1735732803,
  T5: 1735732804,
}

function entry(e: Partial<ConversationEntryView>): ConversationEntryView {
  return {
    position: 0,
    kind: 'content',
    element_type: 'markdown',
    content: '',
    created_at_unix: 0,
    ...e,
  }
}

/** A streamed agent turn: prompt + N text chunks (the wire's chunk level). */
function streamedTurn(prompt: string, chunks: string[], startAt: number): ConversationEntryView[] {
  const out: ConversationEntryView[] = [
    entry({ position: 0, kind: 'prompt', content: prompt, created_at_unix: startAt }),
  ]
  chunks.forEach((c, i) =>
    out.push(
      entry({
        position: i + 1,
        kind: 'content',
        content: c,
        created_at_unix: startAt,
      }),
    ),
  )
  return out
}

/** An agent turn mixing text and process entries around a prompt. */
function mixedTurnEntries(): ConversationEntryView[] {
  return [
    entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 2, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 4, kind: 'content', content: 'mid text.', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 5, kind: 'content', element_type: 'thinking', content: 'reconsider', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 6, kind: 'content', element_type: 'tool', content: '✓ **bash**', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 7, kind: 'content', content: 'final.', created_at_unix: FIXED_DATES.T3 }),
  ]
}

// ---- pure-function coverage -------------------------------------------

describe('splitAgentRuns (D1, 1.1)', () => {
  it('merges contiguous same-kind entries: text concatenates, process accumulates', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, content: 'a' }),
      entry({ position: 2, content: 'b' }),
      entry({ position: 3, element_type: 'tool', content: '📖 **read**' }),
      entry({ position: 4, element_type: 'tool', content: '✓ **read**' }),
      entry({ position: 5, content: 'c' }),
    ])
    expect(runs.map((r) => r.type)).toEqual(['text', 'process', 'text'])
    const [t1, proc, t2] = runs
    if (t1.type !== 'text' || proc.type !== 'process' || t2.type !== 'text') {
      return expect.unreachable()
    }
    expect(t1.content).toBe('ab')
    expect(t1.position).toBe(1)
    expect(proc.items.map((it) => it.position)).toEqual([3, 4])
    expect(proc.position).toBe(3)
    expect(t2.content).toBe('c')
    expect(t2.position).toBe(5)
  })

  it('alternates at every kind change — 正文-过程-正文交错', () => {
    const runs = splitAgentRuns(mixedTurnEntries().filter((e) => e.kind === 'content'))
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process', 'text', 'process', 'text'])
    const procs = runs.flatMap((r) => (r.type === 'process' ? [r] : []))
    expect(procs.map((r) => r.position)).toEqual([1, 3, 5])
    expect(procs[2].items.map((it) => it.position)).toEqual([5, 6])
    expect(procs[2].items.map((it) => it.elementType)).toEqual(['thinking', 'tool'])
  })

  it('thinking and tool are the same kind — one run, both inside', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, element_type: 'thinking', content: 'hmm' }),
      entry({ position: 2, element_type: 'tool', content: '📖 **read**' }),
    ])
    expect(runs).toHaveLength(1)
    if (runs[0].type !== 'process') return expect.unreachable()
    expect(runs[0].items.map((it) => it.elementType)).toEqual(['thinking', 'tool'])
  })

  it('single entries keep their run shape', () => {
    expect(splitAgentRuns([entry({ position: 1, content: 'only' })].map((e) => e))).toEqual([
      { type: 'text', content: 'only', position: 1 },
    ])
    const proc = splitAgentRuns([entry({ position: 1, element_type: 'tool', content: 'x', title: 't' })])
    if (proc[0].type !== 'process') return expect.unreachable()
    expect(proc[0].items[0]).toEqual({
      elementType: 'tool',
      content: 'x',
      title: 't',
      position: 1,
    } satisfies ProcessItem)
  })
})

describe('groupConversation (run model)', () => {
  it('one agent turn from N chunks — turn grouping, not entry grouping', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('do it', ['a', 'b', 'c', 'd'], FIXED_DATES.T1)),
    )
    expect(units).toHaveLength(2)
    expect(units[0].kind).toBe('operator')
    expect(units[1].kind).toBe('agent')
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    const text = agent.runs[0]
    expect(text.type).toBe('text')
    if (text.type !== 'text') return expect.unreachable()
    expect(text.content).toBe('abcd')
  })

  it('text → tool → text: the process run sits between the text segments at its arrival position (2.1)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'let me check.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', content: 'done.', created_at_unix: FIXED_DATES.T2 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units).toHaveLength(2)
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.runs.map((r) => r.type)).toEqual(['text', 'process', 'text'])
    const proc = agent.runs[1]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.position).toBe(2)
    expect(proc.items.map((it) => it.position)).toEqual([2, 3])
  })

  it('a multi-run turn alternates runs at their arrival positions (2.1)', () => {
    const units = groupConversation(mergeSpawnErrors(mixedTurnEntries()))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    // 时间序：每个过程 run 折叠在自己的发生位置，不再是回合级单折叠。
    expect(agent.runs.map((r) => r.type)).toEqual([
      'process',
      'text',
      'process',
      'text',
      'process',
      'text',
    ])
    const procs = agent.runs.flatMap((r) => (r.type === 'process' ? [r] : []))
    expect(procs.map((r) => r.position)).toEqual([1, 3, 5])
    expect(procs[0].items.map((it) => it.position)).toEqual([1])
    expect(procs[1].items.map((it) => it.position)).toEqual([3])
    expect(procs[2].items.map((it) => it.position)).toEqual([5, 6])
    const texts = agent.runs.flatMap((r) => (r.type === 'text' ? [r.content] : []))
    expect(texts).toEqual(['step one.', 'mid text.', 'final.'])
  })

  it('a pure-text turn produces no process fold (2.1)', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('hi', ['a', 'b', 'c'], FIXED_DATES.T1)),
    )
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.runs.map((r) => r.type)).toEqual(['text'])
  })

  it('carries the entry title into process items; missing title tolerates legacy entries (2.2)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'tool', content: '📖 **read**', title: 'read · src/main.rs', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    const proc = agent.runs[0]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.items[0].title).toBe('read · src/main.rs')
    expect(proc.items[1].title).toBeNull()
  })

  it('conversation sides alternate in transcript order', () => {
    const entries = [
      ...streamedTurn('first', ['hello'], FIXED_DATES.T1),
      ...streamedTurn('second', ['world'], FIXED_DATES.T3).map((e, i) => ({ ...e, position: 2 + i })),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'operator', 'agent'])
  })

  it('error entries stay standalone units (fail-fast 3.3)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'error'])
  })

  it('an error mid-turn splits the agent turn and never joins a run (D5, 1.1)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'before.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', content: 'after.', created_at_unix: FIXED_DATES.T2 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'error', 'agent'])
    for (const u of units) {
      if (u.kind !== 'agent') continue
      expect(u.runs.map((r) => r.type)).toEqual(['text'])
    }
  })
})

describe('process run ids (D2, 1.2)', () => {
  it('the run id is the first entry position and stays stable across regrouping', () => {
    const base = mixedTurnEntries()
    const ids = (entries: ConversationEntryView[]): number[] =>
      groupConversation(mergeSpawnErrors(entries)).flatMap((u) =>
        u.kind === 'agent'
          ? u.runs.flatMap((r) => (r.type === 'process' ? [r.position] : []))
          : [],
      )
    const before = ids(base)
    expect(before).toEqual([1, 3, 5])
    // 流式追加（尾部文本段后新开一个过程 run）：既有 run 的 id 全部原位
    // 不动，新 run 拿自己的首条目 position——折叠展开状态据此跨重渲染恢复。
    const after = ids([
      ...base,
      entry({ position: 8, kind: 'content', element_type: 'tool', content: '✓ **edit**', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 9, kind: 'content', content: 'tail.', created_at_unix: FIXED_DATES.T3 }),
    ])
    expect(after).toEqual([1, 3, 5, 8])
  })

  it('run id always equals its first item position', () => {
    const runs = groupConversation(
      mergeSpawnErrors(mixedTurnEntries()),
    ).flatMap((u) => (u.kind === 'agent' ? u.runs : []))
    for (const r of runs) {
      if (r.type !== 'process') continue
      expect(r.position).toBe(r.items[0]?.position)
    }
  })
})

describe('processRunSummary (D3)', () => {
  const run = (items: ProcessItem[]): ProcessRun => ({ type: 'process', items, position: items[0]?.position ?? 0 })

  it('reflects the running (last) entry: structured title wins', () => {
    const s = processRunSummary(
      run([
        { elementType: 'thinking', content: 'hmm', title: null, position: 1 },
        { elementType: 'tool', content: '📖 **read**', title: 'read · src/main.rs', position: 2 },
      ]),
    )
    expect(s.label).toBe('read · src/main.rs')
    expect(s.full).toBe('read · src/main.rs')
  })

  it('falls back to the generic element-type label for untitled entries', () => {
    const s = processRunSummary(
      run([{ elementType: 'tool', content: '✓ **bash**', title: null, position: 3 }]),
    )
    expect(s.label).toBe('tool')
    expect(s.full).toBeNull()
  })

  it('as entries stream in, the moving tail changes the summary', () => {
    const r = run([{ elementType: 'thinking', content: 'hmm', title: null, position: 1 }])
    expect(processRunSummary(r).label).toBe('thinking')
    r.items.push({ elementType: 'tool', content: 'x', title: 'write · a.rs', position: 2 })
    expect(processRunSummary(r).label).toBe('write · a.rs')
  })
})

describe('process honesty for denied tool results (polish-workbench-walkthrough-ux 5.6)', () => {
  const run = (items: ProcessItem[]): ProcessRun => ({ type: 'process', items, position: items[0]?.position ?? 0 })

  it('toolResultDenied: 明确拒绝记号判定（denied / 已拒绝 / ❌），不猜工具语义', () => {
    expect(toolResultDenied('Bash command rejected by user', 'bash')).toBe(true)
    expect(toolResultDenied('已拒绝执行 rm -rf /', null)).toBe(true)
    expect(toolResultDenied('❌ **bash**', 'bash')).toBe(true)
    // 正常成功结果不带拒绝记号。
    expect(toolResultDenied('✓ **bash**', 'bash')).toBe(false)
    expect(toolResultDenied('done in 1.2s', null)).toBe(false)
    // title 里的拒绝记号同样命中。
    expect(toolResultDenied('whatever', 'read · denied')).toBe(true)
  })

  it('deniedLabel: 去掉成功 ✓ 前缀，改 ✗', () => {
    expect(deniedLabel('✓ bash')).toBe('✗ bash')
    expect(deniedLabel('✓ **bash**')).toBe('✗ **bash**')
    // 无 ✓ 前缀的标签原样挂 ✗。
    expect(deniedLabel('bash')).toBe('✗ bash')
  })

  it('processItemLabel: 拒绝条目不再挂 ✓', () => {
    const denied = processItemLabel({
      elementType: 'tool',
      content: 'denied by operator',
      title: '✓ bash',
      position: 1,
    })
    expect(denied.label).toBe('✗ bash')
    expect(denied.full).toBe('bash')
    const ok = processItemLabel({
      elementType: 'tool',
      content: '✓ **bash**',
      title: 'bash · build',
      position: 2,
    })
    expect(ok.label).toBe('bash · build')
  })

  it('processRunDenied: run 内任一条目被拒即整行不显示成功语义', () => {
    const r = run([
      { elementType: 'tool', content: 'ok', title: 'bash · one', position: 1 },
      { elementType: 'tool', content: '已拒绝', title: 'bash · two', position: 2 },
    ])
    expect(processRunDenied(r)).toBe(true)
    expect(
      processRunDenied(run([{ elementType: 'tool', content: '✓ **bash**', title: null, position: 1 }])),
    ).toBe(false)
  })
})

describe('mergeSpawnErrors', () => {
  const SPAWN_ERR = '**spawn failed**: agent binary missing'
  const errEntry = (at: number, content = SPAWN_ERR): ConversationEntryView =>
    entry({ kind: 'content', element_type: 'error', content, created_at_unix: at })

  it('merges adjacent identical errors within the window into one counted entry', () => {
    const merged = mergeSpawnErrors([errEntry(100, SPAWN_ERR), errEntry(100 + ERROR_MERGE_WINDOW_SECS - 1, SPAWN_ERR)])
    expect(merged).toHaveLength(1)
    expect(merged[0].count).toBe(2)
    expect(merged[0].created_at_unix).toBe(100)
  })

  it('keeps identical errors outside the window as separate entries', () => {
    const split = mergeSpawnErrors([errEntry(100), errEntry(100 + ERROR_MERGE_WINDOW_SECS + 1)])
    expect(split).toHaveLength(2)
    expect(split.every((e) => e.count === 1)).toBe(true)
  })

  it('does not merge errors with different reasons; non-errors reset adjacency', () => {
    const A = 'err-a'
    const split = mergeSpawnErrors([errEntry(100, A), errEntry(101, 'other reason')])
    expect(split).toHaveLength(2)
    const reset = mergeSpawnErrors([
      errEntry(100, A),
      entry({ content: 'x', created_at_unix: 101 }),
      errEntry(102, A),
    ])
    expect(reset).toHaveLength(3)
    expect(reset.filter((e) => e.element_type === 'error').every((e) => e.count === 1)).toBe(true)
  })
})

// ---- middle truncation (2.3) ---------------------------------------------

describe('middleTruncate (2.3)', () => {
  const graphemeCount = (s: string): number => {
    const seg = new Intl.Segmenter('en', { granularity: 'grapheme' })
    return [...seg.segment(s)].length
  }

  it('keeps short strings (and strings at the 64-char boundary) as-is', () => {
    expect(middleTruncate('read · src/main.rs')).toBe('read · src/main.rs')
    expect(middleTruncate('a'.repeat(64))).toBe('a'.repeat(64))
  })

  it('collapses the middle of long strings into head 28 + … + tail 28', () => {
    expect(middleTruncate('a'.repeat(65))).toBe('a'.repeat(28) + '…' + 'a'.repeat(28))
    const t = middleTruncate('x'.repeat(200))
    expect(graphemeCount(t)).toBe(28 + 1 + 28)
    expect(t).toContain('…')
  })

  it('never splits multi-byte characters — Chinese, ZWJ family emoji, flags', () => {
    // 中文：按字（码点）切，绝无半个字符的替换符。
    const han = '汉字测试'.repeat(20) // 80 chars > 64
    const t1 = middleTruncate(han)
    expect(t1).not.toContain('\uFFFD')
    expect(t1.startsWith('汉字测试')).toBe(true)
    expect(t1.endsWith('汉字测试')).toBe(true)
    expect(graphemeCount(t1)).toBe(57)
    // ZWJ 家庭 emoji：一个字素 = 多个码点，截断不得把它切成两半。
    const family = '👨‍👩‍👧‍👦'
    const t2 = middleTruncate(family.repeat(70))
    expect(graphemeCount(t2)).toBe(57)
    expect(t2).toContain(family)
    expect(t2.startsWith(family)).toBe(true)
    // 旗标：两个区域指示符为一个字素。
    const flag = '🇨🇳'
    const t3 = middleTruncate(flag.repeat(70))
    expect(graphemeCount(t3)).toBe(57)
    expect(t3).toContain(flag)
  })
})

describe('assistant author label fallback chain (3.1, D1)', () => {
  it('resolves display → slug → assistant in order', () => {
    expect(resolveAgentDisplay('Claude Code')).toBe('Claude Code')
    expect(resolveAgentDisplay('claude-code')).toBe('claude-code')
    expect(resolveAgentDisplay(null)).toBe('assistant')
    expect(resolveAgentDisplay(undefined)).toBe('assistant')
    expect(resolveAgentDisplay('   ')).toBe('assistant')
  })

  it('derives the badge purely from the entry sequence, no Working gate (3.2)', () => {
    const promptOnly = groupConversation(
      mergeSpawnErrors(streamedTurn('q', [], FIXED_DATES.T1)),
    )
    // 排队窗：首个流式事件前会话显示 queued（非 working），角标仍须出现。
    expect(awaitingReceipt(promptOnly)).toBe(true)
    const withReply = groupConversation(
      mergeSpawnErrors(streamedTurn('q', ['answer'], FIXED_DATES.T1)),
    )
    // agent entry 一到（会话此时才翻 working）角标即消失。
    expect(awaitingReceipt(withReply)).toBe(false)
  })
})

// ---- rendered-component coverage ---------------------------------------

describe('sebas-transcript-view (conversation rendering)', () => {
  it('renders N streamed chunks as ONE natural-flow turn, not per-chunk bubbles (2.1)', async () => {
    const el = await mount({
      entries: streamedTurn('do it', ['chunk one ', 'chunk two ', 'chunk three'], FIXED_DATES.T1),
    })
    const assistant = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(assistant?.length).toBe(1)
    const flow = assistant?.[0]?.querySelector<HTMLElement>('.flow')
    expect(flow).toBeTruthy()
    const bodies = flow?.querySelectorAll<HTMLElement>('.body')
    expect(bodies?.length).toBe(1)
    expect(bodies?.[0]?.textContent).toContain('chunk one')
    expect(bodies?.[0]?.textContent).toContain('chunk three')
  })

  it('renders both conversation sides alternating with submission text (2.3)', async () => {
    const entries = [
      ...streamedTurn('first question', ['hello ', 'world'], FIXED_DATES.T1),
      ...streamedTurn('second question', ['again'], FIXED_DATES.T3).map((e, i) => ({
        ...e,
        position: 4 + i,
      })),
    ]
    const el = await mount({ entries })
    const blocks = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block')
    expect(blocks?.length).toBe(4)
    const classes = Array.from(blocks ?? []).map((b) => b.classList.contains('is-user'))
    expect(classes).toEqual([true, false, true, false])
    const firstUser = blocks?.[0]?.querySelector<HTMLElement>('.body p')
    expect(firstUser?.textContent).toBe('first question')
    expect(blocks?.[2]?.querySelector<HTMLElement>('.body p')?.textContent).toBe('second question')
    // 「你」头像与「你」作者标注（5.1 统一 zh-CN，双语混排移除）。
    expect(blocks?.[0]?.querySelector('.avatar.user')?.textContent).toBe('你')
    expect(blocks?.[0]?.querySelector('.author.you')?.textContent).toBe('你')
  })

  it('mixed turn renders one fold PER process run, each at its arrival position (2.1)', async () => {
    const el = await mount({ entries: mixedTurnEntries() })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 时间序切分：三个过程 run = 三个折叠，各在自己的发生位置。
    const folds = assistant.querySelectorAll<HTMLElement>('.process-fold')
    expect(folds.length).toBe(3)
    expect([...folds].map((f) => f.dataset.processId)).toEqual(['1', '3', '5'])
    expect([...folds].map((f) => f.getAttribute('data-process-count'))).toEqual(['1', '1', '2'])
    // 文本段按流序留在折叠外（三段）。
    const segments = assistant.querySelectorAll<HTMLElement>('.flow > .body:not(.fold-body)')
    expect(segments.length).toBe(3)
    expect(segments[0].textContent).toContain('step one.')
    expect(segments[1].textContent).toContain('mid text.')
    expect(segments[2].textContent).toContain('final.')
  })

  it('pure-text turn renders no fold at all (2.1)', async () => {
    const el = await mount({
      entries: streamedTurn('do it', ['chunk one ', 'chunk two '], FIXED_DATES.T1),
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    expect(assistant.querySelector('.process-fold, .process-item')).toBeNull()
    const bodies = assistant.querySelectorAll<HTMLElement>('.flow > .body')
    expect(bodies.length).toBe(1)
    expect(bodies[0].textContent).toContain('chunk one')
  })

  it('fold affordances are lightweight inline links with no details chrome (4.4)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'deep thought', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // spec「fold affordance is a lightweight link」：折叠态 DOM 中 link 控件
    // 存在（glyph + 标签 + 计数），且不再有 details/summary 卡框结构。
    expect(assistant.querySelector('details, summary')).toBeNull()
    const link = assistant.querySelector<HTMLButtonElement>('button.fold-link')!
    expect(link.getAttribute('aria-expanded')).toBe('false')
    expect(link.querySelector('.kind-icon')).toBeTruthy()
    expect(link.querySelector('.label')?.textContent?.trim()).toBe('process')
    expect(link.querySelector('.fold-count')?.textContent?.trim()).toBe('1')
    // 样式面：折叠容器无边框/无底色卡框（link 化的外观合同）。
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    const foldRule = styleText.match(/\.turn-block \.process-fold\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(foldRule).not.toMatch(/\bborder:/)
    expect(foldRule).not.toMatch(/\bbackground:/)
  })

  it('process folds and second-level folds collapse by default; titles show with generic fallback (2.1/2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'deep thought', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '📖 **read_file**', title: 'read_file · src/app.ts', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **read_file**', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    const fold = assistant.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.getAttribute('data-process-count')).toBe('3')
    // 折叠行：固定 process 标签 + 实时摘要（进行中条目 = run 尾部，此处
    // 是无 title 的 tool → 通用标签）+ 条目计数。
    const link = fold.querySelector<HTMLButtonElement>('button.fold-link')!
    expect(link.querySelector('.label')?.textContent?.trim()).toBe('process')
    expect(link.querySelector('.running')?.textContent?.trim()).toBe('tool')
    // run 尾条目无 title → 通用标签、link 不带 title 属性。
    expect(link.hasAttribute('title')).toBe(false)
    expect(link.querySelector('.fold-count')?.textContent?.trim()).toBe('3')
    // 外层默认收起（无展开体——二级折叠只在展开体内渲染，收起态不残留
    // 隐藏 DOM）；展开后二级全部默认收起。
    expect(link.getAttribute('aria-expanded')).toBe('false')
    expect(fold.querySelector('.fold-body')).toBeNull()
    link.click()
    await el.updateComplete
    const items = [...assistant.querySelectorAll<HTMLElement>('.process-item')]
    expect(items.length).toBe(3)
    items.forEach((d) =>
      expect(d.querySelector<HTMLButtonElement>('button.fold-link')!.getAttribute('aria-expanded')).toBe('false'),
    )
    // 二级 link：有 title 显示 title 且 title 属性保全量；无 title 回退
    // 通用标签（thinking / tool）且不带 title 属性。
    expect(items[0].dataset.position).toBe('1')
    expect(items[0].querySelector('.item-title')?.textContent?.trim()).toBe('thinking')
    expect(items[0].querySelector('button.fold-link')!.hasAttribute('title')).toBe(false)
    expect(items[1].querySelector('.item-title')?.textContent?.trim()).toBe(
      'read_file · src/app.ts',
    )
    expect(items[1].querySelector('button.fold-link')!.getAttribute('title')).toBe(
      'read_file · src/app.ts',
    )
    expect(items[2].querySelector('.item-title')?.textContent?.trim()).toBe('tool')
    // 展开层级：逐条点击二级 link 才展开（键盘同路径——原生 button 激活）。
    const itemLinks = [...assistant.querySelectorAll<HTMLButtonElement>('.process-item button.fold-link')]
    itemLinks.forEach((b) => expect(b.getAttribute('aria-expanded')).toBe('false'))
    itemLinks[1].click()
    await el.updateComplete
    expect(itemLinks[1].getAttribute('aria-expanded')).toBe('true')
    expect(items[1].textContent).toContain('read_file')
    // 其余二级折叠不受影响。
    expect(itemLinks[0].getAttribute('aria-expanded')).toBe('false')
    expect(itemLinks[2].getAttribute('aria-expanded')).toBe('false')
    // 再点一次同一 link：收起（点击切换展开/收起）；外层同理。
    link.click()
    await el.updateComplete
    expect(link.getAttribute('aria-expanded')).toBe('false')
    expect(fold.querySelector('.fold-body')).toBeNull()
  })

  it('the fold summary tracks the running tool and entry count as entries stream in (D3, 2.1)', async () => {
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    const linkOf = () =>
      el.shadowRoot!.querySelector<HTMLElement>('.process-fold button.fold-link')!
    expect(el.shadowRoot!.querySelector('.process-fold')!.getAttribute('data-process-count')).toBe('1')
    expect(linkOf().querySelector('.running')?.textContent?.trim()).toBe('thinking')
    expect(linkOf().querySelector('.fold-count')?.textContent?.trim()).toBe('1')
    // 流式增量到达（快照收敛路径重分组）：同一 run 的摘要实时刷新——
    // 进行中的工具 title + 累计条目数。
    el.entries = [
      ...base,
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(linkOf().querySelector('.running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(linkOf().getAttribute('title')).toBe('bash · deploy.sh')
    expect(linkOf().querySelector('.fold-count')?.textContent?.trim()).toBe('2')
  })

  it('a collapsed fold never opens by itself while its entries stream in (D3, 2.1)', async () => {
    // spec 场景「folds stay collapsed with a live summary while streaming」
    // 的收起半边：折叠默认收起，流式条目连续落进同一 run 时不得自开——
    // 摘要照常实时刷新（进行中工具 title + 累计计数）。
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    expect(
      el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!
        .getAttribute('aria-expanded'),
    ).toBe('false')
    el.entries = [
      ...base,
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **bash**', title: 'bash · verify.sh', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.querySelector('.fold-body')).toBeNull()
    expect(fold.getAttribute('data-process-count')).toBe('3')
    expect(fold.querySelector('.running')?.textContent?.trim()).toBe('bash · verify.sh')
  })

  it('an expanded fold stays open and appends streamed entries in place (D2/D3, 2.1)', async () => {
    // 夹具让过程 run 收尾（run 是尾部 run），流式追加的过程条目才会并入
    // 同一 run——这正是「展开后新条目就地追加」的场景。
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    const fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    fold.querySelector<HTMLButtonElement>('button.fold-link')!.click()
    await el.updateComplete
    expect(fold.querySelector('.fold-body')).toBeTruthy()
    // 流式全量重分组（快照收敛）：新过程条目并入同一 run——折叠保持展开、
    // 新条目就地追加。
    el.entries = [
      ...base,
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', title: 'read · src/lib.rs', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const fold2 = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold2.dataset.processId).toBe('2')
    expect(fold2.querySelector('.fold-body')).toBeTruthy()
    expect(fold2.getAttribute('data-process-count')).toBe('2')
    const items = fold2.querySelectorAll<HTMLElement>('.process-item')
    expect(items.length).toBe(2)
    expect(items[1].dataset.position).toBe('3')
  })

  it('a fold that appears later starts collapsed while an expanded sibling stays open (D2/D3)', async () => {
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    const first = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    first.querySelector<HTMLButtonElement>('button.fold-link')!.click()
    await el.updateComplete
    expect(first.querySelector('.fold-body')).toBeTruthy()
    // 流式追加出第二个过程 run：新折叠默认收起，已展开的不受影响。
    el.entries = [
      ...base,
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 4, kind: 'content', content: 'after.', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const folds = el.shadowRoot!.querySelectorAll<HTMLElement>('.process-fold')
    expect(folds.length).toBe(2)
    expect(folds[0].dataset.processId).toBe('1')
    expect(folds[0].querySelector('.fold-body')).toBeTruthy()
    expect(folds[1].dataset.processId).toBe('3')
    expect(folds[1].querySelector('.fold-body')).toBeNull()
  })

  it('second-level folds keep position-keyed DOM identity (2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', element_type: 'thinking', content: 'a', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 9, kind: 'content', element_type: 'tool', content: 'b', title: 'read · x', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 17, kind: 'content', element_type: 'tool', content: 'c', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries })
    // 二级折叠在展开体内：先展开外层折叠。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const items = el.shadowRoot!.querySelectorAll<HTMLElement>('.process-item')
    expect([...items].map((d) => d.dataset.position)).toEqual(['4', '9', '17'])
  })

  it('long titles are middle-truncated in the summary while the title attribute keeps the full string (2.3)', async () => {
    const long = 'read · ' + 'src/very/deep/nested/path/segments/that/keep/going/on/and/on/module/impl/main.rs'
    expect(long.length).toBeGreaterThan(64)
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'tool', content: '📖 **read**', title: long, created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries })
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const link = el.shadowRoot!.querySelector<HTMLButtonElement>('.process-item button.fold-link')!
    const shown = link.querySelector<HTMLElement>('.item-title')?.textContent?.trim()
    expect(shown).toBe(middleTruncate(long))
    expect(shown).toContain('…')
    expect(link.getAttribute('title')).toBe(long)
  })

  it('agent side drops the card shell; user side keeps the tinted block; errors keep their card (2.2)', async () => {
    const el = await mount({
      entries: streamedTurn('do it', ['answer'], FIXED_DATES.T1),
    })
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // agent 侧：裸排流容器在、卡片壳不在。
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    expect(assistant.querySelector('.flow')).toBeTruthy()
    expect(assistant.querySelector('.bubble')).toBeNull()
    // 用户侧：轻底色块保留 tinted 背景、无边框阴影。
    const user = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-user')!
    expect(user.querySelector('.msg-block')).toBeTruthy()
    expect(styleText).toMatch(
      /\.turn-block \.msg-block\s*\{[^}]*background:\s*var\(--sebas-accent-soft\)/,
    )
    expect(styleText).toMatch(/\.turn-block \.msg-block\s*\{[^}]*border-radius:/)
    expect(styleText).not.toContain('.turn-block.is-user .bubble')
    // 错误气泡保留计数卡片形态（D5）。
    const errEl = await mount({
      entries: [
        entry({ kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    expect(errEl.shadowRoot!.querySelector('.turn-block.is-error .bubble')).toBeTruthy()
  })

  it('user block keeps the tint but carries no card chrome — no border, no shadow (2.2/D4)', async () => {
    // spec「lightly tinted block without card chrome (no border or shadow)」：
    // .msg-block 规则体只许有底色/圆角——卡片边框与阴影不得回归。
    const el = await mount({ entries: streamedTurn('hi', ['hello'], FIXED_DATES.T1) })
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // 命中 .msg-block 的全部规则体（含与 .flow 合写的布局规则）——任何一条
    // 都不许给用户色块加回卡片边框或阴影。
    const rules = [
      ...styleText.matchAll(/\.turn-block [^{]*\.msg-block[^{]*\{([^}]*)\}/g),
    ].map((m) => m[1])
    expect(rules.length).toBeGreaterThanOrEqual(2)
    const joined = rules.join('\n')
    expect(joined).toContain('background: var(--sebas-accent-soft)')
    expect(joined).toMatch(/\bborder-radius:/) // 圆角保留
    expect(joined).not.toMatch(/\bborder:\s/) // 卡片边框不得回归
    expect(joined).not.toContain('box-shadow')
  })

  it('a multi-chunk unseen turn counts as ONE unseen turn and the seam never splits it (2.4)', async () => {
    const entries = [
      ...streamedTurn('old', ['seen'], FIXED_DATES.T1),
      ...streamedTurn('new', ['a', 'b', 'c', 'd', 'e', 'f', 'g'], FIXED_DATES.T3).map((e, i) => ({
        ...e,
        position: 2 + i,
        created_at_unix: FIXED_DATES.T4,
      })),
    ]
    // （2.3，D3）段锚：读锚 = 「old」回合的 1 个可见段（7 条 chunk 无论如何
    // 合并只计 1 段——徽标与 seam 同一口径）。未读内容从第二个 agent 回合起。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
    const el = await mount({ entries })
    // 边界下方只有 1 个回合（new 的 agent 回合）：操作者自己的 prompt 不是
    // 未读内容（段口径不计 prompt）；agent 回合无论多少条 chunk 只计 1。
    expect((el as unknown as { unseenCount: number }).unseenCount).toBe(1)
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    expect(seam?.textContent).toContain('~1 new')
    // 边界落在第一个「可见段累计超过锚」的回合（new 的 agent 回合）上方，
    // 不切开任何回合；seam 与徽标读同一条段锚（2.3，D3）。
    const seamNext = seam?.nextElementSibling
    expect(seamNext?.classList.contains('is-assistant')).toBe(true)
    const agentBlocks = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(agentBlocks?.length).toBe(2)
    expect(agentBlocks?.[1]?.textContent).toContain('a')
    expect(agentBlocks?.[1]?.textContent).toContain('g')
  })

  it('a legacy seen_ts-shaped cursor reads as fully read and is overwritten by a pure anchor on first write (2.3)', async () => {
    // 旧 localStorage 形态（含 seen_ts）：首次读取按「无 anchor」对待（读为
    // fully-read、不迁移），随后一次 mark-seen 覆写成纯 {anchor_count}。
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    store.set(
      'sebas:seen:oc_test',
      JSON.stringify({ seen_ts: FIXED_DATES.T1, anchor_count: null }),
    )
    const el = await mount({ entries })
    await el.updateComplete
    // 旧数据 = fully read：无 seam、无未读计数。
    expect(el.shadowRoot?.querySelector<HTMLElement>('.seam')?.hasAttribute('hidden')).toBe(true)
    // 首次写入（贴底 mark-seen 路径）→ 纯单字段形状覆写。
    const scroll = el.shadowRoot?.querySelector<HTMLElement>('.scroll')!
    scroll.scrollTop = scroll.scrollHeight
    scroll.dispatchEvent(new Event('scroll'))
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 300))
    const raw = store.get('sebas:seen:oc_test')!
    const stored = JSON.parse(raw) as Record<string, unknown>
    expect(Object.keys(stored).sort()).toEqual(['anchor_count'])
    expect(stored.anchor_count).toBe(1)
  })

  it('no seam when everything is seen; mark-all-seen writes and hides', async () => {
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    // 两个相邻 md chunk 合并为一段：锚=1 = 全部已读，无 seam。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
    const el = await mount({ entries })
    // 4.2：seam 节点恒渲染（hidden 属性切换显隐）——全部已读时 hidden。
    let seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(true)

    // 未读态（锚=0）→ mark all seen → 游标写入当前段数、seam 消失。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    el.entries = [...entries]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el.updateComplete
    // （2.3，D3）单字段段锚：存储只有 anchor_count；无 msgCount payload 时
    // 写本地已渲染段数（相邻 md 合并 = 1）。
    const stored = JSON.parse(store.get('sebas:seen:oc_test')!) as {
      anchor_count: number
    }
    expect(Object.keys(stored)).toEqual(['anchor_count'])
    expect(stored.anchor_count).toBe(1)
    expect(el.shadowRoot?.querySelector<HTMLElement>('.seam')?.hasAttribute('hidden')).toBe(true)
  })

  it('mark-all-seen advances the shared badge anchor when msgCount rides the payload (D3)', async () => {
    // rail-declutter-unread：payload 带 msg_count 时，标记已读把共享游标的
    // 段数锚推进到 max(服务端段数, 本地已渲染段数)——读到底部 = seam 清零 +
    // 徽标清零（两者同锚）。
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    const el = await mount({ entries, msgCount: 3 })
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    el.entries = [...entries]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el.updateComplete
    const stored = JSON.parse(store.get('sebas:seen:oc_test')!) as {
      anchor_count: number
    }
    expect(stored.anchor_count).toBe(3)
  })

  it('timestamps render inside each meta row with datetime attrs', async () => {
    const el = await mount({
      entries: streamedTurn('hi', ['a', 'b'], FIXED_DATES.T1),
    })
    const times = el.shadowRoot?.querySelectorAll<HTMLTimeElement>(
      '.turn-block .meta time.time',
    )
    expect(times?.length).toBe(2)
    expect(times?.[0]?.getAttribute('datetime')).toBe(new Date(FIXED_DATES.T1 * 1000).toISOString())
    expect(times?.[1]?.getAttribute('datetime')).toBe(new Date(FIXED_DATES.T1 * 1000).toISOString())
  })

  it('skips entries with empty content (2.5)', async () => {
    const entries: ConversationEntryView[] = [
      entry({ position: 0, kind: 'prompt', content: '', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'kept', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries })
    const blocks = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block')
    expect(blocks?.length).toBe(1)
    expect(blocks?.[0]?.textContent).toContain('kept')
  })

  it('renders error entries as counted error bubbles, not assistant bubbles (2.5)', async () => {
    const el = await mount({
      entries: [
        entry({ kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const block = el.shadowRoot?.querySelector<HTMLElement>('.turn-block.is-error')
    expect(block).not.toBeNull()
    expect(block?.getAttribute('data-error-count')).toBe('1')
    expect(block?.textContent).toContain('spawn failed')
    expect(block?.querySelector('.meta .count')).toBeNull()
  })

  it('fill mode lifts the 58vh cap and flexes the scroll region (2.5)', async () => {
    const el = await mount({ entries: streamedTurn('hi', ['a'], FIXED_DATES.T1) })
    expect(el.hasAttribute('fill')).toBe(false)
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(styleText).toContain('58vh')
    const scroll = el.shadowRoot!.querySelector<HTMLElement>('.scroll')!
    el.fill = true
    await el.updateComplete
    expect(el.hasAttribute('fill')).toBe(true)
    expect(styleText).toContain(':host([fill])')
    expect(styleText).toMatch(/:host\(\[fill\]\)\s*\{[^}]*flex:\s*1/)
    expect(styleText).toMatch(/:host\(\[fill\]\)[\s\S]*max-height:\s*none/)
    expect(el.shadowRoot!.querySelector<HTMLElement>('.scroll') === scroll).toBe(true)
  })

  it('shows the catalog display name with a first-grapheme avatar (3.1)', async () => {
    const el = await mount({
      entries: streamedTurn('hi', ['hello'], FIXED_DATES.T1),
      agentDisplay: 'Claude Code',
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    expect(assistant.querySelector('.meta .author')?.textContent?.trim()).toBe('Claude Code')
    // 头像维持文本形态：展示名首字母。
    expect(assistant.querySelector('.avatar.assistant')?.textContent?.trim()).toBe('C')
  })

  it('falls back display → slug → assistant for the author label and avatar (3.1)', async () => {
    const entries = streamedTurn('hi', ['hello'], FIXED_DATES.T1)
    // 第二级：目录无 display → dashboard 传 raw slug。
    const slugEl = await mount({ entries, agentDisplay: 'claude-code' })
    const slugBlock = slugEl.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    expect(slugBlock.querySelector('.meta .author')?.textContent?.trim()).toBe('claude-code')
    expect(slugBlock.querySelector('.avatar.assistant')?.textContent?.trim()).toBe('C')
    // 第三级：目录不可得/未绑定 → 通用 assistant，头像保持既有 AI 形态。
    const genericEl = await mount({ entries, agentDisplay: null })
    const genericBlock = genericEl.shadowRoot!.querySelector<HTMLElement>(
      '.turn-block.is-assistant',
    )!
    expect(genericBlock.querySelector('.meta .author')?.textContent?.trim()).toBe('assistant')
    expect(genericBlock.querySelector('.avatar.assistant')?.textContent?.trim()).toBe('AI')
  })

  it('shows the receipt badge while the prompt is the newest entry — queued, not yet working (3.2)', async () => {
    // 纯 entry 序语义（spec「Operator submission receipt」）：live 流程里
    // 排队窗显示 queued（非 working），首个流式事件才翻 working——角标不
    // 依赖会话 Working 门，否则真实使用中永不出现。
    const el = await mount({
      entries: streamedTurn('waiting on the agent', [], FIXED_DATES.T4),
    })
    const badge = el.shadowRoot?.querySelector<HTMLElement>('[data-receipt]')
    expect(badge).toBeTruthy()
    expect(badge?.textContent).toContain('已收到')
    // 角标挂在最后一条操作者色块上。
    expect(badge?.closest('.turn-block')?.classList.contains('is-user')).toBe(true)
  })

  it('clears the receipt badge once the agent reply arrives (3.2)', async () => {
    const el = await mount({
      entries: streamedTurn('q', ['answer arrived'], FIXED_DATES.T1),
    })
    expect(el.shadowRoot?.querySelector('[data-receipt]')).toBeNull()
  })

  it('keeps the receipt badge cleared after the turn completes (DONE) (3.2)', async () => {
    // DONE 落定后收尾仍是 agent 回复条目——角标不因会话完成而复现。
    const el = await mount({
      entries: streamedTurn('q', ['final answer'], FIXED_DATES.T1),
    })
    expect(el.shadowRoot?.querySelector('[data-receipt]')).toBeNull()
  })

  it('badges only the newest prompt in an ongoing conversation (3.2)', async () => {
    const entries = [
      ...streamedTurn('first', ['hello'], FIXED_DATES.T1),
      entry({ position: 2, kind: 'prompt', content: 'second', created_at_unix: FIXED_DATES.T3 }),
    ]
    const el = await mount({ entries })
    const badges = el.shadowRoot!.querySelectorAll('[data-receipt]')
    expect(badges.length).toBe(1)
    const userBlocks = el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block.is-user')
    expect(userBlocks.length).toBe(2)
    expect(userBlocks[0].querySelector('[data-receipt]')).toBeNull()
    expect(userBlocks[1].querySelector('[data-receipt]')).toBeTruthy()
  })

  it('a queued submission starts its own turn; later output flows into the new turn (turn-start scenario)', async () => {
    // spec 场景「a submission appears when its turn starts」：第二条提交在
    // agent 仍在输出时被接受——它终结前一个 agent 回合，先以带 receipt
    // 角标的操作者色块出现；新输出开新回合，不并入上一回合。
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'first', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'answer one', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'prompt', content: 'second', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries: base })
    expect(el.shadowRoot!.querySelectorAll('.turn-block.is-assistant')).toHaveLength(1)
    const badges = el.shadowRoot!.querySelectorAll('[data-receipt]')
    expect(badges).toHaveLength(1)
    expect(badges[0].closest('.turn-block')?.textContent).toContain('second')
    // 排队中的提交后续回复到达：开新 agent 回合（与第一回合文本互不掺混），
    // receipt 随首个输出条目消失。
    emitTurnAppend('oc_test', [
      entry({ position: 3, kind: 'content', content: 'answer two', created_at_unix: FIXED_DATES.T3 }),
    ])
    await el.updateComplete
    const assistants = el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(assistants).toHaveLength(2)
    expect(assistants[0].textContent).toContain('answer one')
    expect(assistants[0].textContent).not.toContain('answer two')
    expect(assistants[1].textContent).toContain('answer two')
    expect(el.shadowRoot!.querySelector('[data-receipt]')).toBeNull()
  })

  it('clears the receipt badge live when the first reply entry streams in via turn.append (3.2)', async () => {
    // spec 场景「badge clears when the reply streams」的实况迁移：角标先在，
    // 首个 agent 输出条目经 WS 增量到达后当帧消失（非重挂载重放）。
    const el = await mount({
      entries: streamedTurn('waiting on the agent', [], FIXED_DATES.T4),
    })
    expect(el.shadowRoot?.querySelector('[data-receipt]')).toBeTruthy()
    emitTurnAppend('oc_test', [
      entry({ position: 1, kind: 'content', content: 'reply', created_at_unix: FIXED_DATES.T5 }),
    ])
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('[data-receipt]')).toBeNull()
    expect(el.shadowRoot!.querySelector('.turn-block.is-assistant')?.textContent).toContain('reply')
  })

  it('turn.append drives the live fold summary with position dedup and session filter (D3 + sync cursor)', async () => {
    // 增量同步游标面：turn.append 帧按 position 去重并入渲染管线；重复帧
    // 不重复计数、迟到旧帧被丢弃、非聚焦会话的帧被忽略；快照收敛后同一
    // run id / 同一摘要（游标协议与快照一致）。
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'let me check.', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    expect(el.shadowRoot!.querySelector('.process-fold')).toBeNull()

    // 帧 1：thinking 新开过程 run —— 折叠默认收起、摘要 = 通用标签、计数 1。
    emitTurnAppend('oc_test', [
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ])
    await el.updateComplete
    const linkOf = () =>
      el.shadowRoot!.querySelector<HTMLElement>('.process-fold button.fold-link')!
    let fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    expect(linkOf().getAttribute('aria-expanded')).toBe('false')
    expect(linkOf().querySelector('.running')?.textContent?.trim()).toBe('thinking')
    expect(linkOf().querySelector('.fold-count')?.textContent?.trim()).toBe('1')

    // 帧 2：工具条目并入同一 run —— 摘要实时切到进行中的结构化 title、
    // 计数 2、仍保持收起。
    emitTurnAppend('oc_test', [
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(linkOf().getAttribute('aria-expanded')).toBe('false')
    expect(linkOf().querySelector('.running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(linkOf().getAttribute('title')).toBe('bash · deploy.sh')
    expect(linkOf().querySelector('.fold-count')?.textContent?.trim()).toBe('2')

    // 游标去重：重复 position 的帧不重复计数、二级折叠不重复渲染
    // （展开折叠体后核对逐条渲染）。
    emitTurnAppend('oc_test', [
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.getAttribute('data-process-count')).toBe('2')
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.querySelectorAll('.process-item')).toHaveLength(2)
    // 收起复原（后续断言继续以收起态为准）。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete

    // 迟到的旧 position 帧被丢弃；非聚焦会话的帧被忽略。
    emitTurnAppend('oc_test', [
      entry({ position: 1, kind: 'content', content: 'STALE', created_at_unix: FIXED_DATES.T1 }),
    ])
    emitTurnAppend('oc_other', [
      entry({ position: 9, kind: 'content', element_type: 'tool', content: 'elsewhere', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(el.shadowRoot!.textContent).not.toContain('STALE')
    expect(el.shadowRoot!.textContent).not.toContain('elsewhere')

    // 快照收敛把流式尾巴并入 `entries`：同一 run id、同一摘要——游标协议
    // 与快照协议收敛到同一视图。
    el.entries = [
      ...base,
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(linkOf().querySelector('.running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(linkOf().getAttribute('aria-expanded')).toBe('false')
  })
})

// ---- fix-webui-streaming-liveness 4.1/4.2/4.3/4.5 -------------------------

/** jsdom 无布局：把滚动几何装进容器（configurable 以便用例间重定义）。 */
function fakeScrollLayout(box: HTMLElement, scrollHeight: number, clientHeight: number): void {
  Object.defineProperty(box, 'scrollHeight', { configurable: true, value: scrollHeight })
  Object.defineProperty(box, 'clientHeight', { configurable: true, value: clientHeight })
}

const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()))

describe('truncateHtml (4.5，纯函数)', () => {
  it('passes short content through untouched', () => {
    const r = truncateHtml({ content: 'short\nanswer' })
    expect(r.truncated).toBe(false)
    expect(r.preview).toBe('short\nanswer')
    expect(r.omittedLines).toBe(0)
    expect(r.omittedChars).toBe(0)
  })

  it('truncates by the line threshold and reports omitted lines/chars', () => {
    const lines = Array.from({ length: 60 }, (_, i) => `line-${i}`)
    const content = lines.join('\n')
    const r = truncateHtml({ content })
    expect(r.truncated).toBe(true)
    expect(r.preview).toBe(lines.slice(0, TRUNCATE_LINES).join('\n'))
    expect(r.omittedLines).toBe(60 - TRUNCATE_LINES)
    expect(r.omittedChars).toBe(content.length - r.preview.length)
  })

  it('truncates by the char threshold when it hits first', () => {
    const content = 'x'.repeat(TRUNCATE_CHARS + 500) // 单行：行阈值不触发
    const r = truncateHtml({ content })
    expect(r.truncated).toBe(true)
    expect(r.preview).toHaveLength(TRUNCATE_CHARS)
    expect(r.omittedChars).toBe(500)
    expect(r.omittedLines).toBe(0)
  })

  it('the earlier threshold wins when both exceed', () => {
    const content = Array.from({ length: 100 }, () => 'y'.repeat(200)).join('\n')
    const r = truncateHtml({ content })
    // 字符阈值（8_000）先到：行阈值切点是 40×200+39 = 8_039。
    expect(r.preview).toHaveLength(TRUNCATE_CHARS)
    expect(r.truncated).toBe(true)
  })
})

describe('scroll following (fix-webui-streaming-liveness 4.1)', () => {
  function scrollBox(el: SebasTranscriptView): HTMLElement {
    return el.shadowRoot!.querySelector<HTMLElement>('.scroll')!
  }

  it('sticky stays engaged at the bottom and streamed frames pin the view to the bottom', async () => {
    const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1) })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 1500
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(true)
    // 流式帧（turnUnits 变化）驱动滚动提交：瞬时贴底。
    emitTurnAppend('oc_test', [
      entry({ position: 90, kind: 'content', content: 'more', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    await nextFrame()
    await el.updateComplete
    expect(box.scrollTop).toBe(2000)
  })

  it('scrolling up disengages sticky; streamed frames do not yank the view', async () => {
    const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1) })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 100
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(false)
    emitTurnAppend('oc_test', [
      entry({ position: 90, kind: 'content', content: 'more', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    await nextFrame()
    await el.updateComplete
    expect(box.scrollTop).toBe(100)
  })

  it('returning near the bottom re-engages sticky (pure geometry, no seam math)', async () => {
    const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1) })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 100
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(false)
    box.scrollTop = 1800 // 距底 0 ≤ 阈值
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(true)
  })

  it('pure streamed appends commit the scroll even when entries/seam never change', async () => {
    // updated() 的 turnUnits 依赖：全部已读（seam 恒 null）、entries 属性
    // 不变——只有 turnUnits 变化的纯增量帧也必须提交滚动（旧实现漏滚）。
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T5 + 1000))
    const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1) })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 1500
    box.dispatchEvent(new Event('scroll'))
    emitTurnAppend('oc_test', [
      entry({ position: 90, kind: 'content', content: 'more', created_at_unix: FIXED_DATES.T1 }),
    ])
    await el.updateComplete
    await nextFrame()
    await el.updateComplete
    expect(box.scrollTop).toBe(2000)
  })
})

describe('unread boundary advances while focused+visible (polish-workbench-walkthrough-ux 3.1/3.2)', () => {
  const debounceWait = (): Promise<void> =>
    new Promise((r) => setTimeout(r, 300)) // 盖过 MARK_SEEN_DEBOUNCE_MS=250

  const scrollBox = (el: SebasTranscriptView): HTMLElement =>
    el.shadowRoot!.querySelector<HTMLElement>('.scroll')!

  /** 共享游标读回（2.3，D3）：段计数单字段；无键/旧格式（含 seen_ts）= null。 */
  function storedAnchor(key = 'oc_test'): number | null {
    const raw = store.get(`sebas:seen:${key}`)
    if (!raw) return null
    try {
      const v = JSON.parse(raw) as { anchor_count?: unknown }
      return typeof v.anchor_count === 'number' ? v.anchor_count : null
    } catch {
      return null
    }
  }

  it('聚焦 + 可见 + 贴底时到达的回合推进共享游标——重开不再出 seam（3.1）', async () => {
    // 初始：首回合已读（段锚 = 1）；贴底观看。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
    const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1), msgCount: 1 })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 1500
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(true)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    // 新回合在眼前到达（position 90/91 > 游标）。
    emitTurnAppend('oc_test', [
      entry({ position: 90, kind: 'prompt', content: 'again', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 91, kind: 'content', content: 'watched arrive', created_at_unix: FIXED_DATES.T3 }),
    ])
    await el.updateComplete
    await debounceWait()
    await el.updateComplete
    // 段锚推进到已渲染的两段；seam 保持隐藏（看着到达的内容不算未读）。
    expect(storedAnchor()).toBe(2)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('后台 tab（visibilityState=hidden）到达不推进锚——回来看 seam/徽章（3.2）', async () => {
    const orig = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState')
    try {
      Object.defineProperty(Document.prototype, 'visibilityState', {
        configurable: true,
        get: () => 'hidden',
      })
      store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
      const el = await mount({ entries: streamedTurn('q', ['a'], FIXED_DATES.T1), msgCount: 1 })
      const box = scrollBox(el)
      fakeScrollLayout(box, 2000, 500)
      box.scrollTop = 1500
      box.dispatchEvent(new Event('scroll'))
      // 隐藏期间到达。
      emitTurnAppend('oc_test', [
        entry({ position: 90, kind: 'prompt', content: 'again', created_at_unix: FIXED_DATES.T3 }),
        entry({ position: 91, kind: 'content', content: 'arrived hidden', created_at_unix: FIXED_DATES.T3 }),
      ])
      await el.updateComplete
      await debounceWait()
      await el.updateComplete
      // 锚不动：新达的一段保持 unseen。
      expect(storedAnchor()).toBe(1)
      expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(false)
      el.remove()
    } finally {
      if (orig) {
        Object.defineProperty(Document.prototype, 'visibilityState', orig)
      } else {
        delete (Document.prototype as { visibilityState?: unknown }).visibilityState
      }
    }
  })
  it('占位会话首交换经快照到达也推进锚——空流建立锚点，不画 seam（3.1）', async () => {
    // 新占位会话：0 回合、无历史锚（浏览器里没这个 key 的游标）。
    store.delete('sebas:seen:oc_test')
    const el = await mount({ entries: [], msgCount: 0 })
    const box = scrollBox(el)
    fakeScrollLayout(box, 2000, 500)
    box.scrollTop = 1500
    box.dispatchEvent(new Event('scroll'))
    expect(el.sticky).toBe(true)
    // 首条消息与回复经快照到达（秒回子进程：不走 turn.append）。
    el.entries = streamedTurn('hello', ['hello world'], FIXED_DATES.T1)
    el.msgCount = 2
    await el.updateComplete
    await debounceWait()
    await el.updateComplete
    // 锚从空流建立：写锚 = max(服务端段数 2, 本地已渲染 1) = 2，不再有无
    // 边框的「~N new since you last viewed」。
    expect(storedAnchor()).toBe(2)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('组件实例被重建（sessionKey 与首回合同帧到达）仍从空流建立锚（3.1）', async () => {
    // dashboard 在首条消息到达时重建 transcript-view 是常态：登记必须是
    // 模块级的，实例内的回合数差值不足以覆盖这个时序。
    store.delete('sebas:seen:oc_rebuild')
    const placeholder = await mount({ entries: [], sessionKey: 'oc_rebuild', msgCount: 0 })
    placeholder.remove()
    const el = await mount({
      entries: streamedTurn('hello', ['hi'], FIXED_DATES.T1),
      sessionKey: 'oc_rebuild',
      msgCount: 2,
    })
    await el.updateComplete
    await debounceWait()
    await el.updateComplete
    // 首交换仍算「看着到达」：锚已建立（段计数 2），seam 不出现。
    expect(storedAnchor('oc_rebuild')).toBe(2)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('打开既有未读会话的首帧快照不推进锚——seam 必须保留（3.1）', async () => {
    // 浏览器里的段锚停在首回合（1 段），会话在离场期间攒了第二个回合。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
    const el = await mount({
      entries: [
        ...streamedTurn('q', ['a'], FIXED_DATES.T1),
        ...streamedTurn('q2', ['b'], FIXED_DATES.T3).map((e) => ({
          ...e,
          position: e.position + 10,
        })),
      ],
      msgCount: 4,
    })
    await debounceWait()
    await el.updateComplete
    // 打开动作本身不是「看着到达」：锚留在 1 段，第二回合仍标未读。
    expect(storedAnchor()).toBe(1)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(false)
    el.remove()
  })

  it('已渲染会话的快照增长（重拉收敛）在贴底可见时推进锚——不依赖滚动事件（round3 6.1）', async () => {
    // QA 6.1 的时序：聚焦会话的新段经 entries 属性到达（秒回/重拉不走
    // turn.append），且短对话不溢出——滚动事件永不来，此前的锚推进全靠
    // 巧合。同会话（sessionKey 不变）的快照增长在 sticky + 可见下照
    // onTurnAppend 同一 guard 推进共享锚；seam 随之收敛。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 1 }))
    const el = await mount({
      entries: streamedTurn('q', ['a'], FIXED_DATES.T1),
      msgCount: 1,
    })
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    // 新回合经快照到达（属性更新，sessionKey 不变；无滚动事件）。
    el.entries = [
      ...streamedTurn('q', ['a'], FIXED_DATES.T1),
      ...streamedTurn('q2', ['watched arrive'], FIXED_DATES.T3).map((e) => ({
        ...e,
        position: e.position + 10,
      })),
    ]
    el.msgCount = 2
    await el.updateComplete
    await debounceWait()
    await el.updateComplete
    // 锚推进到已渲染段数（max(服务端 2, 本地 2)）；seam 不出现。
    expect(storedAnchor()).toBe(2)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('dashboard 侧空态登记（registerEmptyStreamSession）后首交换经挂载快照建立锚（round3 6.1）', async () => {
    // 真实应用里占位会话为空时主区渲染的是 dashboard 自己的空态占位而非
    // transcript 组件——组件内的空流登记分支永不执行（QA round5：新建即
    // 聚焦会话的首个交换冒徽标 + 缝且不消）。dashboard 渲染空态时经导出
    // 的登记入口补登记；首回合（连 prompt 带回复）经挂载快照到达仍算
    // 「亲眼看着到达」。
    store.delete('sebas:seen:oc_test')
    registerEmptyStreamSession('oc_test')
    const el = await mount({
      entries: streamedTurn('hello', ['hello world'], FIXED_DATES.T1),
      msgCount: 2,
    })
    await debounceWait()
    await el.updateComplete
    // 锚从空流建立：写锚 = max(服务端段数 2, 本地已渲染 1) = 2，seam 不出现。
    expect(storedAnchor()).toBe(2)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('prompt-only 中间态挂载不烧空流登记——回复经 turn.append 到达即时结算（fix-unread-fresh-exchange）', async () => {
    // GUI 真实时序（session-unread-badge「first focused exchange of a fresh
    // placeholder」）：创建写锚 0 → 空详情登记 → 首次快照只含操作者提交
    // （prompt-only 挂载：0 可见段、msg_count 仍 0，写锚 no-op）→ 回复经
    // turn.append 流式到达。旧实现在此挂载即消费登记：回复到达若组件被
    // 重建（hasUpdated=false 跳过快照推进），三条推进路径同时落空，徽章 +
    // 缝永久驻留。修复后无可推进水位不消费；回复到达**同步**结算（无
    // 250ms 防抖窗，seam 从未画出）。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    registerEmptyStreamSession('oc_test')
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'hello', created_at_unix: FIXED_DATES.T1 }),
      ],
      msgCount: 0,
    })
    expect(el.sticky).toBe(true)
    // 无可推进水位：锚原地不动，登记保留。
    expect(storedAnchor()).toBe(0)
    // 回复流式到达（transcript 自身的 turn.append 订阅；含 thinking 前导）。
    emitTurnAppend('oc_test', [
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 2, kind: 'content', content: 'hello world', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    // 同步结算（不等防抖）：锚推进到已渲染段数；seam 从未出现。
    expect(storedAnchor()).toBe(1)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })

  it('组件在 prompt-only 挂载与回复到达之间被重建——登记跨实例保留，新实例首帧快照仍结算（fix-unread-fresh-exchange）', async () => {
    // dashboard 的元素重建在真实浏览器里发生在首回合中途：实例 #1 只见到
    // prompt-only；回复含在实例 #2 的首帧快照里到达（hasUpdated=false 跳过
    // 快照推进——装载不是到达）。旧实现登记已被实例 #1 烧掉，锚永远停在 0；
    // 修复后登记跨实例存活，实例 #2 的挂载仍按「看着到达」结算。
    store.set('sebas:seen:oc_rebuild2', JSON.stringify({ anchor_count: 0 }))
    registerEmptyStreamSession('oc_rebuild2')
    const first = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'hello', created_at_unix: FIXED_DATES.T1 }),
      ],
      sessionKey: 'oc_rebuild2',
      msgCount: 0,
    })
    first.remove()
    const second = await mount({
      entries: streamedTurn('hello', ['hello world'], FIXED_DATES.T1),
      sessionKey: 'oc_rebuild2',
      msgCount: 1,
    })
    await second.updateComplete
    expect(storedAnchor('oc_rebuild2')).toBe(1)
    expect(second.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    second.remove()
  })

  it('空流登记会话的后台 tab 首交换：锚不推进、登记作废，回来看 seam（3.2）', async () => {
    // hidden-tab scenario 与空流登记的交叠：隐藏期到达不算「看着到达」——
    // 锚原地不动，登记即刻作废（翻回 visible 后不被静默结算），到达内容
    // 如实标未读（seam 在场），由操作员回读/mark all seen 推进。
    const orig = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState')
    try {
      Object.defineProperty(Document.prototype, 'visibilityState', {
        configurable: true,
        get: () => 'hidden',
      })
      store.set('sebas:seen:oc_hidden', JSON.stringify({ anchor_count: 0 }))
      registerEmptyStreamSession('oc_hidden')
      const el = await mount({
        entries: [
          entry({ position: 0, kind: 'prompt', content: 'hello', created_at_unix: FIXED_DATES.T1 }),
        ],
        sessionKey: 'oc_hidden',
        msgCount: 0,
      })
      emitTurnAppend('oc_hidden', [
        entry({ position: 1, kind: 'content', content: 'arrived hidden', created_at_unix: FIXED_DATES.T2 }),
      ])
      await el.updateComplete
      await debounceWait()
      await el.updateComplete
      expect(storedAnchor('oc_hidden')).toBe(0)
      expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(false)
      el.remove()
    } finally {
      if (orig) {
        Object.defineProperty(Document.prototype, 'visibilityState', orig)
      } else {
        delete (Document.prototype as { visibilityState?: unknown }).visibilityState
      }
    }
  })
})

describe('seam template identity (fix-webui-streaming-liveness 4.2)', () => {
  it('seam toggling preserves unit DOM identity and fold open state', async () => {
    // 初始全部已读（段锚 = mixedTurn 的 3 段），展开一个过程折叠。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 3 }))
    const el = await mount({ entries: mixedTurnEntries() })
    const link = el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!
    link.click()
    await el.updateComplete
    expect(link.getAttribute('aria-expanded')).toBe('true')
    const firstBlock = el.shadowRoot!.querySelector('.turn-block.is-assistant')!

    // 流式追加新回合（段数越过读锚）→ seam 出现。
    emitTurnAppend('oc_test', [
      entry({
        position: 99,
        kind: 'prompt',
        content: 'again',
        created_at_unix: FIXED_DATES.T5 + 100,
      }),
      entry({
        position: 100,
        kind: 'content',
        content: 'new turn',
        created_at_unix: FIXED_DATES.T5 + 100,
      }),
    ])
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(false)
    // 关键断言：同一 DOM 节点仍在（showSeam 翻转不触发整块重建），折叠
    // 展开态保留。
    expect(el.shadowRoot!.querySelector('.turn-block.is-assistant')).toBe(firstBlock)
    const linkAfter = el.shadowRoot!.querySelector<HTMLButtonElement>(
      '.process-fold button.fold-link',
    )!
    expect(linkAfter).toBe(link)
    expect(linkAfter.getAttribute('aria-expanded')).toBe('true')

    // mark all seen → seam 隐藏；节点身份依旧。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.seam button.link')!.click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    expect(el.shadowRoot!.querySelector('.turn-block.is-assistant')).toBe(firstBlock)
  })
})

describe('markdown incremental rendering (fix-webui-streaming-liveness 4.3)', () => {
  const mockRender = () => vi.mocked(renderMarkdown)

  it('streaming frames render the live tail as plain text — markdown calls stay flat', async () => {
    const el = await mount({
      entries: streamedTurn('q', ['chunk one '], FIXED_DATES.T1),
      turnLive: true,
    })
    await el.updateComplete
    mockRender().mockClear()
    // N 帧纯文本流：live tail 走纯文本，历史条目内容不变——renderMarkdown
    // 调用次数不随帧线性增长（此前每帧对全部文本 run 重解析）。
    for (let i = 2; i <= 6; i++) {
      emitTurnAppend('oc_test', [
        entry({ position: i, kind: 'content', content: `chunk ${i} `, created_at_unix: FIXED_DATES.T1 }),
      ])
      await el.updateComplete
    }
    expect(mockRender().mock.calls.length).toBe(0)

    // 定稿（turnLive 翻 false，随状态刷新到达）：tail 一次性换 markdown。
    el.turnLive = false
    await el.updateComplete
    expect(mockRender().mock.calls.length).toBeGreaterThan(0)
  })

  it('with turnLive false (history posture) markdown stays the default', async () => {
    const el = await mount({ entries: streamedTurn('q', ['settled'], FIXED_DATES.T1) })
    const liveBodies = el.shadowRoot!.querySelectorAll('.body.text-live')
    expect(liveBodies.length).toBe(0)
    expect(el.shadowRoot!.querySelector('.turn-block.is-assistant .body')?.innerHTML).toContain(
      '<p>settled</p>',
    )
  })
})

describe('truncation + view-all dialog (fix-webui-streaming-liveness 4.5)', () => {
  const longEntries = (): ConversationEntryView[] => [
    entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
    entry({
      position: 1,
      kind: 'content',
      element_type: 'tool',
      title: 'bash · big.sh',
      content: Array.from({ length: 80 }, (_, i) => `row ${i}`).join('\n'),
      created_at_unix: FIXED_DATES.T1,
    }),
  ]

  it('an expanded over-threshold entry shows the preview, the omission notice and view-all', async () => {
    const el = await mount({ entries: longEntries() })
    // 展开外层折叠 + 该二级条目（二级 body 只在其自身展开时渲染）。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-item button.fold-link')!.click()
    await el.updateComplete
    const body = el.shadowRoot!.querySelector('[data-truncated]')
    expect(body).toBeTruthy()
    expect(body!.textContent).toContain('row 0')
    expect(body!.textContent).not.toContain('row 79') // 预览不含被截断部分
    const note = el.shadowRoot!.querySelector('[data-testid="truncation-note"]')
    expect(note?.textContent).toContain('已截断')
    expect(note?.textContent).toContain('查看全部')
  })

  it('an expanded under-threshold entry renders in full with no truncation note', async () => {
    const el = await mount({ entries: mixedTurnEntries() })
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-item button.fold-link')!.click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-truncated]')).toBeNull()
    expect(el.shadowRoot!.querySelector('[data-testid="truncation-note"]')).toBeNull()
    expect(el.shadowRoot!.querySelector('button.view-all')).toBeNull()
  })

  it('view-all opens an isolated dialog; closing unmounts it outside the scroll surface', async () => {
    const el = await mount({ entries: longEntries() })
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-item button.fold-link')!.click()
    await el.updateComplete

    el.shadowRoot!.querySelector<HTMLButtonElement>('button.view-all')!.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('[data-testid="view-all-dialog"]')
    expect(dialog).toBeTruthy()
    expect(dialog!.textContent).toContain('row 79') // 完整内容只在弹层
    // 隔离：完整内容不在会话滚动容器内。
    expect(el.shadowRoot!.querySelector('.scroll')!.textContent).not.toContain('row 79')

    // 关闭（wa-hide：Esc/背板/关闭钮的统一出口）→ 弹层整棵卸载。
    dialog!.dispatchEvent(new Event('wa-hide'))
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="view-all-dialog"]')).toBeNull()
    // 对话面本身不受影响：预览仍在、无残留弹层节点。
    expect(el.shadowRoot!.querySelector('[data-truncated]')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('.scroll')!.textContent).toContain('row 0')
  })
})

// ── fix-webui-qa-defects 5.2：错误气泡标签按失败分类渲染 ──────────────────

describe('errorEntryLabel (fix-webui-qa-defects 5.2, design D5)', () => {
  it('labels a spawn failure as spawn failed', () => {
    expect(errorEntryLabel({ failure_class: 'spawn' })).toBe('spawn failed')
  })

  it('labels a stall force-settle as 回合停滞, never as a spawn failure', () => {
    const label = errorEntryLabel({ failure_class: 'stall' })
    expect(label).toContain('停滞')
    expect(label).not.toContain('spawn')
  })

  it('labels generic agent-turn errors (refusal included) neutrally', () => {
    expect(errorEntryLabel({ failure_class: 'generic' })).toBe('错误')
    expect(errorEntryLabel({ failure_class: 'generic', content: 'I cannot help with that.' })).toBe(
      '错误',
    )
  })

  it('falls back to the neutral label for legacy entries without a class', () => {
    expect(errorEntryLabel({})).toBe('错误')
    expect(errorEntryLabel({ failure_class: null })).toBe('错误')
    expect(errorEntryLabel({ failure_class: undefined })).toBe('错误')
  })
})

// ── fix-webui-qa-defects 5.3：被拒工具条目的展开详情与折叠标题一致（✗）────

describe('deniedDetailContent (fix-webui-qa-defects 5.3)', () => {
  const item = (content: string, title?: string | null): ProcessItem => ({
    elementType: 'tool',
    content,
    title,
    position: 0,
  })

  it('rewrites the leading approved prefix of a denied tool result to ✗', () => {
    const content = '✓ **Bash**\n已拒绝 by policy'
    const out = deniedDetailContent(content, item(content, '✓ Bash').title)
    expect(out.startsWith('✗')).toBe(true)
    expect(out.startsWith('✓')).toBe(false)
  })

  it('leaves a non-denied tool result untouched', () => {
    const content = '✓ **Read**\nfile contents'
    expect(deniedDetailContent(content, null)).toBe(content)
  })

  it('flags denial from the structured title even when the result text is plain', () => {
    const content = '✓ **Bash**\nexit 1'
    const out = deniedDetailContent(content, '✓ Bash · denied')
    expect(out.startsWith('✗')).toBe(true)
  })

  it('keeps content without a ✓ prefix intact (nothing to rewrite)', () => {
    const content = '已拒绝：普通文本结果（无 ✓ 前缀）'
    expect(deniedDetailContent(content, null)).toBe(content)
  })
})

// ── close-acceptance-blind-spots 4.2：零输出回合的 notice 中性信息条 ──────

describe('notice entries (close-acceptance-blind-spots 4.2, design D3)', () => {
  const noticeEntry = (position: number, ts: number = FIXED_DATES.T2): ConversationEntryView =>
    entry({
      position,
      kind: 'content',
      element_type: 'notice',
      content: '**回合已结束且无输出**：本轮回合未产生任何可见输出。',
      created_at_unix: ts,
    })

  it('notice entries stay standalone units that split the surrounding agent turn', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'partial', created_at_unix: FIXED_DATES.T1 }),
      noticeEntry(2),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'notice'])
    expect(unitMaxTs(units[2]!)).toBe(FIXED_DATES.T2)
  })

  it('notice contributes zero visible segments (zero-output turn must not inflate unread)', () => {
    const entries = [entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }), noticeEntry(1)]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => unitSegmentCount(u))).toEqual([0, 0])
  })

  it('renders as a neutral info bar, never the error bubble (failure semantics)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'say nothing', created_at_unix: FIXED_DATES.T1 }),
        noticeEntry(1),
      ],
    })
    const block = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-notice')!
    expect(block).toBeTruthy()
    // 中性信息条的可寻址 hook 与结构：i 头像 + 「提示」标签 + 正文。
    expect(block.getAttribute('data-testid')).toBe('notice-entry')
    expect(block.querySelector('.avatar.notice')?.textContent).toBe('i')
    expect(block.querySelector('.author.notice')?.textContent).toBe('提示')
    expect(block.querySelector('.body')?.textContent).toContain('回合已结束且无输出')
    // 无错误语义：不走 error 红泡类，也不渲染计数徽标。
    expect(block.querySelector('.avatar.error')).toBeNull()
    expect(block.querySelector('.bubble.error')).toBeNull()
    expect(block.querySelector('.count')).toBeNull()
    // 同屏的错误条目仍走既有红泡分支——notice 分支是新增而不是替换。
    const both = await mount({
      entries: [
        noticeEntry(0),
        entry({ position: 1, kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T2 }),
      ],
    })
    expect(both.shadowRoot!.querySelector('.turn-block.is-notice')).toBeTruthy()
    expect(both.shadowRoot!.querySelector('.turn-block.is-error .bubble.error')).toBeTruthy()
  })

  it('notice styling derives from theme tokens only (dark/light flip from one source)', async () => {
    const el = await mount({ entries: [noticeEntry(0)] })
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // 信息条的面/边/字全部走语义 token——明暗两态由 tokens.css 同源翻转
    // （dark: 深灰蓝面 / light: 浅灰面），组件内不做任何按主题的硬编码色。
    // var() 内允许 token 缺省兜底值（降级用），主色必须是 token。
    const rules = new Map<string, string>()
    for (const m of styleText.matchAll(/\.turn-block [^{]*\.notice[^{]*\{([^}]*)\}/g)) {
      rules.set(m[0].split('{')[0]!.trim(), m[1]!)
    }
    const avatar = rules.get('.turn-block .avatar.notice')
    const bubble = rules.get('.turn-block .bubble.notice')
    expect(avatar, 'avatar.notice rule').toBeTruthy()
    expect(avatar).toContain('background: var(--sebas-surface-2')
    expect(avatar).toContain('color: var(--sebas-text-dim')
    expect(bubble, 'bubble.notice rule').toBeTruthy()
    expect(bubble).toContain('background: var(--sebas-surface-2')
    expect(bubble).toContain('border-color: var(--sebas-border')
    // 中性 = 不借用任何失败/警示语义色。
    expect(styleText.match(/\.notice[^{]*\{[^}]*status-failed/g)).toBeNull()
  })
})
