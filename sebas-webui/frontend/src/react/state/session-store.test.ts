import { describe, expect, it, vi } from 'vitest'
import type {
  Project,
  SessionList,
  SessionRow,
  SessionSummary,
  Summary,
} from '../../api/client.js'
import type { SessionPhaseFrame } from '../../api/ws.js'
import { SessionStore, type SessionApi } from './session-store.js'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

function row(patch: Partial<SessionRow> = {}): SessionRow {
  return {
    encoded_key: 'oc_1',
    thread_id: null,
    session_id: 's1',
    session_id_short: 's1',
    status: 'working',
    status_label: 'working',
    status_slug: 'working',
    status_glyph: '●',
    last_active: 'now',
    last_active_unix: 1,
    is_active: true,
    project_id: null,
    prompt_preview: 'hello',
    label: null,
    msg_count: 3,
    current_model: null,
    available_models: null,
    agent_kind: null,
    pending_count: 0,
    turn_engaged: true,
    desired_mode: 'ask',
    ...patch,
  } as unknown as SessionRow
}

function summary(patch: Partial<Summary> = {}): Summary {
  return {
    active_count: 1,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 1,
    uptime: '1m',
    recent_sessions: [row()],
    active_session: null,
    active_session_key: 'oc_1',
    reachability: {} as Summary['reachability'],
    ...patch,
  }
}

function list(rows: SessionRow[], patch: Partial<SessionList> = {}): SessionList {
  return {
    recent_sessions: rows,
    active_count: 1,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: rows.length,
    active_session_key: rows[0]?.encoded_key ?? null,
    ...patch,
  }
}

function api(overrides: Partial<SessionApi> = {}): SessionApi {
  return {
    summary: vi.fn(async () => summary()),
    sessions: vi.fn(async () => list([row()])),
    projects: vi.fn(async () => ({ projects: [] as Project[] })),
    ...overrides,
  }
}

type PhasePatch = Partial<
  SessionPhaseFrame & { type: 'session.created' | 'session.updated'; session_id: string }
>

function frame(patch: PhasePatch = {}): SessionPhaseFrame & {
  type: 'session.created' | 'session.updated'
  session_id: string
} {
  return {
    type: 'session.updated',
    session_id: 'oc_1',
    status_slug: 'done',
    turn_engaged: false,
    msg_count: 7,
    pending: [],
    label: 'renamed',
    prompt_preview: 'preview',
    ...patch,
  }
}

describe('SessionStore', () => {
  it('loads summary, sessions and projects in one commit', async () => {
    const projects: Project[] = [
      { id: 'proj-1', path: '/p', name: 'p', added_at: 1 },
    ]
    const store = new SessionStore(
      api({
        summary: async () => summary({ total_sessions: 4 }),
        sessions: async () => list([row({ encoded_key: 'oc_1' }), row({ encoded_key: 'oc_2' })]),
        projects: async () => ({ projects }),
      }),
    )
    const state = await store.refresh()
    expect(state.loading).toBe(false)
    expect(state.summary?.total_sessions).toBe(4)
    expect(state.list?.recent_sessions.map((r) => r.encoded_key)).toEqual(['oc_1', 'oc_2'])
    expect(state.projects).toEqual(projects)
    expect(state.loadedAt).not.toBeNull()
  })

  it('reports errors and clears loading', async () => {
    const store = new SessionStore(
      api({
        sessions: async () => {
          throw new Error('sessions unavailable')
        },
      }),
    )
    const state = await store.refresh()
    expect(state).toMatchObject({ loading: false, error: 'sessions unavailable' })
    expect(state.list).toBeNull()
  })

  it('only the newest refresh commits', async () => {
    const slowSummary = deferred<Summary>()
    const slowSessions = deferred<SessionList>()
    const slowProjects = deferred<{ projects: Project[] }>()
    const fastSummary = deferred<Summary>()
    const fastSessions = deferred<SessionList>()
    const fastProjects = deferred<{ projects: Project[] }>()
    const calls = [
      [slowSummary, slowSessions, slowProjects],
      [fastSummary, fastSessions, fastProjects],
    ] as const
    let round = 0
    const store = new SessionStore({
      summary: () => calls[round]![0].promise,
      sessions: () => calls[round]![1].promise,
      projects: () => calls[round]![2].promise,
      ...{},
    })
    const slow = store.refresh()
    round = 1
    const fast = store.refresh()
    fastSummary.resolve(summary({ total_sessions: 2 }))
    fastSessions.resolve(list([row({ encoded_key: 'new' })], { total_sessions: 1 }))
    fastProjects.resolve({ projects: [] })
    await fast
    slowSummary.resolve(summary({ total_sessions: 99 }))
    slowSessions.resolve(list([row({ encoded_key: 'old' })], { total_sessions: 99 }))
    slowProjects.resolve({ projects: [] })
    await slow
    expect(store.getState().summary?.total_sessions).toBe(2)
    expect(store.getState().list?.recent_sessions[0]?.encoded_key).toBe('new')
  })

  it('patches a row from a live phase frame and inserts unknown sessions', async () => {
    const store = new SessionStore(api())
    await store.refresh()

    store.applyPhaseFrame(frame())
    const patched = store.getState().list!.recent_sessions[0]!
    expect(patched).toMatchObject({
      status_slug: 'done',
      msg_count: 7,
      label: 'renamed',
      pending_count: 0,
      turn_engaged: false,
    })

    store.applyPhaseFrame(frame({ session_id: 'oc_new', type: 'session.created' }))
    const rows = store.getState().list!.recent_sessions
    expect(rows).toHaveLength(2)
    expect(rows[0]!.encoded_key).toBe('oc_new')
  })

  it('removes a session and floors the total at zero', async () => {
    const store = new SessionStore(
      api({ sessions: async () => list([row()], { total_sessions: 0 }) }),
    )
    await store.refresh()
    store.applyRemoved('oc_1')
    expect(store.getState().list).toMatchObject({
      recent_sessions: [],
      total_sessions: 0,
    })
  })

  it('exposes the focused session from the summary', async () => {
    const active = { encoded_key: 'oc_1' } as unknown as SessionSummary
    const store = new SessionStore(api({ summary: async () => summary({ active_session: active }) }))
    expect(store.focusedSession()).toBeNull()
    await store.refresh()
    expect(store.focusedSession()).toBe(active)
  })
})
