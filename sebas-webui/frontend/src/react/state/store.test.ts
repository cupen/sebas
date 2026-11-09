import { describe, expect, it, vi } from 'vitest'
import { createStore, Generation } from './store.js'

describe('createStore', () => {
  it('notifies subscribers and supports functional updates', () => {
    const store = createStore({ count: 0 })
    const listener = vi.fn()
    store.subscribe(listener)
    store.setState((prev) => ({ count: prev.count + 1 }))
    expect(store.getState().count).toBe(1)
    expect(listener).toHaveBeenCalledWith({ count: 1 })
  })

  it('unsubscribe stops delivery', () => {
    const store = createStore(1)
    const listener = vi.fn()
    const off = store.subscribe(listener)
    off()
    store.setState(2)
    expect(listener).not.toHaveBeenCalled()
  })

  it('skips notification for the same snapshot', () => {
    const store = createStore({ value: 1 })
    const listener = vi.fn()
    store.subscribe(listener)
    const same = store.getState()
    store.setState(same)
    expect(listener).not.toHaveBeenCalled()
  })
})

describe('Generation', () => {
  it('only the newest token is current', () => {
    const generation = new Generation()
    const first = generation.next()
    expect(generation.isCurrent(first)).toBe(true)
    const second = generation.next()
    expect(generation.isCurrent(first)).toBe(false)
    expect(generation.isCurrent(second)).toBe(true)
  })
})
