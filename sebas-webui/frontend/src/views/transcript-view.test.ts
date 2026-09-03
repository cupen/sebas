// @vitest-environment jsdom
/**
 * sebas-transcript-view behaviour. The component owns its own seen-boundary
 * (localStorage) and seam visualisation, so we drive it directly rather
 * than through the parent view.
 *
 * Eight scenarios:
 *   - timestamps rendered inside each bubble's meta row with correct datetime attrs
 *   - thinking blocks collapsed by default
 *   - no seam when everything is seen
 *   - seam pill with "~N new since you last viewed" wording
 *   - seam pill copy starts with "~"
 *   - in-place entry update does not move the seam (spec 4.4)
 *   - mark-all-seen link writes localStorage and hides the seam
 *   - empty content entries are skipped
 *
 * The localStorage polyfill below replaces whatever jsdom ships so the
 * tests stay deterministic across environments and so the production
 * code path (which reads/writes through the global) is exercised
 * verbatim.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { CardElementView } from '../api/client.js'
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
  entries: CardElementView[]
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
}

function makeEntries(): CardElementView[] {
  return [
    { element_type: 'markdown', content: 'first', created_at_unix: FIXED_DATES.T1 },
    { element_type: 'markdown', content: 'second', created_at_unix: FIXED_DATES.T2 },
    { element_type: 'markdown', content: 'third', created_at_unix: FIXED_DATES.T3 },
  ]
}

// ---- tests ------------------------------------------------------------

describe('sebas-transcript-view', () => {
  it('renders each timestamp inside its bubble meta row', async () => {
    const el = await mount({ entries: makeEntries() })
    const times = el.shadowRoot?.querySelectorAll<HTMLTimeElement>(
      '.turn-block .bubble .meta time.time',
    )
    expect(times?.length).toBe(3)
    expect(times?.[0]?.getAttribute('datetime')).toBe(
      new Date(FIXED_DATES.T1 * 1000).toISOString(),
    )
    expect(times?.[1]?.getAttribute('datetime')).toBe(
      new Date(FIXED_DATES.T2 * 1000).toISOString(),
    )
    expect(times?.[2]?.getAttribute('datetime')).toBe(
      new Date(FIXED_DATES.T3 * 1000).toISOString(),
    )
  })

  it('collapses thinking blocks by default', async () => {
    const entries: CardElementView[] = [
      { element_type: 'markdown', content: 'visible', created_at_unix: FIXED_DATES.T1 },
      { element_type: 'thinking', content: 'hidden', created_at_unix: FIXED_DATES.T2 },
    ]
    const el = await mount({ entries })
    // Markdown is rendered inline (not wrapped in <details>).
    const sections = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block')
    expect(sections?.length).toBe(2)
    const markdownSection = sections?.[0]
    const thinkingSection = sections?.[1]
    expect(markdownSection?.querySelector('details')).toBeNull()
    expect(thinkingSection?.querySelector('details')).not.toBeNull()
    const summary = thinkingSection?.querySelector<HTMLElement>('summary')
    expect(summary?.textContent?.trim()).toBe('thinking')
    const details = thinkingSection?.querySelector<HTMLDetailsElement>('details')
    expect(details?.open).toBe(false)
  })

  it('does not show a seam when all entries are seen', async () => {
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T3 + 100))
    const el = await mount({ entries: makeEntries() })
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam).not.toBeNull()
    expect(seam?.hasAttribute('hidden')).toBe(true)
    // Pill text should not be visible at all when hidden.
    expect(seam?.textContent ?? '').not.toContain('new since you last viewed')
  })

  it('shows a seam with the unseen count when entries are newer than the boundary', async () => {
    // Stored boundary at T1 — entries T2 and T3 are unseen, T1 sits at
    // or before the boundary (strictly-greater rule means T1 is not
    // unseen). So unseenCount = 2.
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1))
    const el = await mount({ entries: makeEntries() })
    const seam = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(false)
    expect(seam?.textContent ?? '').toContain('~2 new since you last viewed')
  })

  it('seam pill copy starts with "~"', async () => {
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1))
    const el = await mount({ entries: makeEntries() })
    const pill = el.shadowRoot?.querySelector<HTMLElement>('.seam .pill')
    const text = (pill?.textContent ?? '').trim()
    expect(text.startsWith('~')).toBe(true)
  })

  it('keeps the seam in place when an entry updates without a new timestamp', async () => {
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1))
    const el = await mount({ entries: makeEntries() })
    // Initial state: seam at index 1, unseen = 2.
    expect((el as unknown as { seamIndex: number | null }).seamIndex).toBe(1)
    expect((el as unknown as { unseenCount: number }).unseenCount).toBe(2)

    // In-place update of entry at index 0 — content changes, timestamp
    // is the same. The router never bumps `created_at_unix` on a
    // refresh, so the seam must remain anchored at index 1.
    const next = [...makeEntries()]
    next[0] = { ...next[0], content: 'first (revised)' }
    el.entries = next
    await el.updateComplete
    await new Promise((r) => requestAnimationFrame(() => r(null)))
    await el.updateComplete

    expect((el as unknown as { seamIndex: number | null }).seamIndex).toBe(1)
    expect((el as unknown as { unseenCount: number }).unseenCount).toBe(2)
  })

  it('hides the seam and writes localStorage when "mark all seen" is clicked', async () => {
    store.set('sebas:seen:oc_test', String(FIXED_DATES.T1))
    const el = await mount({ entries: makeEntries() })
    const seamBefore = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seamBefore?.hasAttribute('hidden')).toBe(false)

    // The "mark all seen" control is rendered as a <button class="link">
    // inside the seam strip.
    const link = el.shadowRoot?.querySelector<HTMLElement>('.seam button.link, .seam a')
    expect(link).not.toBeNull()
    link?.click()
    await el.updateComplete

    expect(store.get('sebas:seen:oc_test')).toBe(String(FIXED_DATES.T3))
    const seamAfter = el.shadowRoot?.querySelector<HTMLElement>('.seam')
    expect(seamAfter?.hasAttribute('hidden')).toBe(true)
    expect((el as unknown as { unseenCount: number }).unseenCount).toBe(0)
  })

  it('skips entries with empty content', async () => {
    const entries: CardElementView[] = [
      { element_type: 'markdown', content: '', created_at_unix: FIXED_DATES.T1 },
      { element_type: 'markdown', content: 'kept', created_at_unix: FIXED_DATES.T2 },
    ]
    const el = await mount({ entries })
    const sections = el.shadowRoot?.querySelectorAll<HTMLElement>('.turn-block')
    expect(sections?.length).toBe(1)
    // The remaining entry carries the non-empty content.
    expect(sections?.[0]?.textContent ?? '').toContain('kept')
  })

  it('fill mode lifts the 58vh cap and flexes the scroll region', async () => {
    const el = await mount({ entries: makeEntries() })
    // 非 fill：宿主无 fill 属性，.scroll 规则里带 58vh 封顶。
    expect(el.hasAttribute('fill')).toBe(false)
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(styleText).toContain('58vh')
    // fill 模式：宿主反射 fill 属性，滚动区封顶取消（规则切换而非新节点）。
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
