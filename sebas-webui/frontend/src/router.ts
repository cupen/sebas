/**
 * Minimal history-API router. Seven route patterns is small enough that a
 * tiny, fully-tested matcher beats importing a router library.
 *
 * Routes are declared as `{ pattern, render }`; `pattern` may contain
 * `:param` segments captured into `params`. Deep links work because the
 * backend serves the SPA entry for every page path (SPA fallback).
 *
 * `:param` values are the RAW (still percent-encoded) path segments. The
 * session key embeds a NUL byte (`%00` when encoded); decoding it here would
 * produce a literal NUL that breaks every subsequent `fetch('/api/…/' + key)`
 * (the browser strips control characters from URLs → server 400). Consumers
 * decode explicitly when they need the plain text.
 */

export interface RouteMatch {
  id: string
  params: Record<string, string>
}

export interface RouteDef {
  id: string
  pattern: string
}

/** Match a concrete path against the declared route patterns. */
export function matchRoute(routes: RouteDef[], path: string): RouteMatch | null {
  for (const route of routes) {
    const patternSegments = route.pattern.split('/').filter(Boolean)
    const pathSegments = path.split('/').filter(Boolean)
    if (patternSegments.length !== pathSegments.length) continue
    const params: Record<string, string> = {}
    let ok = true
    for (let i = 0; i < patternSegments.length; i++) {
      const pat = patternSegments[i]!
      const seg = pathSegments[i]!
      if (pat.startsWith(':')) {
        params[pat.slice(1)] = seg
      } else if (pat !== seg) {
        ok = false
        break
      }
    }
    if (ok) return { id: route.id, params }
  }
  return null
}

/** Navigate via the History API and notify listeners. */
export function navigate(path: string): void {
  history.pushState({}, '', path)
  window.dispatchEvent(new PopStateEvent('popstate'))
}
