import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createStore } from './store.js'
import { useStore } from './use-store.js'

afterEach(cleanup)

describe('useStore', () => {
  it('renders the selected slice and reacts to updates', () => {
    const store = createStore({ a: 1, b: 1 })
    const { result } = renderHook(() => useStore(store, (state) => state.a))
    expect(result.current).toBe(1)
    act(() => store.setState({ a: 2, b: 1 }))
    expect(result.current).toBe(2)
  })

  it('bails out when an unrelated slice changes', () => {
    const store = createStore({ a: 1, b: 1 })
    const renders = vi.fn()
    renderHook(() => {
      const value = useStore(store, (state) => state.a)
      renders(value)
      return value
    })
    const before = renders.mock.calls.length
    act(() => store.setState({ a: 1, b: 2 }))
    expect(renders.mock.calls.length).toBe(before)
  })

  it('re-renders every consumer of a changed slice', () => {
    const store = createStore({ count: 0 })
    const first = renderHook(() => useStore(store, (state) => state.count))
    const second = renderHook(() => useStore(store, (state) => state.count))
    act(() => store.setState({ count: 5 }))
    expect(first.result.current).toBe(5)
    expect(second.result.current).toBe(5)
  })

  it('unsubscribes on unmount', () => {
    const store = createStore({ count: 0 })
    const listeners = new Set<(state: { count: number }) => void>()
    const subscribe = vi.fn((listener: (state: { count: number }) => void) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    })
    const wrapped = { ...store, subscribe }
    const { unmount } = renderHook(() => useStore(wrapped, (state) => state.count))
    expect(listeners.size).toBe(1)
    unmount()
    expect(listeners.size).toBe(0)
    expect(subscribe).toHaveBeenCalled()
  })
})
