// @vitest-environment jsdom
/**
 * Workbench 主区（IA v2 + workbench-conversation-view）：工作台是唯一对话面
 * ——聚焦会话头（状态徽章/chat id/agent 锁/模型选择/Close/归档）+ review
 * cards + 内联对话（<sebas-transcript-view> 渲染 entries）/ 预览原型空态、
 * composer。api client 全量 mock；workbench-composer 模块打桩（其 WA 表单
 * 依赖 jsdom 缺失的 ElementInternals，且不属于本测试面）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionDetail, SessionRow, Summary } from '../api/client.js'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

// 会话头渲染 WA 表单关联组件（wa-button / wa-dialog / wa-select）——jsdom
// 缺 ElementInternals/setValidity 等会以未处理 rejection 污染整轮退出码。
// 共享垫片（幂等安装）见 test-support/wa-polyfills.ts。
installWaDomPolyfills()

const apiMocks = vi.hoisted(() => ({
  summary: vi.fn(),
  sessions: vi.fn(),
  settings: vi.fn(),
  session: vi.fn(),
  projectsBranch: vi.fn(),
  projectsList: vi.fn(),
  closeSession: vi.fn(),
  archiveSession: vi.fn(),
  nodes: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  api: {
    summary: apiMocks.summary,
    sessions: apiMocks.sessions,
    settings: apiMocks.settings,
    session: apiMocks.session,
    projects: { branch: apiMocks.projectsBranch, list: apiMocks.projectsList },
    closeSession: apiMocks.closeSession,
    archiveSession: apiMocks.archiveSession,
    nodes: apiMocks.nodes,
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
    project_id: 'proj-sebas',
    prompt_preview: null,
    current_model: null,
    available_models: null,
    agent_kind: null,
    pending_count: 0,
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

/** Focused-session detail payload: the conversation entry sequence. */
function detailFixture(): SessionDetail {
  return {
    chat_id: 'chat-live',
    thread_id: null,
    session_id: 'aaaaaaaa-0009',
    status: 'working',
    status_label: 'Working',
    status_slug: 'working',
    status_glyph: '●',
    entries: [
      { position: 0, kind: 'prompt', element_type: 'markdown', content: 'do the thing', created_at_unix: 1_700_000_000 },
      { position: 1, kind: 'content', element_type: 'markdown', content: 'first entry', created_at_unix: 1_700_000_100 },
      { position: 2, kind: 'content', element_type: 'markdown', content: 'second entry', created_at_unix: 1_700_000_200 },
    ],
    msg_id: null,
    last_active: 'just now',
    encoded_key: 'oc_live%00',
    current_model: null,
    available_models: null,
    agent_kind: 'claude',
    pending: [],
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
      current_model: null,
      available_models: null,
      agent_kind: 'claude',
      pending: [],
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
  // dashboard 用 projects.list 把 selectedPath 解析成稳定 id 来分组会话行。
  apiMocks.projectsList.mockResolvedValue({
    projects: [{ id: 'proj-sebas', path: '/home/me/sebas', name: 'sebas', added_at: 0 }],
  })
  apiMocks.sessions.mockResolvedValue({
    recent_sessions: [
      row({}),
      row({ encoded_key: 'oc_y%00', chat_id: 'chat-y', status_slug: 'done', status: 'done' }),
    ],
    active_session_key: null,
  })
  apiMocks.settings.mockResolvedValue({
    card_config: {
      theme_color: '#000',
      fold_long_output: false,
      thinking_display: 'auto',
      max_user_text_chars: 0,
      max_tool_output_chars: 0,
    },
    router: {
      listen: null,
      provider_count: 1,
      debug: false,
      has_auth: false,
      providers: [{ name: 'anthropic / claude', base_url_anthropic: null, base_url_openai: null }],
    },
  })
  apiMocks.projectsBranch.mockResolvedValue({
    project_id: 'proj-sebas',
    branch: 'feat/webui',
    accessible: true,
  })
  apiMocks.nodes.mockResolvedValue({
    nodes: [{ id: 'local', status: 'online', local: true }],
    remote_available: true,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-dashboard (workbench main area)', () => {

  it('shows an inline failure with a retry button when the summary load fails', async () => {
    // add-webui-allowed-roots D6：初始加载失败 = 内联失败态 + 重试入口，
    // 不再只是空白/静默过期。
    apiMocks.summary.mockRejectedValue(new TypeError('Failed to fetch'))
    const el = await mount()
    const callout = el.shadowRoot?.querySelector<HTMLElement>('.callout-error')
    expect(callout).toBeTruthy()
    const retry = el.shadowRoot?.querySelector<HTMLButtonElement>('.retry-btn')
    expect(retry).toBeTruthy()

    // 点击重试：summary 成功返回后错误态清除、正常内容渲染。
    apiMocks.summary.mockResolvedValue(summaryBase)
    retry!.click()
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.callout-error')).toBeNull()
  })
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

  it('renders the inline conversation for the focused session on /', async () => {
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
    expect(transcript.entries).toHaveLength(3)
    expect(transcript.sessionKey).toBe('oc_live%00')
    el.remove()
  })

  it('renders the migrated session head: badge, agent lock, model pick, close + archive (3.3)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      available_models: ['m1', 'm2'],
      current_model: 'm1',
      pending: [{ id: 1, text: 'x', position: 0, disposition: 'turn', priority: false }],
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const head = el.shadowRoot!.querySelector<HTMLElement>('.session-head')
    expect(head).toBeTruthy()
    expect(head!.getAttribute('data-status')).toBe('working')
    expect(head!.querySelector('sebas-status-badge')).toBeTruthy()
    expect(head!.textContent).toContain('chat-live')
    expect(head!.querySelector('[data-testid="agent-lock"]')?.textContent).toContain('claude')
    // 会话内模型选择：available_models 非空才显示。
    const modelSelect = head!.querySelector('wa-select.model-select')
    expect(modelSelect).toBeTruthy()
    // Close/归档动作可达。
    const buttons = [...head!.querySelectorAll('wa-button')].map((b) => b.textContent?.trim())
    expect(buttons).toContain('Close')
    expect(buttons).toContain('Archive')
    // review cards 从工作台可达（3.3）。
    expect(el.shadowRoot!.querySelector('sebas-review-cards')).toBeTruthy()
    el.remove()
  })

  it('gives no session model dropdown when the agent exposes none', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.session-head wa-select.model-select')).toBeNull()
    el.remove()
  })

  it('close confirmation names the discarded pending count (turn-queue semantics)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      pending: [{ id: 1, text: 'queued msg', position: 0, disposition: 'turn', priority: false }],
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 打开确认对话框。
    const closeBtn = [...el.shadowRoot!.querySelectorAll('wa-button')].find((b) =>
      b.textContent?.trim() === 'Close',
    )
    closeBtn!.click()
    await el.updateComplete
    const note = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="close-discards-pending"]')
    expect(note?.textContent).toContain('1')
  })

  it('archives the focused session from the workbench and refetches (3.3)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.archiveSession.mockResolvedValue({ status: 'archived', entry: {} })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const archiveBtn = [...el.shadowRoot!.querySelectorAll('wa-button')].find((b) =>
      b.textContent?.trim() === 'Archive',
    )
    archiveBtn!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.archiveSession).toHaveBeenCalledWith('oc_live%00')
    el.remove()
  })

  it('renders the deep-linked session on /sessions/:key via deepLinkKey (3.4)', async () => {
    // 深链：focus 指针尚未到达（summary 无聚焦），deepLinkKey 先顶上。
    apiMocks.session.mockResolvedValue(detailFixture())
    const el = await mount()
    el.deepLinkKey = 'oc_live%00'
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00')
    expect(el.shadowRoot!.querySelector('.turn-stream-area')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('.empty-stream')).toBeNull()
    el.remove()
  })

  it('shows the honest empty conversation state for a session without entries', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), entries: [] })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const area = el.shadowRoot!.querySelector('div.turn-stream-area')
    expect(area?.textContent).toContain('Nothing yet')
    expect(area!.querySelector('sebas-transcript-view')).toBeNull()
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

