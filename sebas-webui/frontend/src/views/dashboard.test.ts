// @vitest-environment jsdom
/**
 * Workbench 主区（IA v2 + workbench-conversation-view）：工作台是唯一对话面
 * ——聚焦会话头（状态徽章/chat id/agent 锁/模型选择/Close/归档）+ review
 * cards + 内联对话（<sebas-transcript-view> 渲染 entries）/ 预览原型空态、
 * composer。api client 全量 mock；workbench-composer 模块打桩（其 WA 表单
 * 依赖 jsdom 缺失的 ElementInternals，且不属于本测试面）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConversationEntryView, SessionDetail, SessionRow, Summary } from '../api/client.js'
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
  agents: vi.fn(),
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
    agents: apiMocks.agents,
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
    msg_count: 0,
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
    msg_count: 2,
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
  // workbench-agent-identity 3.1：agent 目录（聚焦会话 display 名解析）。
  apiMocks.agents.mockResolvedValue({
    agents: [{ id: 'claude', display: 'Claude Code', reachable: true }],
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

  it('passes the agent display name to the transcript (agent-identity 3.1)', async () => {
    // workbench-agent-identity：dashboard 按 agent_kind 匹配 /api/agents 的
    // display 传入 transcript。已收到角标是纯 entry 序派生态（3.2 收尾修正），
    // 不再需要 sessionWorking 传参。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.agents.mockResolvedValue({
      agents: [{ id: 'claude', display: 'Claude Code', reachable: true }],
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const transcript = el.shadowRoot!.querySelector('sebas-transcript-view') as HTMLElement & {
      agentDisplay: string | null
    }
    expect(transcript.agentDisplay).toBe('Claude Code')
    el.remove()
  })

  it('falls back to the raw agent_kind slug when the catalog has no display entry (3.1)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.agents.mockResolvedValue({ agents: [] })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const transcript = el.shadowRoot!.querySelector('sebas-transcript-view') as HTMLElement & {
      agentDisplay: string | null
    }
    expect(transcript.agentDisplay).toBe('claude')
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

  it('still mounts the composer in the docked area, bound to the focused session', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      available_models: ['m1', 'm2'],
      current_model: 'm1',
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // composer 底座常驻（composer 钉底、area 吃满输入框分割面）。
    const area = el.shadowRoot!.querySelector('.composer-area')
    expect(area).toBeTruthy()
    const composer = area!.querySelector('sebas-workbench-composer') as HTMLElement & {
      sessionKey?: string | null
      turnInFlight?: boolean
      sessionModels?: string[]
      currentModel?: string | null
    }
    expect(composer).toBeTruthy()
    // 聚焦会话指针驱动 composer（workbench-interaction-polish 4.1）。
    expect(composer.sessionKey).toBe('oc_live%00')
    // D4：聚焦会话 working = turnInFlight 下发。
    expect(composer.turnInFlight).toBe(true)
    // 会话内模型面与当前模型照常透传。
    expect(composer.sessionModels).toEqual(['m1', 'm2'])
    expect(composer.currentModel).toBe('m1')
    el.remove()
  })

  it('stages the conversation and the composer in a vertical wa-split-panel (5.2/D1)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const panel = el.shadowRoot!.querySelector('wa-split-panel.vsplit') as HTMLElement
    expect(panel).toBeTruthy()
    expect(panel.getAttribute('orientation')).toBe('vertical')
    // composer 高度边界：最低 120px、最高主区一半；初始高度来自记忆
    // （此环境无 storage → 默认 220）。
    expect(panel.getAttribute('position-in-pixels')).toBe('220')
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(styleText).toMatch(/wa-split-panel\.vsplit\s*\{[^}]*--min:\s*120px/)
    expect(styleText).toMatch(/wa-split-panel\.vsplit\s*\{[^}]*--max:\s*50%/)
    // 舞台浮岛（D6）：stage 列内圆角卡片。
    expect(el.shadowRoot!.querySelector('.stage-island')).toBeTruthy()
    el.remove()
  })

  it('clamps and persists the composer height when the divider drags (5.2/D1)', async () => {
    const el = await mount()
    const target = el as unknown as {
      onComposerReposition: (e: Event) => void
      composerHeight: number
    }
    // getBoundingClientRect 在 jsdom 返回 0 → 面积不可信 → 只保 120px 下限。
    target.onComposerReposition({
      currentTarget: { positionInPixels: 9999 },
    } as unknown as Event)
    expect(target.composerHeight).toBe(120)
    target.onComposerReposition({
      currentTarget: { positionInPixels: Number.NaN },
    } as unknown as Event)
    expect(target.composerHeight).toBe(120)
    el.remove()
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

/**
 * conversation-incremental-sync 2.1/2.2：内存游标与增量 merge。游标 =
 * 已渲染序列的末位 position（per-session Map，页面生命周期内）；首次聚焦
 * 全量拉，后续 refetch 带 `entries_after=游标` 增量 append；merge 按
 * position 升序、过滤 `<= 游标` 去重；失败游标不推进；重载（实例重建）
 * 后重新全量。session mock 按后端契约替身：entries 过滤 position > n。
 */
