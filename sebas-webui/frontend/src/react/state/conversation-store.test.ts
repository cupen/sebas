import { describe, expect, it, vi } from 'vitest'
import type { ConversationEntryView, SessionDetail } from '../../api/client.js'
import { ConversationStore, type ConversationApi } from './conversation-store.js'

function entry(position: number, content = `e${position}`): ConversationEntryView {
  return {
    position,
    kind: 'content',
    element_type: 'markdown',
    content,
    created_at_unix: 1_700_000_000 + position,
  }
}

function detail(key: string, positions: number[]): SessionDetail {
  return {
    encoded_key: key,
    entries: positions.map((position) => entry(position)),
  } as unknown as SessionDetail
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

describe('ConversationStore', () => {
  it('opens a session, sorts entries and records the cursor', async () => {
    const api: ConversationApi = { session: vi.fn(async () => detail('oc_1', [3, 1, 2])) }
    const store = new ConversationStore(api)
    const state = await store.open('oc_1')
    expect(state.sessionKey).toBe('oc_1')
    expect(state.entries.map((e) => e.position)).toEqual([1, 2, 3])
    expect(state.cursor).toBe(3)
    expect(state.loading).toBe(false)
  })

  it('clears the previous transcript before the new detail resolves', () => {
    const pending = deferred<SessionDetail>()
    const store = new ConversationStore({ session: () => pending.promise })
    void store.open('oc_1')
    expect(store.getState()).toMatchObject({
      sessionKey: 'oc_1',
      detail: null,
      entries: [],
      loading: true,
    })
  })

  it('ignores a late response from the previously opened session', async () => {
    const first = deferred<SessionDetail>()
    const second = deferred<SessionDetail>()
    const session = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise)
    const store = new ConversationStore({ session })

    const a = store.open('old')
    const b = store.open('new')
    second.resolve(detail('new', [1]))
    await b
    first.resolve(detail('old', [1, 2, 3]))
    await a
    expect(store.getState().sessionKey).toBe('new')
    expect(store.getState().entries.map((e) => e.position)).toEqual([1])
  })

  it('merges appended frames by position and advances the cursor', async () => {
    const store = new ConversationStore({ session: async () => detail('oc_1', [1, 2]) })
    await store.open('oc_1')

    store.append('oc_1', [entry(2, 'dup'), entry(3), entry(4)])
    expect(store.getState().entries.map((e) => e.position)).toEqual([1, 2, 3, 4])
    expect(store.getState().cursor).toBe(4)

    const snapshot = store.getState()
    store.append('oc_1', [entry(4, 'dup')])
    expect(store.getState()).toBe(snapshot)

    store.append('other', [entry(9)])
    expect(store.getState().entries.map((e) => e.position)).toEqual([1, 2, 3, 4])
  })

  it('resetCursor re-accepts a replayed frame', async () => {
    const store = new ConversationStore({ session: async () => detail('oc_1', [1, 2]) })
    await store.open('oc_1')
    store.append('oc_1', [entry(3)])
    expect(store.getState().cursor).toBe(3)
    store.resetCursor()
    store.append('oc_1', [entry(3, 'replayed')])
    expect(store.getState().entries.find((e) => e.position === 3)?.content).toBe('replayed')
  })

  it('refreshes the current key and reports load errors', async () => {
    const session = vi
      .fn()
      .mockResolvedValueOnce(detail('oc_1', [1]))
      .mockRejectedValueOnce(new Error('boom'))
    const store = new ConversationStore({ session })
    await store.open('oc_1')
    const state = await store.refresh()
    expect(session).toHaveBeenCalledTimes(2)
    expect(session).toHaveBeenLastCalledWith('oc_1', undefined)
    expect(state).toMatchObject({ sessionKey: 'oc_1', loading: false, error: 'boom' })
    expect(state.entries).toHaveLength(1)
  })

  it('applies a phase frame without refetching the transcript', async () => {
    const session = vi.fn(async () => detail('oc_1', [1]))
    const store = new ConversationStore({ session })
    await store.open('oc_1')
    store.applyPhase('oc_1', {
      status_slug: 'done',
      turn_engaged: false,
      msg_count: 9,
      pending: [],
    })
    expect(session).toHaveBeenCalledTimes(1)
    expect(store.getState().detail).toMatchObject({
      status_slug: 'done',
      turn_engaged: false,
      msg_count: 9,
    })
    const snapshot = store.getState()
    store.applyPhase('other', {
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 10,
      pending: [],
    })
    expect(store.getState()).toBe(snapshot)
  })

  it('refresh is a no-op without a focused session', async () => {
    const session = vi.fn()
    const store = new ConversationStore({ session })
    const state = await store.refresh()
    expect(state.sessionKey).toBeNull()
    expect(session).not.toHaveBeenCalled()
  })
})
