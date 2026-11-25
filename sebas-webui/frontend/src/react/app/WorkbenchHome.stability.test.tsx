import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import type { SessionList, Summary } from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { WorkbenchHome } from './WorkbenchHome.js'

afterEach(cleanup)

const summary: Summary = {
  active_count: 0,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: 0,
  uptime: '0h',
  recent_sessions: [],
  active_session: null,
  active_session_key: null,
  reachability: { ok: true },
}

const list: SessionList = {
  recent_sessions: [],
  active_count: 0,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: 0,
  active_session_key: null,
}

function coldStore(): SessionStore {
  return new SessionStore({
    summary: async () => summary,
    sessions: async () => list,
    projects: async () => ({ projects: [] }),
  })
}

describe('WorkbenchHome snapshot stability (P0-1)', () => {
  it('renders without looping while the session list has never loaded', () => {
    // 冷启动：list 仍是 null（未 refresh）。`?? []` 的内联字面量曾让
    // useSyncExternalStore 每次拿到新快照 → Maximum update depth → 白屏。
    const sessions = coldStore()
    expect(sessions.getState().list).toBeNull()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId={null}
        canCreate={false}
        onRequestCreate={() => {}}
        onFocus={() => {}}
      />,
    )
    expect(screen.getAllByTestId('workbench-home')).toHaveLength(1)
  })

  it('does not re-render itself into a loop on repeated store notifications', () => {
    const sessions = coldStore()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId={null}
        canCreate={false}
        onRequestCreate={() => {}}
        onFocus={() => {}}
      />,
    )
    act(() => {
      for (let i = 0; i < 3; i += 1) {
        sessions.store.setState((state) => ({ ...state }))
      }
    })
    // 快照恒等 → 三次通知都是 no-op 重渲，不会累计出第二棵 stage。
    expect(screen.getAllByTestId('workbench-home')).toHaveLength(1)
  })
})