describe('conversation incremental sync (conversation-incremental-sync 2.1/2.2)', () => {
  /** 收敛一轮 refetch 链（mock 全部即时 resolve，一个宏任务轮即够）。 */
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  function transcriptOf(
    el: SebasDashboard,
  ): (HTMLElement & { entries: ConversationEntryView[] }) | null {
    return el.shadowRoot!.querySelector<HTMLElement & { entries: ConversationEntryView[] }>(
      'sebas-transcript-view',
    )
  }

  it('fetches the full sequence on first focus (no entries_after)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledTimes(1)
    // 精确单参调用 = 未携带 entries_after（spec「first fetch is full」）。
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00')
    expect(transcriptOf(el)!.entries.map((e) => e.position)).toEqual([0, 1, 2])
    el.remove()
  })

  it('appends only new entries on a websocket-triggered refetch (incremental)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    // 新条目到达：响应契约 = entries 只含 position > 游标(2)，status 随行。
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      status: 'done',
      status_label: 'Done',
      status_slug: 'done',
      status_glyph: '✓',
      msg_count: 3,
      entries: [
        { position: 3, kind: 'content', element_type: 'markdown', content: 'third entry', created_at_unix: 1_700_000_300 },
      ],
    })
    // WS 事件与动作后 refetch 同走 refetch() → loadFocused 链路
    // （dashboard 监听的 sebas:refetch 与 sharedWs 回调共用同一入口）。
    window.dispatchEvent(new Event('sebas:refetch'))
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00', 2)
    const transcript = transcriptOf(el)!
    expect(transcript.entries.map((e) => e.position)).toEqual([0, 1, 2, 3])
    expect(transcript.entries[3].content).toBe('third entry')
    // D2：status 变化随增量响应同行（会话头状态翻转，entries 只增不重拉）。
    expect(el.shadowRoot!.querySelector('.session-head')?.getAttribute('data-status')).toBe('done')
    el.remove()
  })

  it('keeps per-session cursors: switching back resumes from each session cursor', async () => {
    const a = detailFixture() // positions 0..2
    const b: SessionDetail = {
      ...detailFixture(),
      encoded_key: 'oc_b%00',
      chat_id: 'chat-b',
      session_id: 'bbbbbbbb-0002',
      entries: [
        { position: 0, kind: 'prompt', element_type: 'markdown', content: 'b prompt', created_at_unix: 1_700_001_000 },
        { position: 1, kind: 'content', element_type: 'markdown', content: 'b reply', created_at_unix: 1_700_001_100 },
      ],
    }
    apiMocks.session.mockImplementation((key: string, entriesAfter?: number) => {
      const base = key === a.encoded_key ? a : key === b.encoded_key ? b : null
      if (!base) return Promise.reject(new Error(`404: ${key}`))
      return Promise.resolve({
        ...base,
        entries: base.entries.filter((e) => e.position > (entriesAfter ?? -1)),
      })
    })
    const el = await mount() // summaryBase：无聚焦
    el.deepLinkKey = 'oc_live%00'
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00') // A 首拉全量
    el.deepLinkKey = 'oc_b%00'
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledWith('oc_b%00') // B 首拉全量
    // A 在后台长出新条目（position 3）。
    a.entries.push({ position: 3, kind: 'content', element_type: 'markdown', content: 'a third', created_at_unix: 1_700_000_300 })
    el.deepLinkKey = 'oc_live%00'
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledWith('oc_live%00', 2) // 从 A 自己的游标增量
    expect(transcriptOf(el)!.entries.map((e) => e.content)).toEqual([
      'do the thing',
      'first entry',
      'second entry',
      'a third',
    ])
    // B 的游标独立：切回 B 从 B 的游标(1)增量——既不是全量也不是 A 的游标。
    el.deepLinkKey = 'oc_b%00'
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledWith('oc_b%00', 1)
    expect(transcriptOf(el)!.entries.map((e) => e.position)).toEqual([0, 1])
    el.remove()
  })

  it('keeps sequence and cursor when an incremental fetch fails; next refetch resumes', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    apiMocks.session.mockRejectedValue(new TypeError('Failed to fetch'))
    window.dispatchEvent(new Event('sebas:refetch'))
    await settle(el)
    let calls = apiMocks.session.mock.calls
    expect(calls[calls.length - 1]).toEqual(['oc_live%00', 2])
    // 序列与游标原样保留：transcript 还在、内容完整，不落「Session unavailable」。
    const transcript = transcriptOf(el)!
    expect(transcript.entries.map((e) => e.position)).toEqual([0, 1, 2])
    expect(el.shadowRoot!.textContent).not.toContain('Session unavailable')
    // 下次成功：仍从同一游标重试（未推进），新条目 append 无缺口。
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      entries: [
        { position: 3, kind: 'content', element_type: 'markdown', content: 'after failure', created_at_unix: 1_700_000_300 },
      ],
    })
    window.dispatchEvent(new Event('sebas:refetch'))
    await settle(el)
    calls = apiMocks.session.mock.calls
    expect(calls[calls.length - 1]).toEqual(['oc_live%00', 2])
    expect(transcriptOf(el)!.entries.map((e) => e.content)).toEqual([
      'do the thing',
      'first entry',
      'second entry',
      'after failure',
    ])
    el.remove()
  })

  it('refetches the full sequence after a reload (memory cursors only)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el1 = await mount()
    await settle(el1)
    expect(apiMocks.session).toHaveBeenCalledTimes(1)
    el1.remove()
    // F5 = 全新元素实例：内存游标表是实例字段，随实例消亡 → 重新全量
    // （spec「reload resets cursors」，无 stale localStorage 游标）。
    const el2 = document.createElement('sebas-dashboard') as SebasDashboard
    document.body.appendChild(el2)
    await el2.updateComplete
    await settle(el2)
    expect(apiMocks.session).toHaveBeenCalledTimes(2)
    const calls = apiMocks.session.mock.calls
    expect(calls[calls.length - 1]).toEqual(['oc_live%00']) // 无 entries_after = 全量
    expect(transcriptOf(el2)!.entries.map((e) => e.position)).toEqual([0, 1, 2])
    el2.remove()
  })
})
