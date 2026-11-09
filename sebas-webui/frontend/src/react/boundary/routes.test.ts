import { describe, expect, it } from 'vitest'
import { resolveRoute, sessionKeyFromPath } from './routes.js'

describe('resolveRoute', () => {
  it('maps the four live routes', () => {
    expect(resolveRoute('/')).toEqual({ route: { kind: 'dashboard' }, redirect: null })
    expect(resolveRoute('/sessions')).toEqual({ route: { kind: 'sessions' }, redirect: null })
    expect(resolveRoute('/usage')).toEqual({ route: { kind: 'usage' }, redirect: null })
  })

  it('keeps the session key raw percent-encoded (no decode)', () => {
    const resolved = resolveRoute('/sessions/oc_%00abc')
    expect(resolved.route).toEqual({ kind: 'session', key: 'oc_%00abc' })
    expect(resolved.redirect).toBeNull()
  })

  it('canonicalises retired paths instead of 404ing', () => {
    for (const retired of ['/settings', '/gateway', '/about', '/router']) {
      expect(resolveRoute(retired)).toEqual({
        route: { kind: 'dashboard' },
        redirect: '/',
      })
    }
  })

  it('reports unknown paths for the shell’s workbench fallback', () => {
    expect(resolveRoute('/admin/users')).toEqual({
      route: { kind: 'unknown', path: '/admin/users' },
      redirect: null,
    })
    expect(resolveRoute('/nope')).toEqual({
      route: { kind: 'unknown', path: '/nope' },
      redirect: null,
    })
  })

  it('extracts the raw session key only for session deep links', () => {
    expect(sessionKeyFromPath('/sessions/oc_%00abc')).toBe('oc_%00abc')
    expect(sessionKeyFromPath('/sessions')).toBeNull()
    expect(sessionKeyFromPath('/')).toBeNull()
    expect(sessionKeyFromPath('/sessions/a/b')).toBeNull()
  })
})
