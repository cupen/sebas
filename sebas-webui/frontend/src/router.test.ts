import { describe, expect, it } from 'vitest'
import { matchRoute } from './router.js'

// IA v2 production routes (app-shell.ts ROUTES): the workbench, the
// sessions table and session deep links (the deep link renders the same
// workbench focused — no separate detail view). /settings /router /about
// redirect to / and /admin/* is deleted — no fixture entries for them.
const ROUTES = [
  { id: 'dashboard', pattern: '/' },
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session-deep-link', pattern: '/sessions/:key' },
]

describe('matchRoute', () => {
  it('matches the dashboard root', () => {
    expect(matchRoute(ROUTES, '/')).toEqual({ id: 'dashboard', params: {} })
  })

  it('matches the sessions list', () => {
    expect(matchRoute(ROUTES, '/sessions')?.id).toBe('sessions')
  })

  it('captures the session key param RAW (still percent-encoded)', () => {
    // Regression: the key embeds a NUL (%00 encoded). Params must stay
    // encoded so fetch('/api/sessions/' + key) keeps working; decoding here
    // produced a literal NUL that the browser strips from URLs → 400.
    const m = matchRoute(ROUTES, '/sessions/oc_abc%00')
    expect(m?.id).toBe('session-deep-link')
    expect(m?.params['key']).toBe('oc_abc%00')
  })

  it('returns null for unmatched (incl. retired and deleted) paths', () => {
    expect(matchRoute(ROUTES, '/nope')).toBeNull()
    expect(matchRoute(ROUTES, '/sessions/extra/deep')).toBeNull()
    expect(matchRoute(ROUTES, '/settings')).toBeNull()
    expect(matchRoute(ROUTES, '/admin/status')).toBeNull()
  })
})

// fix-webui-qa-round9 4.1（agent-workbench）：/router 并入工作台后，旧地址
// 必须归一回 canonical `/`，而不是 404 或留在原地。
describe('redirectFor', () => {
  it('redirects the retired /router path to the workbench root', async () => {
    const { redirectFor, RETIRED_REDIRECTS } = await import('./router.js')
    expect(redirectFor('/router')).toBe('/')
    expect(RETIRED_REDIRECTS['/router']).toBe('/')
  })

  it('keeps the other retired redirects intact', async () => {
    const { redirectFor } = await import('./router.js')
    expect(redirectFor('/settings')).toBe('/')
    expect(redirectFor('/gateway')).toBe('/')
    expect(redirectFor('/about')).toBe('/')
    expect(redirectFor('/nope')).toBeNull()
  })
})
