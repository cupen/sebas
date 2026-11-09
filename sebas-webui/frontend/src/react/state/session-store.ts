/**
 * Project/session store (2.6): summary counters, the session list, and the
 * project registry. One generation per refresh so a slow snapshot cannot
 * overwrite a newer one; live phase frames patch the same state in place.
 */

import type {
  Project,
  SessionList,
  SessionRow,
  SessionSummary,
  Summary,
} from '../../api/client.js'
import type { SessionPhaseFrame } from '../../api/ws.js'
import { createStore, Generation, type Store } from './store.js'

export interface SessionState {
  loading: boolean
  error: string | null
  summary: Summary | null
  list: SessionList | null
  projects: Project[]
  loadedAt: number | null
}

export interface SessionApi {
  summary: () => Promise<Summary>
  sessions: () => Promise<SessionList>
  projects: () => Promise<{ projects: Project[] }>
}

const EMPTY: SessionState = {
  loading: false,
  error: null,
  summary: null,
  list: null,
  projects: [],
  loadedAt: null,
}

export class SessionStore {
  readonly store: Store<SessionState>
  private readonly generation = new Generation()
  private readonly api: SessionApi
  /** Frames that arrived before the first list snapshot (cold-load race). */
  private readonly pendingPhases = new Map<string, SessionPhaseFrame>()
  private readonly waitingKeys = new Set<string>()

  constructor(api: SessionApi) {
    this.api = api
    this.store = createStore(EMPTY)
  }

  getState(): SessionState {
    return this.store.getState()
  }

  subscribe(listener: (state: SessionState) => void): () => void {
    return this.store.subscribe(listener)
  }

  /** Load the three independent projections; only the newest batch commits. */
  async refresh(): Promise<SessionState> {
    const token = this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: true, error: null }))
    try {
      const [summary, list, projects] = await Promise.all([
        this.api.summary(),
        this.api.sessions(),
        this.api.projects(),
      ])
      if (!this.generation.isCurrent(token)) return this.getState()
      this.store.setState({
        loading: false,
        error: null,
        summary,
        list: { ...list, recent_sessions: this.overlayRows(list.recent_sessions) },
        projects: projects.projects,
        loadedAt: Date.now(),
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

  /** Invalidate in-flight loads; the next `refresh()` starts a new generation. */
  invalidate(): void {
    this.generation.next()
  }

  /**
   * Re-apply frames that arrived before the list was available, then drop
   * them. A later phase frame always supersedes an optimistic waiting mark.
   */
  private overlayRows(rows: SessionRow[]): SessionRow[] {
    const next = rows.map((row) => {
      const frame = this.pendingPhases.get(row.encoded_key)
      let patched = frame ? patchRow(row, frame) : row
      if (this.waitingKeys.has(row.encoded_key)) {
        patched = {
          ...patched,
          status: 'waiting',
          status_label: 'waiting',
          status_slug: 'waiting',
        }
      }
      return patched
    })
    this.pendingPhases.clear()
    this.waitingKeys.clear()
    return next
  }

  /**
   * Patch from a live session phase frame. The row shape stays whatever the
   * server sent; we only merge the frame-owned facts that the rail uses.
   */
  applyPhaseFrame(
    frame: SessionPhaseFrame & { type: 'session.created' | 'session.updated'; session_id: string },
  ): void {
    const state = this.getState()
    const list = state.list
    if (list === null) {
      this.pendingPhases.set(frame.session_id, frame)
      return
    }
    // Live phase frames are newer than any snapshot fetch already in flight:
    // invalidate that response so it cannot overwrite the frame status later.
    this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: false }))
    this.waitingKeys.delete(frame.session_id)
    const rows = list.recent_sessions.map((row) =>
      row.encoded_key === frame.session_id ? patchRow(row, frame) : row,
    )
    const present = rows.some((row) => row.encoded_key === frame.session_id)
    const nextRows = present
      ? rows
      : [
          {
            encoded_key: frame.session_id,
            thread_id: null,
            session_id: null,
            session_id_short: null,
            status: frame.status_slug,
            status_label: frame.status_slug,
            status_slug: frame.status_slug as SessionRow['status_slug'],
            status_glyph: '',
            last_active: '',
            last_active_unix: 0,
            is_active: frame.status_slug !== 'dormant',
            project_id: null,
            prompt_preview: frame.prompt_preview ?? null,
            label: frame.label,
            msg_count: frame.msg_count,
            current_model: null,
            available_models: null,
            agent_kind: null,
            pending_count: frame.pending.length,
            turn_engaged: frame.turn_engaged,
            desired_mode: 'ask',
          } satisfies SessionRow,
          ...rows,
        ]
    this.store.setState((prev) => ({ ...prev, list: { ...list, recent_sessions: nextRows } }))
  }

  /** Optimistic `waiting` projection when a gate parks outside a phase frame. */
  markWaiting(sessionKey: string): void {
    const list = this.getState().list
    if (list === null) {
      this.waitingKeys.add(sessionKey)
      return
    }
    this.generation.next()
    this.store.setState((prev) => ({
      ...prev,
      loading: false,
      list: {
        ...list,
        recent_sessions: list.recent_sessions.map((row) =>
          row.encoded_key === sessionKey
            ? { ...row, status: 'waiting', status_label: 'waiting', status_slug: 'waiting' }
            : row,
        ),
      },
    }))
  }

  applyRemoved(sessionKey: string): void {
    const list = this.getState().list
    if (list === null) return
    this.store.setState((prev) => ({
      ...prev,
      list: {
        ...list,
        recent_sessions: list.recent_sessions.filter((row) => row.encoded_key !== sessionKey),
        total_sessions: Math.max(0, list.total_sessions - 1),
      },
    }))
  }

  /** Focused session from a deep link, if the summary already knows it. */
  focusedSession(): SessionSummary | null {
    return this.getState().summary?.active_session ?? null
  }
}

function patchRow(row: SessionRow, frame: SessionPhaseFrame): SessionRow {
  return {
    ...row,
    status_slug: frame.status_slug as SessionRow['status_slug'],
    status: frame.status_slug,
    status_label: frame.status_slug,
    msg_count: frame.msg_count,
    label: frame.label,
    prompt_preview: frame.prompt_preview ?? row.prompt_preview,
    pending_count: frame.pending.length,
    turn_engaged: frame.turn_engaged,
    usage: frame.usage ?? row.usage,
  }
}
