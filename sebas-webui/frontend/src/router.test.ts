import { describe, expect, it } from 'vitest'
import { matchRoute } from './router.js'

const ROUTES = [
  { id: 'dashboard', pattern: '/' },
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session-detail', pattern: '/sessions/:key' },
  { id: 'settings', pattern: '/settings' },
  { id: 'gateway', pattern: '/gateway' },
  { id: 'about', pattern: '/about' },
  { id: 'admin-login', pattern: '/admin/login' },
  { id: 'admin', pattern: '/admin/:view' },
]

describe('matchRoute', () => {
  it('matches the dashboard root', () => {
    expect(matchRoute(ROUTES, '/')).toEqual({ id: 'dashboard', params: {} })
  })

  it('matches plain views', () => {
    for (const id of ['sessions', 'settings', 'gateway', 'about']) {
      expect(matchRoute(ROUTES, `/${id}`)?.id).toBe(id)
    }
  })

  it('captures the session key param RAW (still percent-encoded)', () => {
    // Regression: the key embeds a NUL (%00 encoded). Params must stay
    // encoded so fetch('/api/sessions/' + key) keeps working; decoding here
    // produced a literal NUL that the browser strips from URLs → 400.
    const m = matchRoute(ROUTES, '/sessions/oc_abc%00')
    expect(m?.id).toBe('session-detail')
    expect(m?.params['key']).toBe('oc_abc%00')
  })

  it('matches admin login before the admin :view wildcard', () => {
    expect(matchRoute(ROUTES, '/admin/login')?.id).toBe('admin-login')
    expect(matchRoute(ROUTES, '/admin/status')?.id).toBe('admin')
    expect(matchRoute(ROUTES, '/admin/status')?.params['view']).toBe('status')
  })

  it('returns null for unmatched paths', () => {
    expect(matchRoute(ROUTES, '/nope')).toBeNull()
    expect(matchRoute(ROUTES, '/sessions/extra/deep')).toBeNull()
  })
})
