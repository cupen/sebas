// @vitest-environment jsdom
/**
 * sebas-transcript-view — the conversation view
 * (workbench-conversation-view 2.1–2.5 + workbench-agent-identity-and-
 * process-folds 2.1–3.2).
 *
 * The component groups the ordered entry sequence into turns (a prompt
 * opens an operator turn; agent chunks until the next prompt form ONE
 * bubble), chunks each agent turn into text segments + ONE process block,
 * and runs a turn-counting seen-boundary seam. Scenarios:
 *
 *   2.1  N streamed chunks → one agent bubble; all thinking+tool entries
 *        fold into ONE process block at the first process entry position;
 *        text segments stay outside in stream order
 *   2.2  process fold collapsed by default; second-level per-entry folds
 *        also collapsed, titled by the entry title (generic fallback),
 *        DOM identity keyed by position
 *   2.3  middle truncation of long titles (grapheme-safe)
 *   2.4  a multi-chunk turn counts as ONE unseen turn and the seam never
 *        splits a turn
 *   2.5  empty entries skipped, keyboard-operable folds, fill mode intact
 *   3.1  assistant author label display → slug → assistant fallback
 *   3.2  "已收到" receipt badge while the prompt is still the newest entry
 *        (pure entry-sequence: queued/non-working included); gone once the
 *        agent reply arrives
 *
 * The localStorage polyfill below replaces whatever jsdom ships so the
 * tests stay deterministic across environments and so the production
 * code path (which reads/writes through the global) is exercised
 * verbatim.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { ConversationEntryView } from '../api/client.js'
import {
  ERROR_MERGE_WINDOW_SECS,
  awaitingReceipt,
  groupConversation,
  mergeSpawnErrors,
  middleTruncate,
  resolveAgentDisplay,
} from './transcript-view.js'
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

// ---- pure-function coverage -------------------------------------------

describe('groupConversation (design D3/D4)', () => {
  it('one agent turn from N chunks — turn grouping, not entry grouping', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('do it', ['a', 'b', 'c', 'd'], FIXED_DATES.T1)),
    )
    expect(units).toHaveLength(2)
    expect(units[0].kind).toBe('operator')
    expect(units[1].kind).toBe('agent')
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    const text = agent.blocks[0]
    expect(text.type).toBe('text')
    if (text.type !== 'text') return expect.unreachable()
    expect(text.content).toBe('abcd')
  })

  it('text → tool → text: both tool entries land in ONE process block at the first process position (2.1)', () => {
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
    expect(agent.blocks.map((b) => b.type)).toEqual(['text', 'process', 'text'])
    // 单一过程块定位在首个过程条目（position 2），两条工具都在里面。
    const proc = agent.blocks[1]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.position).toBe(2)
    expect(proc.items.map((it) => it.position)).toEqual([2, 3])
  })

  it('a multi-run turn folds ALL thinking+tool entries into the single process block (2.1)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', content: 'mid text.', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 5, kind: 'content', element_type: 'thinking', content: 'reconsider', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 6, kind: 'content', element_type: 'tool', content: '✓ **bash**', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 7, kind: 'content', content: 'final.', created_at_unix: FIXED_DATES.T3 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    // 过程块只有一个、定位在首个过程条目（position 1）；文本段按流序留在块外。
    expect(agent.blocks.map((b) => b.type)).toEqual(['process', 'text', 'text', 'text'])
    const proc = agent.blocks[0]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.position).toBe(1)
    expect(proc.items.map((it) => it.position)).toEqual([1, 3, 5, 6])
    expect(proc.items.map((it) => it.elementType)).toEqual(['thinking', 'tool', 'thinking', 'tool'])
    const texts = agent.blocks.flatMap((b) => (b.type === 'text' ? [b.content] : []))
    expect(texts).toEqual(['step one.', 'mid text.', 'final.'])
  })

  it('a pure-text turn produces no process fold (2.1)', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('hi', ['a', 'b', 'c'], FIXED_DATES.T1)),
    )
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.blocks.map((b) => b.type)).toEqual(['text'])
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
    const proc = agent.blocks[0]
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

// ---- middle truncation (2.3, design D5) ---------------------------------

describe('middleTruncate (2.3/D5)', () => {
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
  it('renders N streamed chunks as ONE assistant bubble (2.1)', async () => {
    const el = await mount({
      entries: streamedTurn('do it', ['chunk one ', 'chunk two ', 'chunk three'], FIXED_DATES.T1),
    })
    const assistant = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(assistant?.length).toBe(1)
    const bodies = assistant?.[0]?.querySelectorAll<HTMLElement>('.bubble > .body')
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

  it('mixed turn renders ONE process fold; text segments stay outside in stream order (2.1)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', content: 'mid text.', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 5, kind: 'content', element_type: 'thinking', content: 'reconsider', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 6, kind: 'content', element_type: 'tool', content: '✓ **bash**', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 7, kind: 'content', content: 'final.', created_at_unix: FIXED_DATES.T3 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 全部过程条目只折一次（旧结构会是 thinking 折叠 + 工具组两个折叠）。
    const folds = assistant.querySelectorAll<HTMLDetailsElement>('details.process-fold')
    expect(folds.length).toBe(1)
    expect(folds[0].getAttribute('data-process-count')).toBe('4')
    // 文本段按流序留在折叠外（三段）。
    const segments = assistant.querySelectorAll<HTMLElement>('.bubble > .body:not(.fold-body)')
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
    const bodies = assistant.querySelectorAll<HTMLElement>('.bubble > .body')
    expect(bodies.length).toBe(1)
    expect(bodies[0].textContent).toContain('chunk one')
  })

  it('process fold and second-level folds collapse by default; titles show with generic fallback (2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'deep thought', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '📖 **read_file**', title: 'read_file · src/app.ts', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **read_file**', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    const fold = assistant.querySelector<HTMLDetailsElement>('details.process-fold')!
    expect(fold.getAttribute('data-process-count')).toBe('3')
    expect(fold.querySelector('summary .label')?.textContent?.trim()).toBe('process · 3')
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
    // 边界不切开回合：seam 下方第一个块是完整回合的开头（operator 气泡），
    // 第二个块就是那个完整的七 chunk agent 气泡。
    const seamNext = seam?.nextElementSibling
    // 边界落在未读第一个回合（operator 气泡）上方，不切开任何回合。
    expect(seamNext?.classList.contains('is-user')).toBe(true)
    const agentBubbles = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block.is-assistant')
    expect(agentBubbles?.length).toBe(2)
    expect(agentBubbles?.[1]?.textContent).toContain('a')
    expect(agentBubbles?.[1]?.textContent).toContain('g')
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

  it('timestamps render inside each bubble meta row with datetime attrs', async () => {
    const el = await mount({
      entries: streamedTurn('hi', ['a', 'b'], FIXED_DATES.T1),
    })
    const times = el.shadowRoot?.querySelectorAll<HTMLTimeElement>(
      '.turn-block .bubble .meta time.time',
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
    // 角标挂在最后一条操作者气泡上。
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
})
