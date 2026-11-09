import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { navigate, useRoute } from './use-route.js'

afterEach(() => {
  cleanup()
  window.history.replaceState({}, '', '/')
})

function setPath(path: string): void {
  window.history.replaceState({}, '', path)
}

describe('useRoute', () => {
  it('follows consecutive session deep links without decoding the key', () => {
    setPath('/sessions/oc_%00a')
    const { result } = renderHook(() => useRoute())
    expect(result.current.pathname).toBe('/sessions/oc_%00a')
    expect(result.current.route).toEqual({ kind: 'session', key: 'oc_%00a' })

    act(() => navigate('/sessions/oc_%00b'))
    expect(result.current.pathname).toBe('/sessions/oc_%00b')
    expect(result.current.route).toEqual({ kind: 'session', key: 'oc_%00b' })

    act(() => navigate('/usage'))
    expect(result.current.route).toEqual({ kind: 'usage' })

    act(() => navigate('/sessions/oc_%00c'))
    expect(result.current.route).toEqual({ kind: 'session', key: 'oc_%00c' })
  })

  it('canonicalises a retired path with replaceState, without a route 404', () => {
    setPath('/settings')
    const { result } = renderHook(() => useRoute())
    expect(result.current.pathname).toBe('/')
    expect(result.current.route).toEqual({ kind: 'dashboard' })
    expect(window.location.pathname).toBe('/')
  })

  it('reports unknown paths with the workbench fallback intent', () => {
    setPath('/admin/users')
    const { result } = renderHook(() => useRoute())
    expect(result.current.route).toEqual({ kind: 'unknown', path: '/admin/users' })
  })

  it('removes its popstate listener on unmount', () => {
    setPath('/')
    const { result, unmount } = renderHook(() => useRoute())
    unmount()
    act(() => navigate('/usage'))
    expect(result.current.pathname).toBe('/')
  })
})
