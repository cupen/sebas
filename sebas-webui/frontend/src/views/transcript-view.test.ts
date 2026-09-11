// @vitest-environment jsdom
/**
 * sebas-transcript-view — the conversation view
 * (workbench-conversation-view 2.1–2.5).
 *
 * The component groups the ordered entry sequence into turns (a prompt
 * opens an operator turn; agent chunks until the next prompt form ONE
 * bubble), chunks each agent turn into text/thinking/tools blocks, and
 * runs a turn-counting seen-boundary seam. Scenarios:
 *
 *   2.1  N streamed chunks → one agent bubble; both conversation sides in order
 *   2.2  text→tool→text yields a three-segment structure; thinking folds
 *   2.3  operator turns render as "你" bubbles, alternating with agent turns
 *   2.4  a multi-chunk turn counts as ONE unseen turn and the seam never
 *        splits a turn
 *   2.5  empty entries skipped, keyboard-operable folds, fill mode intact
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
  groupConversation,
  mergeSpawnErrors,
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
}): Promise<SebasTranscriptView> {
  const el = document.createElement('sebas-transcript-view') as SebasTranscriptView
  el.entries = opts.entries
  el.sessionKey = opts.sessionKey ?? 'oc_test'
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

  it('text → tool → text chunks into a three-segment structure (D4)', () => {
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
    expect(agent.blocks.map((b) => b.type)).toEqual(['text', 'tools', 'text'])
    // The tool group sits BETWEEN the two text segments with both items.
    const tools = agent.blocks[1]
    if (tools.type !== 'tools') return expect.unreachable()
    expect(tools.items).toHaveLength(2)
  })

  it('thinking runs fold into their own blocks, keeping position order', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'why', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: ' aha', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', content: 'because', created_at_unix: FIXED_DATES.T2 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.blocks.map((b) => b.type)).toEqual(['thinking', 'text'])
    const th = agent.blocks[0]
    if (th.type !== 'thinking') return expect.unreachable()
    expect(th.content).toBe('hmm aha')
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

  it('text → tool → text renders three segments and the tool group is expandable (2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'let me check.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '📖 **read_file**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **read_file**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', content: 'done.', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    // 三段式：text 段、工具组、text 段。
    const segments = assistant.querySelectorAll<HTMLElement>('.bubble > .body:not(.fold-body)')
    expect(segments.length).toBe(2)
    expect(segments[0].textContent).toContain('let me check.')
    expect(segments[1].textContent).toContain('done.')
    const tools = assistant.querySelector<HTMLDetailsElement>('details.tools-fold')!
    expect(tools).toBeTruthy()
    expect(tools.getAttribute('data-tool-count')).toBe('2')
    const label = tools.querySelector<HTMLElement>('summary .label')
    expect(label?.textContent?.trim()).toBe('used 2 tools')
    // 默认折叠，summary 可展开（原生 details/summary：键盘可操作）。
    expect(tools.open).toBe(false)
    tools.querySelector<HTMLElement>('summary')!.click()
    await el.updateComplete
    expect(tools.open).toBe(true)
    expect(tools.textContent).toContain('read_file')
  })

  it('thinking folds inside the agent bubble and stays collapsed by default (2.2)', async () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'why', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'deep thought', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', content: 'because', created_at_unix: FIXED_DATES.T2 }),
    ]
    const el = await mount({ entries })
    const assistant = el.shadowRoot!.querySelector<HTMLElement>('.turn-block.is-assistant')!
    const fold = assistant.querySelector<HTMLDetailsElement>('details.thinking-fold')!
    expect(fold).toBeTruthy()
    expect(fold.open).toBe(false)
    expect(fold.querySelector('summary .label')?.textContent?.trim()).toBe('thinking')
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

    // 未读态 → mark all seen → localStorage 写入最大时间戳、seam 消失。
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1 - 10))
    el.entries = [...entries]
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete
    seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    seam!.querySelector<HTMLButtonElement>('button.link')!.click()
    await el.updateComplete
    expect(store.get('sebas:seen:oc_test')).toBe(String(FIXED_DATES.T1))
    expect(el.shadowRoot?.querySelector<HTMLElement>('.seam')?.hasAttribute('hidden')).toBe(true)
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
})