describe('provider label sourcing (fix-webui-detached-status)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    apiMocks.summary.mockResolvedValue(summaryBase)
    apiMocks.session.mockResolvedValue(detailFixture())
    apiMocks.sessions.mockResolvedValue({ recent_sessions: [], active_session_key: null })
    apiMocks.projectsBranch.mockResolvedValue({
      project_id: 'proj-sebas',
      branch: 'feat/webui',
      accessible: true,
    })
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  async function labelFor(router: Record<string, unknown>): Promise<string | null> {
    apiMocks.settings.mockResolvedValue({
      card_config: {
        theme_color: '#000',
        fold_long_output: false,
        thinking_display: 'auto',
        max_user_text_chars: 0,
        max_tool_output_chars: 0,
      },
      router,
    })
    const el = await mount()
    const label = (el as unknown as { providerLabel: string | null }).providerLabel
    el.remove()
    return label
  }

  it('shows the first provider name when the source has data', async () => {
    expect(
      await labelFor({
        providers_available: true,
        providers: [{ name: 'anthropic / claude' }],
      }),
    ).toBe('anthropic / claude')
  })

  it('distinguishes an unavailable provider source from "no provider configured"', async () => {
    expect(await labelFor({ providers_available: false, providers: [] })).toBe(
      'provider status unavailable',
    )
  })

  it('keeps "no provider configured" when the source is reachable but empty', async () => {
    expect(await labelFor({ providers_available: true, providers: [] })).toBe(
      'no provider configured',
    )
  })
})

