// @vitest-environment jsdom
/**
 * Workbench 主区（IA v2）：统计卡条与 "Recent sessions" 表已删除；保留
 * 项目头部（名称 + 分支 pill + `N sessions · ● active` meta）、聚焦会话
 * spotlight + **内联 turn-stream**（<sebas-transcript-view> 就地渲染聚焦
 * 会话）/ 预览原型空态、composer。api client 全量 mock；
 * workbench-composer 模块打桩（其 WA 表单依赖 jsdom 缺失的
 * ElementInternals，且不属于本测试面）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionDetail, SessionRow, Summary } from '../api/client.js'

const apiMocks = vi.hoisted(() => ({
  summary: vi.fn(),
  sessions: vi.fn(),
  settings: vi.fn(),
  session: vi.fn(),
  projectsBranch: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  api: {
    summary: apiMocks.summary,
    sessions: apiMocks.sessions,
    settings: apiMocks.settings,
    session: apiMocks.session,
    projects: { branch: apiMocks.projectsBranch },
  },
}))

vi.mock('../api/shared-ws.js', () => ({
  sharedWs: { subscribe: () => () => {} },
}))

vi.mock('./workbench-composer.js', () => ({}))

import './dashboard.js'
import type { SebasDashboard } from './dashboard.js'

function row(overrides: Partial<SessionRow>): SessionRow {
  return {
    encoded_key: 'oc_x%00',
    chat_id: 'chat-x',
    thread_id: null,
    session_id: 'aaaaaaaa-0001',
    session_id_short: 'aaaa0001',
    status: 'working',
    status_label: 'Working',
    status_slug: 'working',
    status_glyph: '●',
    last_active: '2m ago',
    last_active_unix: 1000,
    is_active: false,
    project_dir: '/home/me/sebas',
    prompt_preview: null,
    ...overrides,
  }
}

const summaryBase: Summary = {
  active_count: 0,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: 2,
  uptime: '1h',
  recent_sessions: [],
  active_session: null,
  active_session_key: null,
  reachability: { ok: true },
}

/** Focused-session detail payload for the inline turn stream. */
function detailFixture(): SessionDetail {
  return {
    chat_id: 'chat-live',
    thread_id: null,
    session_id: 'aaaaaaaa-0009',
    status: 'working',
    status_label: 'Working',
    status_slug: 'working',
    status_glyph: '●',
    user_prompt: 'do the thing',
    body: [
      { element_type: 'markdown', content: 'first entry', created_at_unix: 1_700_000_000 },
      { element_type: 'markdown', content: 'second entry', created_at_unix: 1_700_000_100 },
    ],
    msg_id: null,
    last_active: 'just now',
    encoded_key: 'oc_live%00',
  }
}

/** Summary with a focused session attached. */
function focusedSummary(): Summary {
  return {
    ...summaryBase,
    active_session: {
      chat_id: 'chat-live',
      thread_id: null,
      session_id: 'aaaaaaaa-0009',
      status: 'working',
      status_label: 'Working',
      status_slug: 'working',
      status_glyph: '●',
      encoded_key: 'oc_live%00',
    },
    active_session_key: 'oc_live%00',
  }
}

