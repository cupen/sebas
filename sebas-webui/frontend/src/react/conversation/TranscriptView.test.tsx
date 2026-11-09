import { cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { ConversationElementType, ConversationEntryView } from '../../api/client.js'
import { setMarkdownObserver } from '../../components/markdown.js'
import { TranscriptView } from './TranscriptView.js'

afterEach(cleanup)

let parses = 0
let sanitizes = 0
let highlights = 0

beforeEach(() => {
  parses = 0
  sanitizes = 0
  highlights = 0
  setMarkdownObserver({
    parse: () => (parses += 1),
    sanitize: () => (sanitizes += 1),
    highlight: () => (highlights += 1),
  })
})

function entry(
  position: number,
  kind: 'prompt' | 'content',
  element_type: ConversationElementType,
  content: string,
): ConversationEntryView {
  return { position, kind, element_type, content, created_at_unix: 1_700_000_000 + position }
}

describe('TranscriptView live tail', () => {
  it('renders the engaged tail as plain text and parses Markdown once on settlement', () => {
    const entries = [
      entry(0, 'prompt', 'markdown', 'question'),
      entry(1, 'content', 'markdown', '**live answer**'),
    ]
    const view = render(<TranscriptView entries={entries} live />)
    expect(view.container.querySelectorAll(`pre`)[1]?.textContent).toBe('**live answer**')
    expect(view.container.querySelector(`[class*="markdown"]`)).toBeNull()
    expect(parses).toBe(0)
    expect(sanitizes).toBe(0)

    view.rerender(<TranscriptView entries={entries} live={false} />)
    expect(view.container.querySelector(`strong`)?.textContent).toBe('live answer')
    expect(parses).toBe(1)
    expect(sanitizes).toBe(1)
  })

  it('does not re-render historical units or re-parse historical Markdown per delta', () => {
    const entries = [
      entry(0, 'prompt', 'markdown', 'first'),
      entry(1, 'content', 'markdown', 'settled **one**'),
      entry(2, 'prompt', 'markdown', 'second'),
      entry(3, 'content', 'markdown', 'tail'),
    ]
    const renders = new Map<string, number>()
    const onUnitRender = (key: string) => renders.set(key, (renders.get(key) ?? 0) + 1)
    const view = render(
      <TranscriptView entries={entries} live onUnitRender={onUnitRender} />,
    )
    const before = new Map(renders)
    expect(parses).toBe(1)

    let next = entries
    for (let i = 0; i < 3; i += 1) {
      next = [...next, entry(4 + i, 'content', 'markdown', `chunk${i} `)]
      view.rerender(
        <TranscriptView entries={next} live onUnitRender={onUnitRender} />,
      )
    }

    expect(renders.get('op-0')).toBe(before.get('op-0'))
    expect(renders.get('agent-1')).toBe(before.get('agent-1'))
    expect(renders.get('op-2')).toBe(before.get('op-2'))
    expect(renders.get('agent-3')).toBeGreaterThan(before.get('agent-3') ?? 0)
    expect(parses, 'no Markdown parse during streaming').toBe(1)

    view.rerender(
      <TranscriptView entries={next} live={false} onUnitRender={onUnitRender} />,
    )
    expect(parses, 'settlement parses exactly the tail once').toBe(2)
  })
})
