import { describe, expect, it } from 'vitest'
import './status-badge.js'
import type { SebasStatusBadge } from './status-badge.js'

const SLUGS = ['starting', 'queued', 'working', 'done', 'failed', 'dormant']
const GLYPHS: Record<string, string> = {
  starting: '◇',
  queued: '▹',
  working: '▶',
  done: '✓',
  failed: '✕',
  dormant: '·',
}

describe('sebas-status-badge', () => {
  it('renders label and glyph (status never colour-only)', async () => {
    const el = document.createElement('sebas-status-badge') as SebasStatusBadge
    el.slug = 'working'
    el.label = 'Working'
    el.glyph = GLYPHS['working']
    document.body.appendChild(el)
    await el.updateComplete
    const text = el.shadowRoot?.textContent ?? ''
    expect(text).toContain('Working')
    expect(text).toContain('▶')
    // The slug is exposed for stylesheet hooking, like the SSR data-status.
    expect(el.shadowRoot?.querySelector('[data-status="working"]')).toBeTruthy()
    el.remove()
  })

  it('resolves a distinct CSS variable per slug', async () => {
    const colors = new Set<string>()
    for (const slug of SLUGS) {
      const el = document.createElement('sebas-status-badge') as SebasStatusBadge
      el.slug = slug
      el.label = slug
      el.glyph = GLYPHS[slug] ?? ''
      document.body.appendChild(el)
      await el.updateComplete
      const dot = el.shadowRoot?.querySelector<HTMLElement>('.dot')
      expect(dot?.getAttribute('style')).toContain(`--sebas-status-${slug}`)
      colors.add(dot?.getAttribute('style') ?? '')
      el.remove()
    }
    // Distinct variables per slug — the greyscale/colour-blindness channel.
    expect(colors.size).toBe(SLUGS.length)
  })

  it('falls back to the slug as label when none provided', async () => {
    const el = document.createElement('sebas-status-badge') as SebasStatusBadge
    el.slug = 'done'
    document.body.appendChild(el)
    await el.updateComplete
    expect(el.shadowRoot?.textContent).toContain('done')
    el.remove()
  })
})