async function mount(): Promise<SebasDashboard> {
  const el = document.createElement('sebas-dashboard') as SebasDashboard
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

beforeEach(() => {
  // Call counts must not leak between tests (e.g. the "never fetches a
  // detail when nothing is focused" assertion below).
  vi.clearAllMocks()
  apiMocks.summary.mockResolvedValue(summaryBase)
  apiMocks.session.mockResolvedValue(detailFixture())
  apiMocks.sessions.mockResolvedValue({
    recent_sessions: [
      row({}),
      row({ encoded_key: 'oc_y%00', chat_id: 'chat-y', status_slug: 'done', status: 'done' }),
    ],
  })
  apiMocks.settings.mockResolvedValue({
    card_config: {
      theme_color: '#000',
      fold_long_output: false,
      thinking_display: 'auto',
      max_user_text_chars: 0,
      max_tool_output_chars: 0,
    },
    gateway: {
      listen: null,
      provider_count: 1,
      debug: false,
      has_auth: false,
      providers: [{ name: 'anthropic / claude', base_url_anthropic: null, base_url_openai: null }],
    },
  })
  apiMocks.projectsBranch.mockResolvedValue({
    path: '/home/me/sebas',
    branch: 'feat/webui',
    accessible: true,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-dashboard (workbench main area)', () => {
  it('drops the stats strip and the recent-sessions table entirely', async () => {
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.stats')).toBeNull()
    expect(el.shadowRoot!.querySelector('table')).toBeNull()
    expect(el.shadowRoot!.querySelector('.workbench')).toBeNull()
    expect(el.shadowRoot!.querySelector('sebas-project-rail')).toBeNull()
    el.remove()
  })

  it('renders the project header with branch pill and N sessions · active meta', async () => {
    const el = await mount()
    el.selectedPath = '/home/me/sebas'
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    const header = el.shadowRoot!.querySelector('.project-header')
    expect(header?.textContent).toContain('sebas')
    expect(header?.querySelector('.branch-pill')?.textContent).toBe('feat/webui')
    expect(header?.textContent).toContain('2 sessions')
    expect(header?.querySelector('.meta-item.is-active')?.textContent).toContain('active')
    el.remove()
  })

  it('shows the preview-style empty state when no session is focused', async () => {
    const el = await mount()
    const empty = el.shadowRoot!.querySelector('.empty-stream')
    expect(empty?.querySelector('.glyph')).toBeTruthy()
    expect(empty?.textContent).toContain('No session focused')
    expect(empty?.textContent).toContain('sidebar')
    el.remove()
  })

  it('folds the focused-session deep link into the project header when a session is focused', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    // 深链不再是大卡片（.spotlight），而是 header 右段的 .focused-link。
    expect(el.shadowRoot!.querySelector('a.spotlight')).toBeNull()
    const link = el.shadowRoot!.querySelector<HTMLAnchorElement>('a.focused-link')
    expect(link?.getAttribute('href')).toBe('/sessions/oc_live%00')
    expect(link?.textContent).toContain('chat-live')
    expect(el.shadowRoot!.querySelector('.empty-stream')).toBeNull()
    el.remove()
  })

  it('renders the inline turn stream for the focused session on /', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00')
    // 满幅面板区：.turn-stream-area 容器 + fill 模式的 transcript。
    const area = el.shadowRoot!.querySelector('div.turn-stream-area')
    expect(area).toBeTruthy()
    const transcript = area!.querySelector('sebas-transcript-view') as HTMLElement & {
      fill: boolean
      entries: unknown[]
      sessionKey: string
    }
    expect(transcript).toBeTruthy()
    expect(transcript.fill).toBe(true)
    expect(transcript.entries).toHaveLength(2)
    expect(transcript.sessionKey).toBe('oc_live%00')
    el.remove()
  })

  it('degrades to a gentle note when the focused detail cannot be loaded', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockRejectedValue(new Error('404'))
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const area = el.shadowRoot!.querySelector('div.turn-stream-area')
    expect(area?.textContent).toContain('Session unavailable')
    expect(area!.querySelector('sebas-transcript-view')).toBeNull()
    el.remove()
  })

  it('never fetches a detail when nothing is focused', async () => {
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.session).not.toHaveBeenCalled()
    expect(el.shadowRoot!.querySelector('div.turn-stream-area')).toBeNull()
    el.remove()
  })

  it('still mounts the composer in the docked area, bound to the selected project', async () => {
    const el = await mount()
    el.selectedPath = '/home/me/sebas'
    await el.updateComplete
    // 无聚焦会话时也渲染 composer 底座（composer 钉底、area flex 吃满）。
    const area = el.shadowRoot!.querySelector('.composer-area')
    expect(area).toBeTruthy()
    const composer = area!.querySelector('sebas-workbench-composer') as HTMLElement & {
      projectDir?: string | null
    }
    expect(composer).toBeTruthy()
    expect(composer.projectDir).toBe('/home/me/sebas')
    el.remove()
  })
})
