import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  api,
  type SessionDetail,
  type SessionList,
  type SessionRow,
  type Summary,
} from '../../api/client.js'
import { SessionStore } from '../state/session-store.js'
import { WorkbenchHome } from './WorkbenchHome.js'

afterEach(cleanup)

const row: SessionRow = {
  encoded_key: 'oc_focus',
  thread_id: null,
  session_id: 's-focus',
  session_id_short: 's-focus',
  status: 'waiting',
  status_label: 'waiting',
  status_slug: 'waiting',
  status_glyph: '',
  last_active: '',
  last_active_unix: 1_700_000_000,
  is_active: true,
  project_id: 'p1',
  prompt_preview: '等待审批',
  label: null,
  msg_count: 2,
  current_model: 'm1',
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
    spawning_count: 1,
    total_sessions: 3,
    uptime: '2h',
    recent_sessions: [row],
    active_session: {
      chat_id: 'c-focus',
      encoded_key: row.encoded_key,
      thread_id: null,
      session_id: row.session_id,
      session_id_short: row.session_id_short,
      status_slug: row.status_slug,
      status: row.status,
      status_label: row.status_label,
      status_glyph: '',
      current_model: row.current_model,
      prompt_preview: row.prompt_preview,
      msg_count: row.msg_count,
      pending_count: 0,
      available_models: null,
      agent_kind: 'codex',
      pending: [],
      desired_mode: 'ask',
    } as Summary['active_session'],
    active_session_key: row.encoded_key,
    reachability: { ok: true },
  }
  const list: SessionList = {
    recent_sessions: [row],
    active_count: 1,
    dormant_count: 0,
    spawning_count: 1,
    total_sessions: 3,
    active_session_key: row.encoded_key,
  }
  const sessions = new SessionStore({
    summary: async () => summary,
    sessions: async () => list,
    projects: async () => ({ projects: [] }),
  })
  await sessions.refresh()
  return sessions
}

const detail: SessionDetail = {
  chat_id: 'c1',
  thread_id: null,
  session_id: 's-1',
  status: 'working',
  status_label: 'working',
  status_slug: 'working',
  status_glyph: '',
  entries: [
    { position: 0, kind: 'prompt', element_type: 'markdown', content: '重构登录', created_at_unix: 0 },
    { position: 1, kind: 'agent', element_type: 'markdown', content: '开始处理', created_at_unix: 0 },
  ],
  msg_id: null,
  last_active: '',
  encoded_key: 'oc_zoom%2F1',
  msg_count: 2,
  current_model: 'deepseek-chat',
  available_models: null,
  agent_kind: 'codex',
  desired_mode: 'ask',
  pending: [],
  turn_engaged: false,
}

describe('WorkbenchHome', () => {
  it('shows counters and opens the focused session', async () => {
    const sessions = await store()
    const onFocus = vi.fn()
    render(
      <WorkbenchHome
        sessionKey={null}
        sessions={sessions}
        selectedProjectId={null}
        canCreate
        onRequestCreate={() => {}}
        onFocus={onFocus}
      />,
    )
    expect(screen.getByText('活跃会话')).toBeTruthy()
    expect(screen.getByText('2h')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '打开会话' }))
    expect(onFocus).toHaveBeenCalledWith('oc_focus')
  })

  it('loads a deep link by its raw encoded key and renders the read-only transcript', async () => {
    const session = vi.spyOn(api, 'session').mockResolvedValue(detail)
    const sessions = await store()
    render(
      <WorkbenchHome
        sessionKey="oc_zoom%2F1"
        sessions={sessions}
        selectedProjectId={null}
        canCreate={false}
        onRequestCreate={() => {}}
        onFocus={() => {}}
      />,
    )
    const transcript = await screen.findByTestId('react-transcript')
    expect(within(transcript).getByText('重构登录')).toBeTruthy()
    expect(within(transcript).getByText('开始处理')).toBeTruthy()
    expect(session).toHaveBeenCalledWith('oc_zoom%2F1')
    expect(screen.getByText('oc_zoom%2F1')).toBeTruthy()
    expect(screen.getAllByText('deepseek-chat').length).toBeGreaterThan(0)
    session.mockRestore()
  })

  it('reports a load failure instead of an empty transcript', async () => {
    const session = vi.spyOn(api, 'session').mockRejectedValue(new Error('HTTP 404: gone'))
    const sessions = await store()
    render(
      <WorkbenchHome
        sessionKey="oc_missing"
        sessions={sessions}
        selectedProjectId={null}
        canCreate={false}
        onRequestCreate={() => {}}
        onFocus={() => {}}
      />,
    )
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('会话加载失败'))
    session.mockRestore()
  })
})
