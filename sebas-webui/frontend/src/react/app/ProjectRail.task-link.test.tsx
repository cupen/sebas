import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  api,
  type Project,
  type SessionList,
  type SessionRow,
  type Summary,
} from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { ProjectRail } from './ProjectRail.js'

afterEach(cleanup)

const projects: Project[] = [{ id: 'p1', name: 'alpha', path: '/work/alpha', added_at: 1 }]

const linkedRow: SessionRow = {
  encoded_key: 'oc_linked',
  thread_id: null,
  session_id: 's1',
  session_id_short: 's1',
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  last_active: '',
  last_active_unix: 1_700_000_000,
  is_active: false,
  project_id: 'p1',
  prompt_preview: '推进任务',
  label: null,
  msg_count: 1,
  current_model: null,
  available_models: null,
  agent_kind: 'claude',
  pending_count: 0,
  turn_engaged: false,
  desired_mode: 'ask',
  task_id: 'bd-42',
}

async function store(): Promise<SessionStore> {
  const summary: Summary = {
    active_count: 0,
    dormant_count: 1,
    spawning_count: 0,
    total_sessions: 1,
    uptime: '1h',
    recent_sessions: [linkedRow],
    active_session: null,
    active_session_key: null,
    reachability: { ok: true },
  }
  const list: SessionList = {
    recent_sessions: [linkedRow],
    active_count: 0,
    dormant_count: 1,
    spawning_count: 0,
    total_sessions: 1,
    active_session_key: null,
  }
  const sessions = new SessionStore({
    summary: async () => summary,
    sessions: async () => list,
    projects: async () => ({ projects }),
  })
  await sessions.refresh()
  return sessions
}

describe('rail density (agent-task-center 9.2)', () => {
  it('does not render the task back-link on rail session rows', async () => {
    vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [] })
    const sessions = await store()
    render(
      <ProjectRail
        sessions={sessions}
        selectedProjectId="p1"
        onSelectProject={() => {}}
        canCreate
        onFocus={() => {}}
        onRequestCreate={() => {}}
      />,
    )
    expect(screen.getByText('推进任务')).toBeTruthy()
    expect(screen.queryByText(/Task bd-/)).toBeNull()
    vi.restoreAllMocks()
  })
})
