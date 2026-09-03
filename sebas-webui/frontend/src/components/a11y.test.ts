// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import './status-badge.js'
import type { SebasStatusBadge } from './status-badge.js'

describe('accessibility baseline', () => {
  it('status badge text remains available in greyscale (label + glyph nodes)', async () => {
    const el = document.createElement('sebas-status-badge') as SebasStatusBadge
    el.slug = 'failed'
    el.label = 'Failed'
    el.glyph = '✕'
    document.body.appendChild(el)
    await el.updateComplete
    const label = el.shadowRoot?.querySelector('.label')
    const glyph = el.shadowRoot?.querySelector('.glyph')
    // Shape and word nodes exist: greyscale cannot erase the state.
    expect(label?.textContent).toBe('Failed')
    expect(glyph?.textContent).toBe('✕')
    el.remove()
  })

  it('reduced motion + theme class hooks ship in tokens.css, focus ring in app.css', async () => {
    const fs = await import('node:fs')
    const path = await import('node:path')
    const here = path.dirname(new URL(import.meta.url).pathname)
    const tokens = fs.readFileSync(path.join(here, '../../src/styles/tokens.css'), 'utf8')
    expect(tokens).toContain('prefers-reduced-motion')
    // Light re-map is keyed on the single wa-dark switch (see src/theme.ts),
    // so an explicit Settings → Appearance override can beat the OS preference.
    expect(tokens).toContain(':root:not(.wa-dark)')
    // 5.5: --signal token exists and is referenced outside the token definition.
    expect(tokens).toContain('--sebas-signal')
    const reviewCard = fs.readFileSync(path.join(here, '../components/review-card.ts'), 'utf8')
    expect(reviewCard).toContain('--sebas-signal')
    const composer = fs.readFileSync(path.join(here, '../views/workbench-composer.ts'), 'utf8')
    expect(composer).toContain('--sebas-signal')
    const app = fs.readFileSync(path.join(here, '../../src/styles/app.css'), 'utf8')
    expect(app).toContain(':focus-visible')
  })

  it('close buttons carry accessible names in the sessions view', async () => {
    const { readFileSync } = await import('node:fs')
    const { dirname, join } = await import('node:path')
    const hereDir = dirname(new URL(import.meta.url).pathname)
    const src = readFileSync(join(hereDir, '../views/sessions.ts'), 'utf8')
    // Close is an icon-ish destructive control in row context: the template
    // must give it an accessible name derived from the row key.
    expect(src).toContain('aria-label=${`Close session ${row.chat_id}`}')
  })
})
