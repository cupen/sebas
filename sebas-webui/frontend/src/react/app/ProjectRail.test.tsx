import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { api, type ArchiveEntry, type Project, type SessionList, type SessionRow, type Summary } from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { ProjectRail } from './ProjectRail.js'

afterEach(cleanup)

const projects: Project[] = [
  { id: 'p1', name: 'alpha', path: '/work/alpha', added_at: 1 },
  { id: 'p2', name: 'beta', path: '/work/beta', added_at: 2 },
]

const row: SessionRow = {
  encoded_key: 'oc_1',
  thread_id: null,
  session_id: 's1',
  session_id_short: 's1',
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  last_active: '',
  last_active_unix: 1_700_000_000,
  is_active: true,
  project_id: 'p1',
  prompt_preview: '首个任务',
  label: null,
  msg_count: 3,
  current_model: 'm1',
  available_models: null,
  agent_kind: 'codex',
  pending_count: 0,
  turn_engaged: false,
  desired_mode: 'ask',
}

async function store(): Promise<SessionStore> {
  const summary: Summary = {
    active_count: 1,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 1,
    uptime: '1h',
    recent_sessions: [row],
    active_session: null,
    active_session_key: null,
    reachability: { ok: true },
  }
  const list: SessionList = {
    recent_sessions: [row],
    active_count: 1,
    dormant_count: 0,
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

const archived: ArchiveEntry = {
  session_key: 'oc_old',
  project_path: '/work/alpha',
  label: '旧会话',
  archived_at: 1_700_000_000,
  retention_deadline: 1_800_000_000,
  operator_label: '归档标题',
}

describe('ProjectRail', () => {
  it('groups sessions under projects and focuses them', async () => {
    vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [] })
    const sessions = await store()
    const onFocus = vi.fn()
    render(
      <ProjectRail
        sessions={sessions}
        selectedProjectId={null}
        onSelectProject={() => {}}
        canCreate
        onFocus={onFocus}
        onRequestCreate={() => {}}
      />,
    )
    expect(await screen.findByText('alpha')).toBeTruthy()
    expect(screen.getByText('beta')).toBeTruthy()
    fireEvent.click(screen.getByText('首个任务'))
    expect(onFocus).toHaveBeenCalledWith('oc_1')
    vi.restoreAllMocks()
  })

  it('offers an up/down alternative instead of drag for project order', async () => {
    vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [] })
    const reorder = vi.spyOn(api.projects, 'reorder').mockResolvedValue({ projects })
    const sessions = await store()
    render(
      <ProjectRail
        sessions={sessions}
        selectedProjectId={null}
        onSelectProject={() => {}}
        canCreate
        onFocus={() => {}}
        onRequestCreate={() => {}}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: '项目操作 beta' }))
    fireEvent.click(await screen.findByText('上移项目'))
    await waitFor(() => expect(reorder).toHaveBeenCalledWith(['p2', 'p1']))
    vi.restoreAllMocks()
  })

  it('restores an archived session from the history section', async () => {
    vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [archived] })
    const restore = vi.spyOn(api, 'restoreSession').mockResolvedValue({ status: 'ok', entry: archived })
    const sessions = await store()
    render(
      <ProjectRail
        sessions={sessions}
        selectedProjectId={null}
        onSelectProject={() => {}}
        canCreate={false}
        onFocus={() => {}}
        onRequestCreate={() => {}}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: /历史/ }))
    expect(await screen.findByText('归档标题')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /恢\s*复/ }))
    await waitFor(() => expect(restore).toHaveBeenCalledWith('oc_old'))
    vi.restoreAllMocks()
  })

  it('hides project mutations for viewers', async () => {
    vi.spyOn(api, 'archiveList').mockResolvedValue({ archived_sessions: [] })
    const sessions = await store()
    render(
      <ProjectRail
        sessions={sessions}
        selectedProjectId={null}
        onSelectProject={() => {}}
        canCreate={false}
        onFocus={() => {}}
        onRequestCreate={() => {}}
      />,
    )
    await screen.findByText('alpha')
    expect(screen.queryByRole('button', { name: '添加项目' })).toBeNull()
    expect(screen.queryByRole('button', { name: '项目操作 alpha' })).toBeNull()
    vi.restoreAllMocks()
  })
})
