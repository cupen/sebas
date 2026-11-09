import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { SessionList, SessionRow, Summary } from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { SessionsPage } from './SessionsPage.js'

afterEach(cleanup)

const row: SessionRow = {
  encoded_key: 'oc_abc',
  thread_id: null,
  session_id: 's-1',
  session_id_short: 's-1',
  status: 'working',
  status_label: 'working',
  status_slug: 'working',
  status_glyph: '',
  last_active: '',
  last_active_unix: 1_700_000_000,
  is_active: true,
  project_id: 'p1',
  prompt_preview: '修复构建',
  label: '构建修复',
  msg_count: 4,
  current_model: 'deepseek-chat',
  available_models: null,
  agent_kind: 'codex',
  pending_count: 0,
  turn_engaged: true,
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
    projects: async () => ({ projects: [] }),
  })
  await sessions.refresh()
  return sessions
}

describe('SessionsPage', () => {
  it('renders cross-project cards and focuses a session', async () => {
    const sessions = await store()
    const onFocus = vi.fn()
    render(
      <SessionsPage sessions={sessions} canCreate onFocus={onFocus} onRequestCreate={() => {}} />,
    )
    expect(screen.getByText('构建修复')).toBeTruthy()
    expect(screen.getByText('运行中')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /聚\s*焦/ }))
    expect(onFocus).toHaveBeenCalledWith('oc_abc')
  })

  it('hides create/rename/archive controls for viewers', async () => {
    const sessions = await store()
    render(
      <SessionsPage sessions={sessions} canCreate={false} onFocus={() => {}} onRequestCreate={() => {}} />,
    )
    expect(screen.queryByRole('button', { name: /新建会话/ })).toBeNull()
    expect(screen.queryByRole('button', { name: '重命名' })).toBeNull()
    expect(screen.queryByRole('button', { name: '归档' })).toBeNull()
    expect(screen.getByText(/只读总览/)).toBeTruthy()
  })
})
