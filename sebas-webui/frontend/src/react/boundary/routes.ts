/**
 * React route boundary (2.4). Reuses the existing matcher and retirement
 * table instead of importing a router library; `:key` stays the RAW
 * percent-encoded segment so `%00` deep links survive unchanged.
 */

import { matchRoute, redirectFor, type RouteDef } from '../../router.js'

export const APP_ROUTES: readonly RouteDef[] = [
  { id: 'dashboard', pattern: '/' },
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session', pattern: '/sessions/:key' },
  { id: 'usage', pattern: '/usage' },
]

export type AppRoute =
  | { kind: 'dashboard' }
  | { kind: 'sessions' }
  | { kind: 'session'; key: string }
  | { kind: 'usage' }
  | { kind: 'unknown'; path: string }

export interface RouteResolution {
  route: AppRoute
  /** Set when the path is retired and should be canonicalised with pushState. */
  redirect: string | null
}

/**
 * Resolve a pathname. Retired paths resolve to their redirect target; any
 * other unmatched path is reported as `unknown` so the shell can render the
 * workbench fallback (the existing Lit shell's behaviour) instead of a 404.
 */
export function resolveRoute(pathname: string): RouteResolution {
  const redirect = redirectFor(pathname)
  if (redirect !== null) return { route: { kind: 'dashboard' }, redirect }
  const matched = matchRoute(APP_ROUTES, pathname)
  if (matched === null) return { route: { kind: 'unknown', path: pathname }, redirect: null }
  switch (matched.id) {
    case 'sessions':
      return { route: { kind: 'sessions' }, redirect: null }
    case 'session':
      return { route: { kind: 'session', key: matched.params.key ?? '' }, redirect: null }
    case 'usage':
      return { route: { kind: 'usage' }, redirect: null }
    case 'dashboard':
    default:
      return { route: { kind: 'dashboard' }, redirect: null }
  }
}

/** Raw encoded session key from a `/sessions/{key}` deep link, if any. */
export function sessionKeyFromPath(pathname: string): string | null {
  const resolved = resolveRoute(pathname)
  return resolved.route.kind === 'session' ? resolved.route.key : null
}