/**
 * add-remote-execution-node 8.3/8.4/8.5：远端会话的期望态 vs 生效态、auto 的
 * ungated 标记、悬空审批的「等待 ≠ 运行中」，以及节点标注与离线成因。
 */
describe('remote session presentation (add-remote-execution-node 8.3-8.5)', () => {
  async function mountWithRemote(
    remote: NonNullable<SessionDetail['remote']>,
  ): Promise<SebasDashboard> {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), remote })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    return el
  }

  it('shows both modes and says the desired mode is not enforced when they differ', async () => {
    const el = await mountWithRemote({
      node_id: 'dev-box',
      node_status: 'online',
      desired_mode: 'ask',
      effective_mode: 'edit',
      parked_approvals: 0,
    })
    const mismatch = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="mode-mismatch"]')
    expect(mismatch).toBeTruthy()
    // 两个值都要显示，且必须说明「没生效」——只显示期望值就是撒谎。
    expect(mismatch!.textContent).toContain('ask')
    expect(mismatch!.textContent).toContain('edit')
    expect(mismatch!.textContent).toContain('无法强制')
    el.remove()
  })

  it('shows a single mode without a mismatch note when desired == effective', async () => {
    const el = await mountWithRemote({
      node_id: 'dev-box',
      node_status: 'online',
      desired_mode: 'ask',
      effective_mode: 'ask',
      parked_approvals: 0,
    })
    expect(el.shadowRoot!.querySelector('[data-testid="mode-mismatch"]')).toBeNull()
    const mode = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-mode"]')
    expect(mode!.textContent).toContain('ask')
    el.remove()
  })

  it('marks an auto session as ungated so it is distinguishable from a gated one', async () => {
    const el = await mountWithRemote({
      node_id: 'dev-box',
      node_status: 'online',
      desired_mode: 'auto',
      effective_mode: 'auto',
      parked_approvals: 0,
    })
    const ungated = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-ungated"]')
    expect(ungated).toBeTruthy()
    expect(ungated!.textContent).toContain('ungated')
    el.remove()
  })

  it('presents a parked session as waiting, not running, and surfaces the count', async () => {
    // 底层 status 仍是 working（进程活着）；有悬空审批就必须读作等待。
    const el = await mountWithRemote({
      node_id: 'dev-box',
      node_status: 'online',
      desired_mode: 'ask',
      effective_mode: 'ask',
      parked_approvals: 2,
    })
    const head = el.shadowRoot!.querySelector<HTMLElement>('.session-head')
    expect(head!.getAttribute('data-status')).toBe('waiting')
    const banner = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="parked-approvals"]')
    expect(banner).toBeTruthy()
    expect(banner!.textContent).toContain('2')
    expect(banner!.textContent).toContain('等待')
    // 决定入口（review cards）可达。
    expect(el.shadowRoot!.querySelector('sebas-review-cards')).toBeTruthy()
    el.remove()
  })

  it('labels the session with its node and states the cause when the node is unreachable', async () => {
    const el = await mountWithRemote({
      node_id: 'dev-box',
      node_status: 'offline',
      node_cause: '链路断开（节点进程未重连）',
      desired_mode: 'ask',
      effective_mode: 'ask',
      parked_approvals: 0,
    })
    const tag = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-head-node"]')
    expect(tag).toBeTruthy()
    expect(tag!.textContent).toContain('dev-box')
    // 成因写在 title 上（如实陈述，不是笼统的「不可用」）。
    expect(tag!.getAttribute('title')).toContain('链路断开')
    el.remove()
  })
})
