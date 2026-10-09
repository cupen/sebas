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
  entriesAwaitReceipt,
  errorEntryLabel,
  foldCrossRunToolResults,
  groupConversation,
  isDecidedToolResult,
  mergeSpawnErrors,
  mergeToolCalls,
  middleTruncate,
  processItemLabel,
  processRunDenied,
  processRunKind,
  processRunSummary,
  registerEmptyStreamSession,
  resolveAgentDisplay,
  splitAgentRuns,
  toolResultDenied,
  toolCallBlockDenied,
  toolCallBlockOutcome,
  truncateHtml,
  unitMaxTs,
  unitSegmentCount,
} from './transcript-view.js'
import type { ProcessItem, ProcessRun, ToolCallBlock } from './transcript-view.js'
import type { SebasTranscriptView } from './transcript-view.js'
import {
  armOpeningSeam,
  clearOpeningSeam,
  peekOpeningSeam,
} from './unread-cursor.js'

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
  currentModel?: string | null
  turnLive?: boolean
}): Promise<SebasTranscriptView> {
  const el = document.createElement('sebas-transcript-view') as SebasTranscriptView
  el.entries = opts.entries
  el.sessionKey = opts.sessionKey ?? 'oc_test'
  if (opts.msgCount !== undefined) el.msgCount = opts.msgCount
  if (opts.agentDisplay !== undefined) el.agentDisplay = opts.agentDisplay
  if (opts.currentModel !== undefined) el.currentModel = opts.currentModel
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
    const proc = splitAgentRuns([
      entry({ position: 1, element_type: 'tool', content: 'x', title: 't', tool_use_id: 'tc-1' }),
    ])
    if (proc[0].type !== 'process') return expect.unreachable()
    // fold-tool-calls-into-process-tree 4.1：条目携带上游 call id（配对键）。
    expect(proc[0].items[0]).toEqual({
      elementType: 'tool',
      content: 'x',
      title: 't',
      toolUseId: 'tc-1',
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

  // ── fix-webui-qa-round10 2.4（B-DEF-02）：回合观察模型时间线（纯函数）──

  it('turnObservedModels replays model_change timeline per agent turn (round10 2.4)', async () => {
    const { turnObservedModels } = await import('./transcript-view.js')
    const modelChange = (pos: number, from: string | null, to: string) =>
      entry({
        position: pos,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from, to }),
        created_at_unix: FIXED_DATES.T1,
      })
    const units = groupConversation([
      entry({ position: 0, kind: 'prompt', content: 'q1', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'a1', created_at_unix: FIXED_DATES.T1 }),
      modelChange(2, null, 'b'),
      entry({ position: 3, kind: 'prompt', content: 'q2', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 4, kind: 'content', content: 'a2', created_at_unix: FIXED_DATES.T2 }),
    ])
    const timeline = turnObservedModels(units)
    // 切换前的回合无观察值（空串 = 无徽章）；切换后的回合取新模型。
    expect(timeline.get(1)).toBe('')
    expect(timeline.get(4)).toBe('b')
    // model_change 自身不产生映射。
    expect(timeline.size).toBe(2)
  })

  it('turnObservedModels: a mid-turn switch splits the turn and the tail takes the new model', async () => {
    const { turnObservedModels } = await import('./transcript-view.js')
    const units = groupConversation([
      entry({ position: 0, kind: 'prompt', content: 'q', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'before ', created_at_unix: FIXED_DATES.T1 }),
      entry({
        position: 2,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from: 'a', to: 'b' }),
        created_at_unix: FIXED_DATES.T1,
      }),
      entry({ position: 3, kind: 'content', content: 'after', created_at_unix: FIXED_DATES.T2 }),
    ])
    // model_change 终结当前 agent 单元：切换后的条目自成新单元，取新模型
    // （「切换从下一回合生效」语义）。
    const timeline = turnObservedModels(units)
    expect(timeline.get(1)).toBe('')
    expect(timeline.get(3)).toBe('b')
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

// ─── fix-webui-qa-round3 1.1（D1）：thinking 折叠成员与可区分性 ──────────────

describe('thinking fold membership and distinguishability (fix-webui-qa-round3 D1)', () => {
  it('splitAgentRuns never routes text entries into process runs (alternating turn)', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, element_type: 'thinking', content: 'hmm' }),
      entry({ position: 2, element_type: 'markdown', content: 'part one. ' }),
      entry({ position: 3, element_type: 'thinking', content: 'hmm again' }),
      entry({ position: 4, element_type: 'markdown', content: 'part two.' }),
    ])
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process', 'text'])
    const texts = runs.filter((r): r is Extract<typeof r, { type: 'text' }> => r.type === 'text')
    expect(texts.map((t) => t.content)).toEqual(['part one. ', 'part two.'])
  })

  it('body segments never wear a process chip; chips live only on fold rows (DOM)', async () => {
    // fake-claude thinking 场景的 wire 形态：两段 thinking 与两段正文交替。
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 2, kind: 'content', element_type: 'markdown', content: 'thought out loud', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 3, kind: 'content', element_type: 'thinking', content: 'hmm again', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 4, kind: 'content', element_type: 'markdown', content: 'and the answer', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 正文段：零折叠链接、零「process」字样——不携带任何过程芯片/提示。
    const bodies = [...assistant.querySelectorAll<HTMLElement>('.flow > .body')]
    expect(bodies).toHaveLength(2)
    for (const body of bodies) {
      expect(body.querySelector('.fold-link, .process-fold, .kind-icon')).toBeNull()
      expect(body.textContent).not.toMatch(/process/i)
    }
    // 过程芯片只出现在过程折叠的收起行上（两条 thinking 段各一折）。
    const folds = [...assistant.querySelectorAll<HTMLElement>('.process-fold')]
    expect(folds).toHaveLength(2)
    for (const fold of folds) {
      expect(fold.querySelector('.fold-link .label')?.textContent?.trim()).toBe('过程')
    }
    el.remove()
  })

  it('a thinking-only fold is marked thinking (data-kind + glyph) in its collapsed default', async () => {
    // thinking 独占与 tool 独占各渲一折：glyph 必须不同（thinking 专用
    // 标识 vs 工具的 zap）。
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'deep thought', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 2, kind: 'prompt', content: 'again', created_at_unix: FIXED_DATES.T2 }),
        entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', title: 'read · x', created_at_unix: FIXED_DATES.T2 }),
      ],
    })
    const folds = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.process-fold')]
    expect(folds).toHaveLength(2)
    const thinkingFold = folds[0]!
    const toolFold = folds[1]!
    // spec「thinking fold is titled and distinguishable」：收起行即标明
    // thinking——data-kind + thinking 专用 glyph（非工具的 zap）。
    expect(thinkingFold.dataset.kind).toBe('thinking')
    expect(toolFold.dataset.kind).toBe('tool')
    expect(thinkingFold.querySelector('.fold-link .running')?.textContent?.trim()).toBe('thinking')
    const thinkingGlyph = thinkingFold.querySelector('.kind-icon')!.innerHTML
    const toolGlyph = toolFold.querySelector('.kind-icon')!.innerHTML
    expect(thinkingGlyph).not.toBe(toolGlyph)
    expect(thinkingGlyph).toContain('svg')
    el.remove()
  })

  it('a tool-only fold keeps the tool kind; mixed runs are marked mixed', async () => {
    const runs: ProcessRun[] = [
      { type: 'process', position: 1, items: [{ elementType: 'tool', content: 'x', title: null, position: 1 }] },
      { type: 'process', position: 2, items: [
        { elementType: 'thinking', content: 'h', title: null, position: 2 },
        { elementType: 'tool', content: 't', title: null, position: 3 },
      ] },
    ]
    expect(processRunKind(runs[0]!)).toBe('tool')
    expect(processRunKind(runs[1]!)).toBe('mixed')
  })

  it('second-level thinking folds carry the thinking glyph next to the title', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    assistant.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const item = assistant.querySelector<HTMLElement>('.process-item[data-element-type="thinking"]')!
    expect(item.querySelector('.item-kind-icon')).toBeTruthy()
    expect(item.querySelector('.item-title')?.textContent?.trim()).toBe('thinking')
    el.remove()
  })

  // （fix-webui-qa-round6 3.1，design D4）展开的过程折叠内 thinking 条目
  // **默认**显示实际内容——不需要对二级折叠的第二次点击；占位词只作标签。
  it('an expanded process fold shows the thinking content inline by default (round6 3.1)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'think please', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'hmm again', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 一次点击（过程折叠）后内容即在场——无需任何二级交互。
    assistant.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const foldBody = assistant.querySelector<HTMLElement>('.process-fold .fold-body')!
    expect(foldBody.textContent).toContain('hmm')
    expect(foldBody.textContent).toContain('hmm again')
    // 占位词仍是条目标签，但内容不再被占位词顶替。
    // （fix-webui-qa-round8 7.1）相邻思考增量聚合为一个连续段落：逐帧落账
    // 的两条 thinking 在展开体里合并为**一条**条目（内容顺序拼接），不再是
    // 一屏碎片行。
    const items = [...foldBody.querySelectorAll<HTMLElement>('.process-item[data-element-type="thinking"]')]
    expect(items).toHaveLength(1)
    expect(items[0]!.querySelector('.item-title')?.textContent?.trim()).toBe('thinking')
    expect(items[0]!.querySelector('.body.item-body')).toBeTruthy()
    expect(items[0]!.querySelector('.body.item-body')!.textContent).toContain('hmm')
    expect(items[0]!.querySelector('.body.item-body')!.textContent).toContain('hmm again')
    // thinking 条目不再渲染二级折叠开关（内容默认在场）。
    expect(foldBody.querySelector('.process-item[data-element-type="thinking"] button')).toBeNull()
    el.remove()
  })

  it('collapsed fold rows are visually distinct from prose (chip styling, no border/card)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // 与正文可区分：折叠行（.fold-link）带胶囊底色。
    const linkRule = styleText.match(/\.turn-block \.fold-link\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(linkRule).toMatch(/background:\s*var\(--sebas-surface-2\)/)
    expect(linkRule).toMatch(/border-radius:/)
    // 仍是轻量行内控件：无边框、无卡框（4.4 合同不被胶囊破坏）。
    expect(linkRule).toMatch(/border:\s*none/)
    const foldRule = styleText.match(/\.turn-block \.process-fold\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(foldRule).not.toMatch(/\bborder:/)
    expect(foldRule).not.toMatch(/\bbackground:/)
    el.remove()
  })

  // ── fix-webui-qa-round13 1.1（B-2）：展开面板留白收敛 ──
  // 空带来源 = .body 的 pre-wrap（round11 4.1）经类名与继承进入展开面板，
  // 把模板缩进换行与 markdown 块间换行逐个渲染成可见空行（QA b48/a54：
  // 💭 thinking 标签与内容之间大段空带，两轮采样稳定）。修法钉在展开态
  // 样式：fold-body 与内层条目体恢复 normal；jsdom 算不出布局，容器断言
  // 走组件样式表（round11 4.1 同姿态）。
  it('expanded fold body collapses whitespace-driven blank bands (round13 1.1, B-2)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'think please', created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 展开体在场：收敛的是空白排版，内容行零变化。
    assistant.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const foldBody = assistant.querySelector<HTMLElement>('.process-fold .fold-body')!
    expect(foldBody.textContent).toContain('hmm')
    const { SebasTranscriptView: Impl } = await import('./transcript-view.js')
    const styles = (Impl as typeof import('./transcript-view.js').SebasTranscriptView).styles
    const css = (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
    // 展开体与内层条目体恢复 normal（模板/markdown 换行不再成为可见空行）。
    const foldWsRule = css.match(/\.turn-block \.fold-body\s*\{[^}]*\}/)?.[0] ?? ''
    expect(foldWsRule).toContain('white-space: normal')
    const innerWsRule = css.match(/\.turn-block \.fold-body \.body\s*\{[^}]*\}/)?.[0] ?? ''
    expect(innerWsRule).toContain('white-space: normal')
    // 级联成立：fold-body 自身同挂 .body（同特异性 (0,2,0)），恢复规则必须
    // 晚于 pre-wrap 主规则；内层选择器特异性更高，不受顺序影响。
    const bodyRuleStart = css.indexOf(css.match(/\.turn-block \.body\s*\{[^}]*\}/)?.[0] ?? '')
    expect(bodyRuleStart).toBeGreaterThanOrEqual(0)
    expect(css.indexOf(foldWsRule)).toBeGreaterThan(bodyRuleStart)
    // 语义不回退：外层 .body 的 pre-wrap（多行提交，round11 4.1）原样保留
    // ——收敛只发生在展开面板分支内。
    expect(css.match(/\.turn-block \.body\s*\{[^}]*\}/)?.[0] ?? '').toContain('white-space: pre-wrap')
    el.remove()
  })
})

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

  // ── fix-webui-qa-round11 4.1（B-4）：多行提交保留换行 ──
  // spec「three-line message renders as three lines」：容器 pre-wrap（渲染
  // 不再把 \n 折叠成空格）+ 文本逐字保留换行（wire 往返语义），两者合成
  // 三行视觉呈现；用户气泡与 agent 回显同一容器同一规则（恢复后的转录走
  // 同一路径，reload 不丢行结构）。jsdom 算不出布局/继承样式，容器的
  // pre-wrap 断言走组件样式表（dashboard.test 同姿态）。
  it('a three-line message renders as three lines in user bubble and agent echo (round11 4.1)', async () => {
    const threeLines = '键盘行甲\n键盘行乙\n键盘行丙'
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: threeLines, created_at_unix: FIXED_DATES.T1 }),
        entry({ position: 1, kind: 'content', content: threeLines, created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    // 渲染半边：文本逐字保留换行（三行 = 两处 \n），气泡容器与回显同构。
    const bodies = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block .body')]
    expect(bodies.length).toBe(2)
    for (const body of bodies) {
      expect(body.textContent, '文本内容逐字保留 \\n').toBe(threeLines)
      expect(body.textContent?.split('\n')).toHaveLength(3)
    }
    // 容器半边：.turn-block .body 规则带 pre-wrap（渲染层不再折叠换行），
    // 且长 token 折行语义（overflow-wrap: anywhere）不回退。
    const { SebasTranscriptView: Impl } = await import('./transcript-view.js')
    const styles = (Impl as typeof import('./transcript-view.js').SebasTranscriptView).styles
    const css = (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
    const bodyRule = css.match(/\.turn-block \.body\s*\{[^}]*\}/)?.[0] ?? ''
    expect(bodyRule).toContain('white-space: pre-wrap')
    expect(bodyRule).toContain('overflow-wrap: anywhere')
    el.remove()
  })

  // ── fix-webui-qa-round10 2.4（B-DEF-02）：模型徽章按回合观察值保真 ──
  // round9 4.3 的「与会话头同源（current_model 下传）」语义被 B-DEF-02 证伪
  // （切模型后历史徽章被回溯改写）：徽章改读 model_change 留痕重放的回合
  // 观察值，会话当前模型不再参与（currentModel 属性退役为传参兼容）。

  it('a turn after a model_change shows the new model; earlier turns keep theirs (round10 2.4)', async () => {
    // 时间线：turn A（无观察 → 无徽章）→ model_change a→b → turn B（b）→
    // model_change b→c → turn C（c）。切到 c 后 A/B 的徽章原样不动。
    const entries: ConversationEntryView[] = [
      entry({ position: 0, kind: 'prompt', content: 'q1', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'answer a', created_at_unix: FIXED_DATES.T1 }),
      entry({
        position: 2,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from: 'a', to: 'b' }),
        created_at_unix: FIXED_DATES.T1,
      }),
      entry({ position: 3, kind: 'prompt', content: 'q2', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 4, kind: 'content', content: 'answer b', created_at_unix: FIXED_DATES.T3 }),
      entry({
        position: 5,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from: 'b', to: 'c' }),
        created_at_unix: FIXED_DATES.T3,
      }),
      entry({ position: 6, kind: 'prompt', content: 'q3', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 7, kind: 'content', content: 'answer c', created_at_unix: FIXED_DATES.T3 }),
    ]
    const el = await mount({ entries, currentModel: 'c' })
    const turns = el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(turns.length).toBe(3)
    const chipOf = (t: HTMLElement) =>
      t.querySelector<HTMLElement>('[data-testid="turn-model"]')?.textContent ?? null
    // 首个回合无观察值：无徽章（不伪造），且不回填会话当前模型 c。
    expect(chipOf(turns[0]!)).toBeNull()
    expect(chipOf(turns[1]!)).toBe('b')
    expect(chipOf(turns[2]!)).toBe('c')
    el.remove()
  })

  it('switching the session model later never rewrites earlier badges (round10 2.4)', async () => {
    // B-DEF-02 的直接复现面：徽章文本在 currentModel 翻转前后逐字节不变
    // （QA：bad-model 时期的回复在切到 ok-model 后被改写成 ok-model）。
    const modelChange: ConversationEntryView = {
      position: 3,
      kind: 'content',
      element_type: 'model_change',
      content: JSON.stringify({ from: 'bad-model', to: 'ok-model' }),
      created_at_unix: FIXED_DATES.T1,
    }
    const q = entry({ position: 4, kind: 'prompt', content: 'q', created_at_unix: FIXED_DATES.T3 })
    const a = entry({
      position: 5,
      kind: 'content',
      content: 'answer',
      created_at_unix: FIXED_DATES.T3,
    })
    const badgesAt = (el: SebasTranscriptView): (string | null)[] =>
      [...el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block.is-assistant')].map(
        (t) => t.querySelector<HTMLElement>('[data-testid="turn-model"]')?.textContent ?? null,
      )
    const before = await mount({
      entries: [q, a],
      currentModel: 'bad-model',
    })
    expect(badgesAt(before)).toEqual([null])
    before.remove()
    const after = await mount({
      entries: [q, a, modelChange],
      currentModel: 'ok-model',
    })
    // 同一批历史条目在切换落痕后重渲：仍无徽章（无观察值不回填），
    // 绝不显示 ok-model。
    expect(badgesAt(after)).toEqual([null])
    after.remove()
  })

  it('unknown model renders no model chip — never a fabricated placeholder (round9 4.3)', async () => {
    for (const model of [null, '', '   ']) {
      const el = await mount({
        entries: streamedTurn('do it', ['chunk one'], FIXED_DATES.T1),
        currentModel: model,
      })
      const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
      // （round10 2.4）无 model_change 留痕 = 无观察值：无论会话当前模型
      // 是什么都无徽章（观察不到就如实缺省，不伪造）。
      expect(assistant.querySelector('[data-testid="turn-model"]')).toBeNull()
      el.remove()
    }
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
    expect(link.querySelector('.label')?.textContent?.trim()).toBe('过程')
    expect(link.querySelector('.fold-count')?.textContent?.trim()).toBe('1')
    // 样式面：折叠容器无边框/无底色卡框（link 化的外观合同）。
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    const foldRule = styleText.match(/\.turn-block \.process-fold\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(foldRule).not.toMatch(/\bborder:/)
    expect(foldRule).not.toMatch(/\bbackground:/)
  })

  it('process folds and tool-call blocks collapse by default; titles show with generic fallback (2.1/2.2 + fold-tool-calls 4.3)', async () => {
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
    expect(link.querySelector('.label')?.textContent?.trim()).toBe('过程')
    expect(link.querySelector('.running')?.textContent?.trim()).toBe('tool')
    // run 尾条目无 title → 通用标签、link 不带 title 属性。
    expect(link.hasAttribute('title')).toBe(false)
    expect(link.querySelector('.fold-count')?.textContent?.trim()).toBe('3')
    // 外层默认收起（无展开体——子折叠只在展开体内渲染，收起态不残留
    // 隐藏 DOM）。
    expect(link.getAttribute('aria-expanded')).toBe('false')
    expect(fold.querySelector('.fold-body')).toBeNull()
    link.click()
    await el.updateComplete
    // （fold-tool-calls-into-process-tree 4.2/4.3）单棵过程树：thinking 是
    // 二级条目、工具条目是合并块，两者都默认收起。
    const thinkingItem = assistant.querySelector<HTMLElement>('.process-item')!
    expect(thinkingItem.dataset.position).toBe('1')
    expect(thinkingItem.querySelector('.item-title')?.textContent?.trim()).toBe('thinking')
    expect(thinkingItem.querySelector('.body.item-body')?.textContent).toContain('deep thought')
    const blocks = [...assistant.querySelectorAll<HTMLElement>('[data-testid="tool-result-entry"]')]
    expect(blocks.length).toBe(2)
    // 两个块都默认收起（foldOpen 未命中即收起）。
    blocks.forEach((d) =>
      expect(
        d.querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!.getAttribute('aria-expanded'),
      ).toBe('false'),
    )
    // 块标题：有 title 显示结构化标题且 title 属性保全量；无 title 回退
    // 通用标签（tool）且不带 title 属性。
    expect(blocks[0].dataset.position).toBe('2')
    expect(blocks[0].querySelector('.item-title')?.textContent?.trim()).toBe(
      'read_file · src/app.ts',
    )
    expect(blocks[0].querySelector('[data-testid="tool-result-link"]')!.getAttribute('title')).toBe(
      'read_file · src/app.ts',
    )
    expect(blocks[1].dataset.position).toBe('3')
    expect(blocks[1].querySelector('.item-title')?.textContent?.trim()).toBe('tool')
    // 展开层级：点击合并块自身的 link 才展开（键盘同路径——原生 button
    // 激活）；展开的是参数段（调用态，无章）。
    const blockLink = blocks[0].querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!
    blockLink.click()
    await el.updateComplete
    expect(blockLink.getAttribute('aria-expanded')).toBe('true')
    expect(blocks[0].textContent).toContain('read_file')
    // 其余块不受影响。
    expect(
      blocks[1]
        .querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!
        .getAttribute('aria-expanded'),
    ).toBe('false')
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
    // （fold-tool-calls-into-process-tree 4.2）新条目是工具调用 → 就地追加
    // 的是合并块（不再是裸二级条目）。
    expect(fold2.querySelectorAll('.process-item')).toHaveLength(1)
    const block = fold2.querySelector<HTMLElement>('[data-testid="tool-result-entry"]')!
    expect(block).toBeTruthy()
    expect(block.dataset.position).toBe('3')
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

  it('second-level nodes keep position-keyed DOM identity (2.2 + fold-tool-calls 4.1)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', element_type: 'thinking', content: 'a', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 9, kind: 'content', element_type: 'tool', content: 'b', title: 'read · x', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 17, kind: 'content', element_type: 'tool', content: 'c', created_at_unix: FIXED_DATES.T1 }),
    ]
    const el = await mount({ entries })
    // 子节点在展开体内：先展开外层折叠。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    // thinking 保持二级条目；工具条目是合并块——各自的 DOM 身份都是条目
    // position（data-position），节点序 = 到达序。
    const nodes = el.shadowRoot!.querySelectorAll<HTMLElement>(
      '.process-item, [data-testid="tool-result-entry"]',
    )
    expect([...nodes].map((d) => d.dataset.position)).toEqual(['4', '9', '17'])
    expect(nodes[0].classList.contains('process-item')).toBe(true)
    expect(nodes[1].getAttribute('data-testid')).toBe('tool-result-entry')
    expect(nodes[2].getAttribute('data-testid')).toBe('tool-result-entry')
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
    // （fold-tool-calls-into-process-tree 4.2）工具条目的标题住在合并块的
    // 收起行上。
    const link = el.shadowRoot!.querySelector<HTMLButtonElement>(
      '[data-testid="tool-result-link"]',
    )!
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
    expect(seam?.textContent).toContain('~1 条新消息')
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
    el.remove()

    // （fix-webui-qa-round2 2.1）未读态要看到 seam 需要一次**新的开卷**：
    // 分界线边界在开卷帧捕获（锚=0 < 段数 1）。mark all seen → 游标写入
    // 当前段数、seam 消失、开卷登记作废。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    const el2 = await mount({ entries })
    await el2.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el2.updateComplete
    seam = el2.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el2.updateComplete
    // （2.3，D3）单字段段锚：存储只有 anchor_count；无 msgCount payload 时
    // 写本地已渲染段数（相邻 md 合并 = 1）。
    const stored = JSON.parse(store.get('sebas:seen:oc_test')!) as {
      anchor_count: number
    }
    expect(Object.keys(stored)).toEqual(['anchor_count'])
    expect(stored.anchor_count).toBe(1)
    expect(el2.shadowRoot?.querySelector<HTMLElement>('.seam')?.hasAttribute('hidden')).toBe(true)
    el2.remove()
  })

  it('mark-all-seen advances the shared badge anchor when msgCount rides the payload (D3)', async () => {
    // rail-declutter-unread：payload 带 msg_count 时，标记已读把共享游标的
    // 段数锚推进到 max(服务端段数, 本地已渲染段数)——读到底部 = seam 清零 +
    // 徽标清零（两者同锚）。
    const entries = streamedTurn('do it', ['a', 'b'], FIXED_DATES.T1)
    // （fix-webui-qa-round2 2.1）锚=0 在**开卷前**就位：边界在开卷帧捕获，
    // seam 对开卷时未读的内容呈现。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    const el = await mount({ entries, msgCount: 3 })
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
    // （fold-tool-calls-into-process-tree 4.2）重复 position 帧不重复渲染：
    // thinking 二级条目 1 + 工具合并块 1。
    expect(fold.querySelectorAll('.process-item, [data-testid="tool-result-entry"]')).toHaveLength(2)
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

  it('打开既有未读会话不推进锚——锚只反映已见内容，seam 与徽标水位保留（round8 5.1）', async () => {
    // 浏览器里的段锚停在首回合（1 段），会话在离场期间攒了第二个回合。
    // （fix-webui-qa-round8 5.1）开卷不再无条件把锚推到服务端当前计数——
    // 打开不等于看完：溢出可视高度的未读内容保持未读水位（jsdom 无布局
    // 几何，fit-viewport 结算不触发），锚停在 1、分界线照画；滚动贴底 /
    // mark all seen / 跳到最新才推进。
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
    // （fix-webui-qa-round2 2.1）操作员已上滚（不贴底）：到达不再是「看着
    // 到达」，开卷边界不因此清账——越过边界的新回合之上有 seam。
    el.sticky = false
    await el.updateComplete

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
    // 展开外层折叠 + 该合并块（块 body 只在其自身展开时渲染）。
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!.click()
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
    // 外层折叠全部展开：thinking 条目（round6 3.1）内容默认在场；工具条目
    // 是合并块，再点块自身的 link。
    for (const fold of [
      ...el.shadowRoot!.querySelectorAll<HTMLButtonElement>('.process-fold button.fold-link'),
    ]) {
      fold.click()
      await el.updateComplete
    }
    el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!.click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-truncated]')).toBeNull()
    expect(el.shadowRoot!.querySelector('[data-testid="truncation-note"]')).toBeNull()
    expect(el.shadowRoot!.querySelector('button.view-all')).toBeNull()
  })

  it('view-all opens an isolated dialog; closing unmounts it outside the scroll surface', async () => {
    const el = await mount({ entries: longEntries() })
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!.click()
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
    expect(errorEntryLabel({ failure_class: 'spawn' })).toBe('启动失败')
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

// ── fold-tool-calls-into-process-tree 4.1/4.2/4.3：合并块与单棵过程树 ────────
// （原 D-C3a「已决结果顶层化」契约退役：调用与结果按 tool_use_id 配对合并
// 为过程折叠内的一个收起块，顶层 tool_result 块不复存在。）

describe('tool call blocks merge invocation and result (fold-tool-calls 4.1)', () => {
  const requestEntry = (pos: number, id?: string): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'tool',
    content: '📖 **Bash**\n```json\n{"command": "rm -rf /"}\n```',
    title: 'Bash · rm -rf /',
    tool_use_id: id ?? null,
    created_at_unix: FIXED_DATES.T1,
  })
  const resultEntry = (
    pos: number,
    id?: string,
    content = '✓ **Bash**\nperm done\n',
  ): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'tool',
    content,
    title: '✓ Bash',
    tool_use_id: id ?? null,
    created_at_unix: FIXED_DATES.T1,
  })

  it('isDecidedToolResult: ✓/✗ titled tool entries are results; requests and thinking are not', () => {
    expect(isDecidedToolResult(resultEntry(0))).toBe(true)
    expect(isDecidedToolResult(resultEntry(0, undefined, '✗ **Bash**\ndenied by fake'))).toBe(true)
    expect(isDecidedToolResult(requestEntry(0))).toBe(false)
    expect(
      isDecidedToolResult({
        element_type: 'thinking',
        content: 'the request was denied earlier',
        title: null,
      }),
      // thinking entries are never results even when the text mentions denial.
    ).toBe(false)
  })

  it('splitAgentRuns: decided results stay inside the process run (no top-level tool_result runs)', () => {
    const runs = splitAgentRuns([requestEntry(0, 'tc-1'), resultEntry(1, 'tc-1')])
    expect(runs.map((r) => r.type)).toEqual(['process'])
    const fold = runs[0] as ProcessRun
    // 过程 run 收全部两条原始条目（合并是渲染层的纯派生）。
    expect(fold.items).toHaveLength(2)
  })

  it('mergeToolCalls: a paired invocation+result becomes ONE block', () => {
    const items = splitAgentRuns([requestEntry(0, 'tc-1'), resultEntry(1, 'tc-1')])
    if (items[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(items[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.invocation.title).toBe('Bash · rm -rf /')
    expect(block.result!.title).toBe('✓ Bash')
    expect(block.position).toBe(0)
    // 配对成功 → 结果章 ✓（collapse 行即读）。
    expect(toolCallBlockOutcome(block)).toBe('ok')
    expect(toolCallBlockDenied(block)).toBe(false)
  })

  it('mergeToolCalls: an unpaired invocation renders as its own call-state block (no outcome)', () => {
    const items = splitAgentRuns([requestEntry(0, 'tc-1')])
    if (items[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(items[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.result).toBeNull()
    // 调用态标题：无 ✓/✗ 章。
    expect(toolCallBlockOutcome(block)).toBeNull()
  })

  it('mergeToolCalls: concurrent calls to the same tool pair by id, never by position', () => {
    const entries = [
      requestEntry(0, 'tc-a'),
      { ...requestEntry(1, 'tc-b'), title: 'Read · /tmp/b', content: '📖 **Read**\n```json\n{"file_path": "/tmp/b"}\n```' },
      { ...resultEntry(2, 'tc-b'), content: '✓ **Read**\ncontent of b' },
      { ...resultEntry(3, 'tc-a'), content: '✓ **Read**\ncontent of a' },
    ]
    const runs = splitAgentRuns(entries)
    if (runs[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(runs[0].items)
    expect(nodes).toHaveLength(2)
    const [blockA, blockB] = nodes.map((n) => n as ToolCallBlock)
    // 结果乱序回流：各自按 id 回到自己的调用块——b 的结果先到也不串块。
    expect(blockA.invocation.title).toBe('Bash · rm -rf /')
    expect(blockA.result!.content).toContain('content of a')
    expect(blockB.invocation.title).toBe('Read · /tmp/b')
    expect(blockB.result!.content).toContain('content of b')
  })

  it('mergeToolCalls: a lone result (truncated transcript) still renders as its own block', () => {
    const runs = splitAgentRuns([resultEntry(0, 'tc-x')])
    if (runs[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(runs[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.result).toBeNull()
    // 孤立 ✓ 条目自带已决标记 → 保留 ✓ 章（内容零丢失）。
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })

  // ── fold-tool-calls-into-process-tree review F1：跨 run 配对 ──────────────
  // native 载体对泊车/决策落 markdown 条目（⏳/🛡），把同一调用的结果切进
  // 另一个过程 run；foldCrossRunToolResults 在回合范围内按 id 把结果搬回
  // 调用所在 run（spec「配对 SHALL NOT rely on arrival position」）。

  const textEntry = (pos: number, content: string): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'markdown',
    content,
    created_at_unix: FIXED_DATES.T1,
  })

  it('foldCrossRunToolResults: a result split off by parking notes moves back to its invocation run', () => {
    // native 被门控序列：tool(调用) → markdown(⏳/🛡) → tool(✓ 结果)。
    const runs = splitAgentRuns([
      requestEntry(0, 'toolu_1'),
      textEntry(1, '⏳ bash awaits approval'),
      textEntry(2, '🛡 policy allow'),
      resultEntry(3, 'toolu_1'),
    ])
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    const regrouped = foldCrossRunToolResults(runs)
    // 结果搬回调用所在 run；被搬空的过程 run 丢弃；正文 run 原样保留。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text'])
    const fold = regrouped[0] as ProcessRun
    expect(fold.items).toHaveLength(2)
    expect((regrouped[1] as { content: string }).content).toContain('awaits approval')
    // 逐 run 合并后：一块、带结果、✓ 章（调用块与汇总行都能读到达成）。
    const nodes = mergeToolCalls(fold.items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.result).not.toBeNull()
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })

  it('foldCrossRunToolResults: parallel results scattered across runs each return to their own call', () => {
    const runs = splitAgentRuns([
      requestEntry(0, 'tc-a'),
      { ...requestEntry(1, 'tc-b'), title: 'Read · /tmp/b', content: '📖 **Read**' },
      textEntry(2, '⏳ a awaits approval'),
      textEntry(3, '⏳ b awaits approval'),
      { ...resultEntry(4, 'tc-b'), content: '✓ **Read**\nb done' },
      { ...resultEntry(5, 'tc-a'), content: '✓ **Bash**\na done' },
    ])
    const regrouped = foldCrossRunToolResults(runs)
    // 两个结果都回到第一个 run；中间泊车正文不动；收尾 run 搬空即弃。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text'])
    const nodes = mergeToolCalls((regrouped[0] as ProcessRun).items)
    expect(nodes).toHaveLength(2)
    const [a, b] = nodes.map((n) => n as ToolCallBlock)
    expect(a.result!.content).toContain('a done')
    expect(b.result!.content).toContain('b done')
  })

  it('foldCrossRunToolResults: a lone result in a later run without any invocation stays put', () => {
    const runs = splitAgentRuns([resultEntry(0, 'tc-x'), textEntry(1, 'done'), resultEntry(2, 'tc-y')])
    const regrouped = foldCrossRunToolResults(runs)
    // 没有可认领的调用：三条 run 原样（孤立结果自成一块的契约不破坏）。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    expect(regrouped).toBe(runs)
  })

  it('foldCrossRunToolResults: duplicate results — only the first is claimed, the rest stay', () => {
    const runs = splitAgentRuns([
      requestEntry(0, 'tc-1'),
      textEntry(1, '🛡 policy'),
      resultEntry(2, 'tc-1'),
      resultEntry(3, 'tc-1'),
    ])
    const regrouped = foldCrossRunToolResults(runs)
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    const first = mergeToolCalls((regrouped[0] as ProcessRun).items)
    expect(first).toHaveLength(1)
    expect((first[0] as ToolCallBlock).result).not.toBeNull()
    // 重复结果留在原地，走「未配对结果自成一块」的既有契约。
    const last = mergeToolCalls((regrouped[2] as ProcessRun).items)
    expect(last).toHaveLength(1)
    expect((last[0] as ToolCallBlock).result).toBeNull()
  })

  it('groupConversation: gated native turn renders ONE merged block in the invocation fold', () => {
    const entries: ConversationEntryView[] = [
      { position: 0, kind: 'prompt', element_type: 'markdown', content: 'go', created_at_unix: FIXED_DATES.T1 },
      requestEntry(1, 'toolu_1'),
      textEntry(2, '⏳ bash awaits approval'),
      resultEntry(3, 'toolu_1'),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units.find((u) => u.kind === 'agent')
    if (!agent || agent.kind !== 'agent') return expect.unreachable()
    const processRuns = agent.runs.filter((r) => r.type === 'process') as ProcessRun[]
    // 单棵树：整个回合只剩调用所在的一个过程 run。
    expect(processRuns).toHaveLength(1)
    const nodes = mergeToolCalls(processRuns[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.result).not.toBeNull()
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })

  it('render: one collapsed block inside the process fold; expanding reveals args + result', async () => {
    const entries: ConversationEntryView[] = [
      {
        position: 0,
        kind: 'prompt',
        element_type: 'markdown',
        content: 'perm',
        created_at_unix: FIXED_DATES.T1,
      },
      requestEntry(1, 'tc-1'),
      resultEntry(2, 'tc-1'),
    ]
    const el = await mount({ entries })
    // 单棵过程树：合并块在 .process-fold 内，不存在与过程折叠并列的顶层
    // 结果块；回合内只有 div.process-fold 一层树。
    const fold = el.shadowRoot!.querySelector<HTMLElement>('.process-fold')!
    expect(fold).toBeTruthy()
    expect(el.shadowRoot!.querySelector('[data-testid="tool-result-entry"]')).toBeNull()
    // 过程折叠收起行的 ✓/✗ 章（过程汇总行）在合并块展开前即可读。
    expect(fold.querySelector('[data-testid="tool-outcome"]')?.textContent).toContain('✓ 已执行')
    // 展开过程折叠：合并块在场且默认收起（aria-expanded=false、体不在 DOM）。
    fold.querySelector<HTMLButtonElement>('button.fold-link')!.click()
    await el.updateComplete
    const block = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="tool-result-entry"]')!
    expect(block).toBeTruthy()
    expect(block.closest('.process-fold')).toBe(fold)
    const blockLink = block.querySelector<HTMLButtonElement>('[data-testid="tool-result-link"]')!
    expect(blockLink.getAttribute('aria-expanded')).toBe('false')
    expect(block.querySelector('[data-testid="tool-call-body"]')).toBeNull()
    // 收起标题自带关键参数 + 结果章（不展开即可读）。
    expect(block.querySelector('.item-title')?.textContent).toContain('Bash · rm -rf /')
    expect(block.querySelector('[data-testid="tool-outcome"]')?.textContent).toContain('✓ 已执行')
    // 展开合并块：参数段 + 结果段都在；父折叠保持展开（D7 层级不死锁）。
    blockLink.click()
    await el.updateComplete
    expect(blockLink.getAttribute('aria-expanded')).toBe('true')
    const body = block.querySelector('[data-testid="tool-call-body"]')!
    expect(body.textContent).toContain('rm -rf /')
    expect(body.textContent).toContain('perm done')
    expect(
      fold.querySelector<HTMLButtonElement>('button.fold-link')!.getAttribute('aria-expanded'),
    ).toBe('true')
    // 再点收起：体离开 DOM，父折叠仍展开。
    blockLink.click()
    await el.updateComplete
    expect(block.querySelector('[data-testid="tool-call-body"]')).toBeNull()
    expect(
      fold.querySelector<HTMLButtonElement>('button.fold-link')!.getAttribute('aria-expanded'),
    ).toBe('true')
    el.remove()
  })

  it('render: denied result carries the ✗ chip while collapsed (permission-flow)', async () => {
    const entries: ConversationEntryView[] = [
      requestEntry(0, 'tc-1'),
      resultEntry(1, 'tc-1', '✓ **Bash**\ndenied by fake'),
    ]
    const el = await mount({ entries })
    el.shadowRoot!.querySelector<HTMLButtonElement>('.process-fold button.fold-link')!.click()
    await el.updateComplete
    const block = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="tool-result-entry"]')!
    expect(block.getAttribute('data-denied')).toBe('true')
    expect(block.querySelector('[data-testid="tool-outcome-denied"]')?.textContent).toContain(
      '✗ 已拒绝',
    )
    expect(block.querySelector('[data-testid="tool-outcome"]')).toBeNull()
    el.remove()
  })
})

// ── fix-webui-qa-round2 1.2/3.3：回执相位对空 prompt 条目的防御 ──────────────

describe('entriesAwaitReceipt ignores empty prompt entries (round2 1.2)', () => {
  it('an empty-content prompt tail is not a submission', () => {
    expect(
      entriesAwaitReceipt([
        { kind: 'content', content: 'hi' },
        { kind: 'prompt', content: '' },
      ]),
    ).toBe(false)
    expect(
      entriesAwaitReceipt([
        { kind: 'content', content: 'hi' },
        { kind: 'prompt', content: '/compact' },
      ]),
    ).toBe(true)
    // content 字段缺席（旧调用方形状）按非空对待，语义不变。
    expect(entriesAwaitReceipt([{ kind: 'prompt' }])).toBe(true)
    expect(entriesAwaitReceipt([{ kind: 'content', content: 'x' }])).toBe(false)
  })
})

// ── fix-webui-qa-round2 2.1：开卷边界登记（unread-cursor 侧） ────────────────

describe('opening-seam registry (round2 2.1)', () => {
  it('arm/peek/clear round-trips per session key', () => {
    armOpeningSeam('k1', 3)
    expect(peekOpeningSeam('k1')).toBe(3)
    armOpeningSeam('k2', null)
    expect(peekOpeningSeam('k2')).toBeNull()
    expect(peekOpeningSeam('k3')).toBeUndefined()
    // 同会话重复聚焦覆盖登记。
    armOpeningSeam('k1', 5)
    expect(peekOpeningSeam('k1')).toBe(5)
    clearOpeningSeam('k1')
    expect(peekOpeningSeam('k1')).toBeUndefined()
  })
})

// ── fix-webui-qa-round2 2.5（D-B13）：大批量分片摄入 ─────────────────────────

describe('flood ingest drains in chunks (round2 2.5, D-B13)', () => {
  it('a >50-entry turn.append queues and drains over frames without loss', async () => {
    const el = await mount({
      entries: [
        {
          position: 0,
          kind: 'prompt',
          element_type: 'markdown',
          content: 'flood',
          created_at_unix: FIXED_DATES.T1,
        },
      ],
    })
    const total = 1200
    const incoming: ConversationEntryView[] = Array.from({ length: total }, (_, i) => ({
      position: i + 1,
      kind: 'content',
      element_type: 'markdown',
      content: 'f'.concat(String(i)),
      created_at_unix: FIXED_DATES.T1,
    }))
    emitTurnAppend('oc_test', incoming)
    // 直渲染被分片路径替代：经若干 rAF 片段后全量收敛、顺序保真、零丢失。
    await el.updateComplete
    for (let i = 0; i < 40; i++) {
      await new Promise((r) => requestAnimationFrame(() => r(null)))
      await el.updateComplete
      if ((el as unknown as { streamQueue: unknown[] }).streamQueue.length === 0) break
    }
    const sebasEntries = (el as unknown as { streamEntries: ConversationEntryView[] })
      .streamEntries
    expect(sebasEntries).toHaveLength(total)
    expect(sebasEntries[0]!.content).toBe('f0')
    expect(sebasEntries[total - 1]!.content).toBe('f1199')
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('f1199')
    el.remove()
  })
})

// ── fix-webui-qa-round7 1.2（agent-workbench D1）：转录任意内容不破坏布局 ────
// happy-dom 无布局引擎，scrollWidth 断言不可得（浏览器旅程由 3c 承接）；
// 这里把**样式合同**钉死：约束链的三处内容层缺口逐条锁定在组件样式表上。

describe('transcript layout contract for arbitrary content (round7 1.2, D1)', () => {
  async function styleText(el: SebasTranscriptView): Promise<string> {
    return [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
  }

  it('the body carries a wrapping strategy that participates in min-content shrinking', async () => {
    // 2100+ 字符无空格 token 不撑破面板的机制面：.body 本体挂
    // overflow-wrap: anywhere（可继承，覆盖 markdown 管线的全部产出）。
    // 不参与 min-content 计算的 break-word 不得回归（旧缺陷根因）。
    const el = await mount({ entries: streamedTurn('x'.repeat(2100), ['ok'], FIXED_DATES.T1) })
    const css = await styleText(el)
    expect(css).toMatch(/\.turn-block \.body\s*\{[^}]*overflow-wrap:\s*anywhere/)
    // 流式纯文本尾巴同一策略（pre-wrap 可换行，长 token 同样要断）。
    expect(css).toMatch(
      /\.turn-block \.body\.text-live\s*\{[^}]*overflow-wrap:\s*anywhere/,
    )
    el.remove()
  })

  it('retires the old break-word-per-tag rule that caused the blowout', async () => {
    // 回归守卫：子标签级的 break-word 规则整体退役——anywhere 已在 .body
    // 继承生效，任何把 break-word 加回内容标签的写法都是缺陷回炉。
    const el = await mount({ entries: streamedTurn('hi', ['a'], FIXED_DATES.T1) })
    const css = await styleText(el)
    expect(css).not.toMatch(/overflow-wrap:\s*break-word/)
    el.remove()
  })

  it('GFM tables render as block-level horizontal scrollers, never blow out the parent', async () => {
    const el = await mount({ entries: streamedTurn('table', ['wide'], FIXED_DATES.T1) })
    const css = await styleText(el)
    expect(css).toMatch(
      /\.turn-block \.body table\s*\{[^}]*display:\s*block/,
    )
    expect(css).toMatch(
      /\.turn-block \.body table\s*\{[^}]*max-width:\s*100%[^}]*overflow-x:\s*auto/,
    )
    // 「查看全部」弹层同约束（不成为布局破坏旁路）。
    expect(css).toMatch(/\.view-all-body table\s*\{[^}]*overflow-x:\s*auto/)
    // 3c：table 内恢复 overflow-wrap:normal——anywhere 继承会把文字表格
    // 压到一字符宽，横滚永不触发；normal 让 min-content 超宽时走上横滚。
    expect(css).toMatch(/\.turn-block \.body table\s*\{[^}]*overflow-wrap:\s*normal/)
    expect(css).toMatch(/\.view-all-body table\s*\{[^}]*overflow-wrap:\s*normal/)
    // pre 代码块滚动语义不回退（round7 1.3：nowrap/滚动保持既有形态）。
    expect(css).toMatch(/\.turn-block \.body pre\s*\{[^}]*overflow-x:\s*auto/)
    el.remove()
  })

  it('meta author names carry the shrink guard so timestamps/receipt chips stay put', async () => {
    const el = await mount({ entries: streamedTurn('hi', ['a'], FIXED_DATES.T1) })
    const css = await styleText(el)
    expect(css).toMatch(
      /\.turn-block \.meta \.author\s*\{[^}]*min-width:\s*0[^}]*text-overflow:\s*ellipsis/,
    )
    el.remove()
  })

  it('timestamps sit inline after the author on both sides (round7 4.3)', async () => {
    // 单一约定：时间戳紧随作者名（用户气泡与 agent 行一致）。旧的
    // margin-left:auto 把 agent 时间戳推到整行右缘悬空——不得回归。
    const el = await mount({ entries: streamedTurn('hi', ['a'], FIXED_DATES.T1) })
    const css = await styleText(el)
    expect(css).toMatch(/\.turn-block \.meta \.time\s*\{[^}]*\}/)
    expect(css).not.toMatch(/\.turn-block \.meta \.time\s*\{[^}]*margin-left:\s*auto/)
    // 两侧 meta 行都渲染 time 节点（用户 + agent）。
    const times = el.shadowRoot!.querySelectorAll('.turn-block .meta time.time')
    expect(times.length).toBe(2)
    el.remove()
  })

  // ── fix-webui-qa-round10 2.1/2.2/2.3：直播态不空白 + 超宽内容不撑破容器 ──
  // C-DEF-01（直播态整面空白：DOM/aria 完好、console 零错误、reload 恢复）
  // 与 C-DEF-03（容器被超宽内容撑到 5350px 隐藏溢出、滚动区无可见滚动条）
  // 同域：超宽内容沿布局链撑出的数千 px 宽图层既是溢出面也是合成层失效
  // （空白绘制）的病灶。绘制级断言（非零绘制尺寸）需要真实浏览器，属
  // Playwright 链（GUI 复核留主 agent）；这里以 DOM/CSS 合同钉住机制面。

  it('the conversation scroll never scrolls horizontally — width pinned to layout (round10 2.2)', async () => {
    const el = await mount({
      entries: streamedTurn('table', ['wide'], FIXED_DATES.T1),
    })
    const css = await styleText(el)
    // .scroll 的 overflow-x 收敛为 hidden：overflow-y:auto 曾把缺省
    // visible 计算成 auto，超宽内容把容器撑出数千 px 隐藏横向溢出。
    const scrollRule = css.match(/\.scroll\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(scrollRule).toMatch(/overflow-x:\s*hidden/)
    expect(scrollRule).toMatch(/overflow-y:\s*auto/)
    el.remove()
  })

  it('turn blocks pin their inline size against oversized content (round10 2.3)', async () => {
    const el = await mount({
      entries: streamedTurn('table', ['wide'], FIXED_DATES.T1),
    })
    const css = await styleText(el)
    // .flow/.msg-block 挂 inline-size 包含：内容固有宽度不参与宽度计算，
    // 宽表/长代码行只能在条目内层滚动区横向滚动（对话面布局宽恒定）。
    const flowRule = css.match(/\.turn-block \.flow,\s*\n\s*\.turn-block \.msg-block\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(flowRule).toMatch(/contain:\s*inline-size/)
    expect(flowRule).toMatch(/min-width:\s*0/)
    expect(flowRule).toMatch(/max-width:/)
    // 行级收缩守卫：超宽内容的 min-content 不沿 flex 链上传。
    const turnRule = css.match(/\.turn-block\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(turnRule).toMatch(/min-width:\s*0/)
    expect(turnRule).toMatch(/max-width:\s*100%/)
    el.remove()
  })

  it('oversized turn after a wide-table turn keeps rendering entries in the DOM (round10 2.1)', async () => {
    // 直播态空白绘制的 DOM 半边合同：宽表回合定稿后连续流式回合，条目
    // 仍在文档中（绘制可见性由浏览器套件复核）。回放 QA 复现序列：
    // 宽表回合（含 <table> 的 markdown 定稿）→ 下一回合流式帧到达。
    const wideTable = [
      '| c001 | c002 | c003 |',
      '|---|---|---|',
      '| v | v | v |',
    ].join('\n')
    const entries: ConversationEntryView[] = [
      entry({ position: 0, kind: 'prompt', content: 'table', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: wideTable, created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'prompt', content: 'go on', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 3, kind: 'content', content: 'streaming tail', created_at_unix: FIXED_DATES.T3 }),
    ]
    const el = await mount({ entries, turnLive: true })
    // 两个回合块都在 DOM；宽表回合正文在档（markdown 管线 mock 下以源文
    // 本呈现）、流式尾回合有文本节点。
    const turns = el.shadowRoot!.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(turns.length).toBe(2)
    expect(turns[0]!.querySelector('.body')?.textContent).toContain('c001')
    expect(turns[1]!.querySelector('.body')?.textContent).toContain('streaming tail')
    // 条目存在于文档且带非零 DOM 尺寸语义（jsdom 无布局——绘制尺寸断言
    // 属浏览器套件；这里钉住「条目不缺失、不隐藏」的 DOM 前提）。
    for (const t of turns) {
      expect(t.hasAttribute('hidden')).toBe(false)
    }
    el.remove()
  })

  it('entry scroll regions present a visible scrollbar affordance (round10 2.2)', async () => {
    const el = await mount({ entries: streamedTurn('code', ['x'], FIXED_DATES.T1) })
    const css = await styleText(el)
    // 表格/代码块滚动区挂可见滚动条（Firefox scrollbar-* + Chromium
    // ::-webkit-scrollbar 双通道），右缘不再是无可发现的硬裁切。
    expect(css).toMatch(
      /\.turn-block \.body pre,\s*\n\s*\.turn-block \.body table[^{]*\{[^}]*scrollbar-width:\s*thin/,
    )
    expect(css).toMatch(
      /\.turn-block \.body pre::-webkit-scrollbar,\s*\n\s*\.turn-block \.body table::-webkit-scrollbar[^{]*\{[^}]*height:\s*8px/,
    )
    expect(css).toMatch(
      /\.turn-block \.body pre::-webkit-scrollbar-thumb,[^{]*\{[^}]*background:\s*var\(--sebas-border-strong/,
    )
    el.remove()
  })
})

// ─── fix-webui-qa-round8：滚动浮标 / 升级降级 / 模型留痕 / 围栏容错 / 思考聚合 ──

describe('fix-webui-qa-round8: scroll pill, contract entries, fence, thinking merge', () => {
  it('closeUnclosedFence closes an unbalanced fence and leaves balanced content alone (7.2)', async () => {
    const { closeUnclosedFence } = await import('./transcript-view.js')
    const unclosed = 'code:\n```js\nlet x = 1'
    expect(closeUnclosedFence(unclosed)).toBe(unclosed + '\n```')
    expect(closeUnclosedFence('```js\nok\n```')).toBe('```js\nok\n```')
    expect(closeUnclosedFence('plain text')).toBe('plain text')
    // 成对围栏夹单行文本：仍配平。
    expect(closeUnclosedFence('```a\n```\nmid\n```b\n```')).toBe('```a\n```\nmid\n```b\n```')
  })

  it('an unclosed fence in one entry never swallows later entries of the same run (7.2)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({
          position: 1,
          kind: 'content',
          content: 'code:\n```js\nlet x',
          created_at_unix: FIXED_DATES.T1,
        }),
        entry({ position: 2, kind: 'content', content: 'plain prose after', created_at_unix: FIXED_DATES.T1 }),
      ],
    })
    // 渲染管线里，第二条目的围栏已收口：run 拼接体以闭合围栏结尾，
    // 后续条目按正常形态渲染（不被卷进代码块）。
    const run = el.shadowRoot!.querySelector('.turn-block.is-assistant .body')!
    expect(run.textContent).toContain('plain prose after')
    const source = (renderMarkdown as ReturnType<typeof vi.fn>).mock.calls
      .map((c) => c[0] as string)
      .find((src) => src.includes('plain prose after'))!
    // 收口围栏落在该条目自己的末尾（后续正文之前）。
    const closingIdx = source.indexOf('let x')
    expect(source.slice(closingIdx, closingIdx + 12).startsWith('let x\n```')).toBe(true)
    el.remove()
  })

  it('mergeAdjacentThinking merges adjacent thinking items and keeps tool boundaries (7.1)', async () => {
    const { mergeAdjacentThinking } = await import('./transcript-view.js')
    const items: ProcessItem[] = [
      { elementType: 'thinking', content: 'a', title: null, position: 1 },
      { elementType: 'thinking', content: 'b', title: null, position: 2 },
      { elementType: 'tool', content: 'x', title: null, position: 3 },
      { elementType: 'thinking', content: 'c', title: null, position: 4 },
    ]
    const merged = mergeAdjacentThinking(items)
    expect(merged).toHaveLength(3)
    expect(merged[0]).toMatchObject({ elementType: 'thinking', content: 'ab', position: 1 })
    expect(merged[1]?.elementType).toBe('tool')
    expect(merged[2]).toMatchObject({ elementType: 'thinking', content: 'c', position: 4 })
  })

  it('escalate_downgrade entries render as a neutral system entry with the reason (1.2)', async () => {
    const el = await mount({
      entries: [
        entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
        entry({
          position: 1,
          kind: 'content',
          element_type: 'escalate_downgrade',
          content: JSON.stringify({
            request_id: 'req-1',
            tool: 'Bash',
            reason: '需要 sudo 装依赖',
            detail: '该决策已按「仅放行一次」降级执行。',
          }),
          created_at_unix: FIXED_DATES.T2,
        }),
      ],
    })
    const unit = el.shadowRoot!.querySelector<HTMLElement>(
      '[data-testid="escalate-downgrade-entry"]',
    )!
    expect(unit).toBeTruthy()
    expect(unit.textContent).toContain('仅放行一次')
    expect(unit.textContent).toContain('Bash')
    expect(
      unit.querySelector('[data-testid="escalate-downgrade-reason"]')?.textContent,
    ).toContain('需要 sudo 装依赖')
    // 中性系统条目，不是错误红泡。
    expect(unit.querySelector('.bubble.error')).toBeNull()
    el.remove()
  })

  it('model_change entries render as a neutral system entry with both models (5.2)', async () => {
    const el = await mount({
      entries: [
        entry({
          position: 0,
          kind: 'content',
          element_type: 'model_change',
          content: JSON.stringify({ from: 'test/text', to: 'test/long' }),
          created_at_unix: FIXED_DATES.T1,
        }),
      ],
    })
    const unit = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="model-change-entry"]')!
    expect(unit).toBeTruthy()
    expect(unit.textContent).toContain('test/text')
    expect(unit.textContent).toContain('test/long')
    expect(unit.textContent).toContain('模型已切换')
    // 新条目独立成单元、不卷进正文 run（seam 段数口径不受影响）。
    expect(
      groupConversation([
        entry({ position: 0, kind: 'content', element_type: 'model_change', content: '{}' }),
      ]).map((u) => u.kind),
    ).toEqual(['model_change'])
    el.remove()
  })

  it('the jump-latest pill appears when detached with content below and restores follow on click (3.1)', async () => {
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
    const el = await mount({
      entries: streamedTurn('q', ['a'], FIXED_DATES.T1),
    })
    const scrollEl = el.shadowRoot!.querySelector<HTMLElement>('.scroll')!
    // 伪造几何：内容高于视口（有滚动余量）。
    Object.defineProperty(scrollEl, 'scrollHeight', { value: 2000, configurable: true })
    Object.defineProperty(scrollEl, 'clientHeight', { value: 400, configurable: true })
    scrollEl.scrollTop = 0

    // 操作者主动上滚（距底超阈值）→ 脱离跟随 → 浮标出现。
    scrollEl.dispatchEvent(new Event('scroll'))
    await el.updateComplete
    expect((el as unknown as { sticky: boolean }).sticky).toBe(false)
    const pill = el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="jump-latest"]')!
    expect(pill.hasAttribute('hidden')).toBe(false)

    // 点击浮标 → 回底 + 恢复跟随 + 浮标消失 + 按贴底语义推进已读。
    pill.click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 300)) // 盖过 MARK_SEEN_DEBOUNCE_MS=250
    expect((el as unknown as { sticky: boolean }).sticky).toBe(true)
    expect(
      el.shadowRoot!.querySelector('[data-testid="jump-latest"]')!.hasAttribute('hidden'),
    ).toBe(true)
    expect(store.get('sebas:seen:oc_test')).not.toBeNull()
    el.remove()
  })

  it('未读缝开卷保持脱离：浮标承接回底，回底后恢复跟随（3.1）', async () => {
    // 开卷带未读缝：边界冻结在锚 0、msgCount=4（seam 在场，jsdom 无滚动
    // 几何 → fit-viewport 结算不触发，边界保留）。定位停在中部——sticky
    // 保持脱离（后续到达不把操作者拽离 seam、不把未见内容计为已见），
    // 「跳到最新」浮标是回底 + 恢复跟随的桥。
    // jsdom 没有 scrollIntoView：垫一个空实现让开卷定位分支真实走到。
    Element.prototype.scrollIntoView = () => {}
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 0 }))
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
    expect((el as unknown as { sticky: boolean }).sticky).toBe(false)
    const scrollEl = el.shadowRoot!.querySelector<HTMLElement>('.scroll')!
    Object.defineProperty(scrollEl, 'scrollHeight', { value: 2000, configurable: true })
    Object.defineProperty(scrollEl, 'clientHeight', { value: 400, configurable: true })
    // 新条目到达：浮标出现（未贴底 + 内容在下方），锚不推进（非「看着到达」）。
    emitTurnAppend('oc_test', [
      entry({
        position: 99,
        kind: 'content',
        content: 'fresh arrival',
        created_at_unix: FIXED_DATES.T5,
      }),
    ])
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    expect((el as unknown as { sticky: boolean }).sticky).toBe(false)
    const pill = el.shadowRoot!.querySelector('[data-testid="jump-latest"]')!
    expect(pill.hasAttribute('hidden')).toBe(false)
    expect(store.get('sebas:seen:oc_test')).toBe(JSON.stringify({ anchor_count: 0 }))
    // 点击浮标：回底 + 跟随重新咬合 + 按贴底语义推进已读（seam 清账）。
    ;(pill as HTMLButtonElement).click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 300)) // 盖过 MARK_SEEN_DEBOUNCE_MS=250
    expect((el as unknown as { sticky: boolean }).sticky).toBe(true)
    expect(
      el.shadowRoot!.querySelector('[data-testid="jump-latest"]')!.hasAttribute('hidden'),
    ).toBe(true)
    expect(store.get('sebas:seen:oc_test')).not.toBe(JSON.stringify({ anchor_count: 0 }))
    el.remove()
  })

  it('全读开卷保持贴底跟随：定位不把 sticky 翻 false（3.1 的另一半）', async () => {
    // 无未读（锚=段数）：seam 不在场 → 开卷不进入 seam 定位分支，sticky
    // 保持 true——round8 的误伤面（开卷程序化定位把跟随停摆）不再存在。
    store.set('sebas:seen:oc_test', JSON.stringify({ anchor_count: 2 }))
    const el = await mount({
      entries: streamedTurn('q', ['a', 'b'], FIXED_DATES.T1),
      msgCount: 2,
    })
    expect((el as unknown as { sticky: boolean }).sticky).toBe(true)
    expect(el.shadowRoot!.querySelector('.seam')?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })
})
