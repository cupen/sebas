/**
 * Conversation store (2.6). Switching sessions bumps a generation, so a
 * late detail response from the previous key can never leak into the new
 * transcript; `turn.append` frames merge by position with duplicate-proof
 * ordering.
 */

import type { ConversationEntryView, SessionDetail } from '../../api/client.js'
import { createStore, Generation, type Store } from './store.js'

export interface ConversationState {
  sessionKey: string | null
  detail: SessionDetail | null
  entries: ConversationEntryView[]
  loading: boolean
  error: string | null
  /** Highest position already merged (cursor for `entries_after`). */
  cursor: number
}

export interface ConversationApi {
  session: (encodedKey: string, entriesAfter?: number) => Promise<SessionDetail>
}

const EMPTY: ConversationState = {
  sessionKey: null,
  detail: null,
  entries: [],
  loading: false,
  error: null,
  cursor: -1,
}

export class ConversationStore {
  readonly store: Store<ConversationState>
  private readonly generation = new Generation()
  private readonly api: ConversationApi

  constructor(api: ConversationApi) {
    this.api = api
    this.store = createStore(EMPTY)
  }

  getState(): ConversationState {
    return this.store.getState()
  }

  subscribe(listener: (state: ConversationState) => void): () => void {
    return this.store.subscribe(listener)
  }

  /** Focus a session: clear the old transcript immediately, then load it. */
  async open(sessionKey: string): Promise<ConversationState> {
    const token = this.generation.next()
    this.store.setState({
      sessionKey,
      detail: null,
      entries: [],
      loading: true,
      error: null,
      cursor: -1,
    })
    return this.load(sessionKey, undefined, token)
  }

  /** Explicit refresh of the current session (generation invalidates old I/O). */
  async refresh(): Promise<ConversationState> {
    const key = this.getState().sessionKey
    if (key === null) return this.getState()
    const token = this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: true, error: null }))
    return this.load(key, undefined, token)
  }

  /** Invalidate in-flight responses; the next open/refresh wins. */
  invalidate(): void {
    this.generation.next()
  }

  /**
   * Merge one `turn.append` frame. Entries at or below the cursor are
   * duplicates (late or replayed); a frame for another session is ignored.
   */
  append(sessionKey: string, entries: readonly ConversationEntryView[]): void {
    const state = this.getState()
    if (state.sessionKey !== sessionKey || entries.length === 0) return
    const fresh = entries.filter((entry) => entry.position > state.cursor)
    if (fresh.length === 0) return
    const merged = mergeEntries(state.entries, fresh)
    this.store.setState((prev) => ({
      ...prev,
      entries: merged,
      cursor: Math.max(prev.cursor, ...fresh.map((entry) => entry.position)),
    }))
  }

  /** A resync signal drops the local cursor; the next load is authoritative. */
  resetCursor(): void {
    this.store.setState((prev) => ({ ...prev, cursor: -1 }))
  }

  /**
   * Apply a `session.updated` phase frame to the focused detail without a
   * fetch (per-frame full reload is forbidden by the performance budget).
   */
  applyPhase(
    sessionKey: string,
    frame: {
      status_slug: string
      turn_engaged: boolean
      msg_count: number
      pending: SessionDetail['pending']
      usage?: SessionDetail['usage']
    },
  ): void {
    const state = this.getState()
    if (state.sessionKey !== sessionKey || state.detail === null) return
    this.store.setState((prev) => ({
      ...prev,
      detail:
        prev.detail === null
          ? null
          : {
              ...prev.detail,
              status_slug: frame.status_slug as SessionDetail['status_slug'],
              status_label: frame.status_slug,
              turn_engaged: frame.turn_engaged,
              msg_count: frame.msg_count,
              pending: frame.pending,
              ...(frame.usage === undefined ? {} : { usage: frame.usage }),
            },
    }))
  }

  private async load(
    sessionKey: string,
    entriesAfter: number | undefined,
    token: number,
  ): Promise<ConversationState> {
    try {
      const detail = await this.api.session(sessionKey, entriesAfter)
      if (!this.generation.isCurrent(token)) return this.getState()
      this.store.setState({
        sessionKey,
        detail,
        entries: sortEntries(detail.entries),
        loading: false,
        error: null,
        cursor: detail.entries.reduce((max, entry) => Math.max(max, entry.position), -1),
      })
      return this.getState()
    } catch (error) {
      if (!this.generation.isCurrent(token)) return this.getState()
      this.store.setState((prev) => ({
        ...prev,
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      }))
      return this.getState()
    }
  }
}

function sortEntries(entries: readonly ConversationEntryView[]): ConversationEntryView[] {
  return [...entries].sort((a, b) => a.position - b.position)
}

function mergeEntries(
  current: readonly ConversationEntryView[],
  incoming: readonly ConversationEntryView[],
): ConversationEntryView[] {
  const byPosition = new Map<number, ConversationEntryView>()
  for (const entry of current) byPosition.set(entry.position, entry)
  for (const entry of incoming) byPosition.set(entry.position, entry)
  return [...byPosition.values()].sort((a, b) => a.position - b.position)
}
