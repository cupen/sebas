import { useEffect, useState } from 'react'
import { navigate } from '../../router.js'
import { resolveRoute, type RouteResolution } from './routes.js'

export interface RouterState extends RouteResolution {
  pathname: string
}

function readLocation(): RouterState {
  const pathname = window.location.pathname
  return { pathname, ...resolveRoute(pathname) }
}

/**
 * History-API route subscription. Retired paths are canonicalised in place
 * (`replaceState`) so a bookmark to `/settings` lands on `/` without adding a
 * history entry.
 */
export function useRoute(): RouterState {
  const [state, setState] = useState<RouterState>(readLocation)

  useEffect(() => {
    const update = () => {
      const next = readLocation()
      if (next.redirect !== null) {
        window.history.replaceState({}, '', next.redirect)
        setState({ ...next, pathname: next.redirect, route: { kind: 'dashboard' } })
        return
      }
      setState(next)
    }
    update()
    window.addEventListener('popstate', update)
    return () => window.removeEventListener('popstate', update)
  }, [])

  return state
}

export { navigate }
