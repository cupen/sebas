import { useSyncExternalStore } from 'react'
import type { Store } from './store.js'

/**
 * Subscribe a component to a selector slice. The selector must return a
 * stable-comparable value (primitive or a reference that only changes when
 * the slice really changes): React bails out with Object.is, so unrelated
 * state updates do not re-render the consumer.
 */
export function useStore<T, S>(store: Store<T>, selector: (state: T) => S): S {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.getState()),
    () => selector(store.getState()),
  )
}
