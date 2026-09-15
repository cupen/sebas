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
  awaitingReceipt,
  groupConversation,
  mergeSpawnErrors,
  middleTruncate,
  processRunSummary,
  resolveAgentDisplay,
  splitAgentRuns,
} from './transcript-view.js'
import type { ProcessItem, ProcessRun } from './transcript-view.js'
import type { SebasTranscriptView } from './transcript-view.js'

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
}): Promise<SebasTranscriptView> {
  const el = document.createElement('sebas-transcript-view') as SebasTranscriptView
  el.entries = opts.entries
  el.sessionKey = opts.sessionKey ?? 'oc_test'
  if (opts.msgCount !== undefined) el.msgCount = opts.msgCount
  if (opts.agentDisplay !== undefined) el.agentDisplay = opts.agentDisplay
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
    // 「你」头像与 you 作者标注（既有 is-user 样式族）。
    expect(blocks?.[0]?.querySelector('.avatar.user')?.textContent).toBe('你')
    expect(blocks?.[0]?.querySelector('.author.you')?.textContent).toBe('you')
  })

  it('mixed turn renders one fold PER process run, each at its arrival position (2.1)', async () => {
    const el = await mount({ entries: mixedTurnEntries() })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 时间序切分：三个过程 run = 三个折叠，各在自己的发生位置。
    const folds = assistant.querySelectorAll<HTMLDetailsElement>('details.process-fold')
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
    expect(assistant.querySelector('details')).toBeNull()
    const bodies = assistant.querySelectorAll<HTMLElement>('.flow > .body')
    expect(bodies.length).toBe(1)
    expect(bodies[0].textContent).toContain('chunk one')
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
    const fold = assistant.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.getAttribute('data-process-count')).toBe('3')
    // 折叠行：固定 process 标签 + 实时摘要（进行中条目 = run 尾部，此处
    // 是无 title 的 tool → 通用标签）+ 条目计数。
    expect(fold.querySelector('summary .label')?.textContent?.trim()).toBe('process')
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('tool')
    // run 尾条目无 title → 通用标签、summary 不带 title 属性。
    expect(fold.querySelector('summary')!.hasAttribute('title')).toBe(false)
    expect(fold.querySelector('summary .fold-count')?.textContent?.trim()).toBe('3')
    // 外层默认收起；二级也全部默认收起。
    expect(fold.open).toBe(false)
    const items = assistant.querySelectorAll<HTMLDetailsElement>('details.process-item')
    expect(items.length).toBe(3)
    items.forEach((d) => expect(d.open).toBe(false))
    // 二级 summary：有 title 显示 title 且 title 属性保全量；无 title 回退
    // 通用标签（thinking / tool）且不带 title 属性。
    expect(items[0].dataset.position).toBe('1')
    expect(items[0].querySelector('summary .item-title')?.textContent?.trim()).toBe('thinking')
    expect(items[0].querySelector('summary')!.hasAttribute('title')).toBe(false)
    expect(items[1].querySelector('summary .item-title')?.textContent?.trim()).toBe(
      'read_file · src/app.ts',
    )
    expect(items[1].querySelector('summary')!.getAttribute('title')).toBe('read_file · src/app.ts')
    expect(items[2].querySelector('summary .item-title')?.textContent?.trim()).toBe('tool')
    // 展开层级：外层打开后二级仍收起，逐条再点才展开（键盘同路径）。
    fold.querySelector('summary')!.click()
    await el.updateComplete
    expect(fold.open).toBe(true)
    items.forEach((d) => expect(d.open).toBe(false))
    items[1].querySelector('summary')!.click()
    await el.updateComplete
    expect(items[1].open).toBe(true)
    expect(items[1].textContent).toContain('read_file')
    // 其余二级折叠不受影响。
    expect(items[0].open).toBe(false)
    expect(items[2].open).toBe(false)
  })

  it('the fold summary tracks the running tool and entry count as entries stream in (D3, 2.1)', async () => {
    const base = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries: base })
    let fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.getAttribute('data-process-count')).toBe('1')
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('thinking')
    expect(fold.querySelector('summary .fold-count')?.textContent?.trim()).toBe('1')
    // 流式增量到达（快照收敛路径重分组）：同一 run 的摘要实时刷新——
    // 进行中的工具 title + 累计条目数。
    el.entries = [
      ...base,
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(fold.querySelector('summary')!.getAttribute('title')).toBe('bash · deploy.sh')
    expect(fold.querySelector('summary .fold-count')?.textContent?.trim()).toBe('2')
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
    expect(el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!.open).toBe(
      false,
    )
    el.entries = [
      ...base,
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **bash**', title: 'bash · verify.sh', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('1')
    expect(fold.open).toBe(false)
    expect(fold.getAttribute('data-process-count')).toBe('3')
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('bash · verify.sh')
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
    const fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    fold.querySelector('summary')!.click()
    await el.updateComplete
    expect(fold.open).toBe(true)
    // 流式全量重分组（快照收敛）：新过程条目并入同一 run——折叠保持展开、
    // 新条目就地追加。
    el.entries = [
      ...base,
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', title: 'read · src/lib.rs', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const fold2 = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold2.dataset.processId).toBe('2')
    expect(fold2.open).toBe(true)
    expect(fold2.getAttribute('data-process-count')).toBe('2')
    const items = fold2.querySelectorAll<HTMLDetailsElement>('details.process-item')
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
    const first = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    first.querySelector('summary')!.click()
    await el.updateComplete
    expect(first.open).toBe(true)
    // 流式追加出第二个过程 run：新折叠默认收起，已展开的不受影响。
    el.entries = [
      ...base,
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 4, kind: 'content', content: 'after.', created_at_unix: FIXED_DATES.T2 }),
    ]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const folds = el.shadowRoot!.querySelectorAll<HTMLDetailsElement>('details.process-fold')
    expect(folds.length).toBe(2)
    expect(folds[0].dataset.processId).toBe('1')
    expect(folds[0].open).toBe(true)
    expect(folds[1].dataset.processId).toBe('3')
    expect(folds[1].open).toBe(false)
  })

  it('second-level folds keep position-keyed DOM identity (2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', element_type: 'thinking', content: 'a', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 9, kind: 'content', element_type: 'tool', content: 'b', title: 'read · x', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 17, kind: 'content', element_type: 'tool', content: 'c', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries })
    const items = el.shadowRoot!.querySelectorAll<HTMLDetailsElement>('details.process-item')
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
    const summary = el.shadowRoot!.querySelector<HTMLDetailsElement>(
      'details.process-item summary',
    )!
    const shown = summary.querySelector<HTMLElement>('.item-title')?.textContent?.trim()
    expect(shown).toBe(middleTruncate(long))
    expect(shown).toContain('…')
    expect(summary.getAttribute('title')).toBe(long)
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
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1))
    const el = await mount({ entries })
    // 边界下方是两个回合（new 提交 + 七条 chunk 的 agent 回合），但
    // agent 回合无论多少条 chunk 只计 1。
    expect((el as unknown as { unseenCount: number }).unseenCount).toBe(2)
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    expect(seam?.textContent).toContain('~2 new')
    // 边界落在未读第一个回合（operator 块）上方，不切开任何回合。
    const seamNext = seam?.nextElementSibling
    expect(seamNext?.classList.contains('is-user')).toBe(true)
    const agentBlocks = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(agentBlocks?.length).toBe(2)
    expect(agentBlocks?.[1]?.textContent).toContain('a')
    expect(agentBlocks?.[1]?.textContent).toContain('g')
  })

  it('no seam when everything is seen; mark-all-seen writes and hides', async () => {
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T2))
    const el = await mount({ entries })
    let seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(true)
    expect(seam?.textContent ?? '').not.toContain('new since you last viewed')

    // 未读态 → mark all seen → 游标写入最大时间戳、seam 消失。
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1 - 10))
    el.entries = [...entries]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el.updateComplete
    // rail-declutter-unread 2.2：存储迁到共享游标模块（JSON 形状），时间戳
    // 语义不变；无 msgCount payload 时段数锚保持 null（按已读）。
    const stored = JSON.parse(store.get('sebas:seen:oc_test')!) as {
      seen_ts: number
      anchor_count: number | null
    }
    expect(stored.seen_ts).toBe(FIXED_DATES.T1)
    expect(stored.anchor_count).toBeNull()
    expect(el.shadowRoot?.querySelector<HTMLElement>('.seam')?.hasAttribute('hidden')).toBe(true)
  })

  it('mark-all-seen advances the shared badge anchor when msgCount rides the payload (D3)', async () => {
    // rail-declutter-unread：payload 带 msg_count 时，标记已读把共享游标的
    // 段数锚推进到当前值——读到底部 = seam 清零 + 徽标清零（两者同锚）。
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    const el = await mount({ entries, msgCount: 3 })
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1 - 10))
    el.entries = [...entries]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el.updateComplete
    const stored = JSON.parse(store.get('sebas:seen:oc_test')!) as {
      seen_ts: number
      anchor_count: number | null
    }
    expect(stored.seen_ts).toBe(FIXED_DATES.T1)
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
    expect(el.shadowRoot!.querySelector('details.process-fold')).toBeNull()

    // 帧 1：thinking 新开过程 run —— 折叠默认收起、摘要 = 通用标签、计数 1。
    emitTurnAppend('oc_test', [
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    ])
    await el.updateComplete
    let fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    expect(fold.open).toBe(false)
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('thinking')
    expect(fold.querySelector('summary .fold-count')?.textContent?.trim()).toBe('1')

    // 帧 2：工具条目并入同一 run —— 摘要实时切到进行中的结构化 title、
    // 计数 2、仍保持收起。
    emitTurnAppend('oc_test', [
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.open).toBe(false)
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(fold.querySelector('summary')!.getAttribute('title')).toBe('bash · deploy.sh')
    expect(fold.querySelector('summary .fold-count')?.textContent?.trim()).toBe('2')

    // 游标去重：重复 position 的帧不重复计数、二级折叠不重复渲染。
    emitTurnAppend('oc_test', [
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '⚙ **bash**', title: 'bash · deploy.sh', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(fold.querySelectorAll('details.process-item')).toHaveLength(2)

    // 迟到的旧 position 帧被丢弃；非聚焦会话的帧被忽略。
    emitTurnAppend('oc_test', [
      entry({ position: 1, kind: 'content', content: 'STALE', created_at_unix: FIXED_DATES.T1 }),
    ])
    emitTurnAppend('oc_other', [
      entry({ position: 9, kind: 'content', element_type: 'tool', content: 'elsewhere', created_at_unix: FIXED_DATES.T2 }),
    ])
    await el.updateComplete
    fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
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
    fold = el.shadowRoot!.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.dataset.processId).toBe('2')
    expect(fold.getAttribute('data-process-count')).toBe('2')
    expect(fold.querySelector('summary .running')?.textContent?.trim()).toBe('bash · deploy.sh')
    expect(fold.open).toBe(false)
  })
})
