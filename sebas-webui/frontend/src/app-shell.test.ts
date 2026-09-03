// @vitest-environment jsdom
/**
 * Primary-navigation demotion (tasks 7.1): `/sessions` is gone from the
 * shell's primary nav while the route itself keeps resolving (rail link +
 * old deep links), every remaining nav link matches a declared route (so a
 * nav click can never 404), and the workbench rail still exposes a
 * `/sessions` link.
 */

import { describe, expect, it } from 'vitest'
import { matchRoute } from './router.js'
import { NAV_ITEMS, ROUTES, SebasApp } from './app-shell.js'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

describe('primary navigation', () => {
  it('does not list /sessions; the other destinations remain', async () => {
    const el = document.createElement('sebas-app') as SebasApp
    document.body.appendChild(el)
    await el.updateComplete
    const hrefs = [...el.shadowRoot!.querySelectorAll('nav a.item')].map((a) =>
      a.getAttribute('href'),
    )
    expect(hrefs).not.toContain('/sessions')
    expect(hrefs).toContain('/')
    expect(hrefs).toContain('/settings')
    expect(hrefs).toContain('/gateway')
    expect(hrefs).toContain('/about')
    expect(hrefs).toContain('/admin/status')
    expect(hrefs).toHaveLength(NAV_ITEMS.length)
    el.remove()
  })

  it('still routes /sessions and session deep links (route kept, nav entry dropped)', () => {
    expect(matchRoute(ROUTES, '/sessions')?.id).toBe('sessions')
    expect(matchRoute(ROUTES, '/sessions/oc_abc%00')?.id).toBe('session-detail')
  })

  it('resolves every NAV_ITEMS href against a declared route (none can 404)', () => {
    expect(NAV_ITEMS.length).toBeGreaterThan(0)
    for (const item of NAV_ITEMS) {
      const match = matchRoute(ROUTES, item.href)
      expect(match, `nav href ${item.href} matched no route`).not.toBeNull()
    }
  })

  it('the workbench rail still exposes a /sessions link (demoted, not deleted)', () => {
    const here = dirname(new URL(import.meta.url).pathname)
    const src = readFileSync(join(here, 'views/dashboard.ts'), 'utf8')
    const railStart = src.indexOf('<aside class="rail">')
    expect(railStart).toBeGreaterThan(-1)
    const railEnd = src.indexOf('</aside>', railStart)
    const rail = src.slice(railStart, railEnd)
    expect(rail).toContain('href="/sessions"')
  })
})
