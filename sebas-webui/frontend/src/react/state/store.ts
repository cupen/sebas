/**
 * Minimal framework-agnostic store (2.6): one immutable snapshot, explicit
 * subscribers, and a selector hook for React. No global state library — the
 * shell owns the instances and passes them down.
 */

export interface Store<T> {
  getState(): T
  setState(update: T | ((prev: T) => T)): void
  subscribe(listener: (state: T) => void): () => void
}

export function createStore<T>(initial: T): Store<T> {
  let state = initial
  const listeners = new Set<(state: T) => void>()
  return {
    getState: () => state,
    setState(update) {
      const next = typeof update === 'function' ? (update as (prev: T) => T)(state) : update
      if (Object.is(next, state)) return
      state = next
      for (const listener of listeners) listener(state)
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

/**
 * Monotonic generation token. Every async load captures the current value;
 * only a response whose generation is still current may write state. Opening
 * another session (or refetching) bumps it, so a late response from the
 * previous request can never overwrite the newer view.
 */
export class Generation {
  private value = 0

  /** Invalidate every in-flight response and return the new token. */
  next(): number {
    this.value += 1
    return this.value
  }

  get current(): number {
    return this.value
  }

  isCurrent(token: number): boolean {
    return token === this.value
  }
}
