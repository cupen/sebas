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
  activateSession: vi.fn(async () => ({ status: 'already-running' })),
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
    activateSession: apiMocks.activateSession,
    nodes: apiMocks.nodes,
    agents: apiMocks.agents,
  },
}))

/**
 * 共享 WS 客户端 mock（fix-pending-queue-liveness 扩展）：subscribe 捕获
 * handler 供用例派发 WS 帧（emit），验证「事件驱动的 refetch 链」对会话
 * 状态迁移的覆盖（session.updated / turn.append）。其余用例不派发帧，
 * 行为与旧 no-op mock 等价。
 */
const wsMocks = vi.hoisted(() => {
  const handlers = new Set<(ev: unknown) => void>()
  return {
    subscribe: vi.fn((h: (ev: unknown) => void) => {
      handlers.add(h)
      return () => handlers.delete(h)
    }),
    emit: (ev: unknown): void => {
      for (const h of handlers) h(ev)
    },
    clearHandlers: (): void => handlers.clear(),
  }
})

vi.mock('../api/shared-ws.js', () => ({ sharedWs: wsMocks }))

// workbench-composer 模块打桩：WA 表单依赖 jsdom 缺失的 ElementInternals。
// dashboard 同时从该模块取一次性对焦请求的事件名（workbench-rail-polish
// 3.2），mock 里补上常量。
vi.mock('./workbench-composer.js', () => ({
  COMPOSER_FOCUS_REQUEST: 'sebas:composer-focus',
}))

// 对焦中介的可观察 composer 替身：真组件被 mock 掉了，这里顶一个同名
// 元素——focusInput 记录调用并真实把焦点落进 shadow 里的输入框替身
// （用例自行 append `<wa-textarea tabindex="-1">`），偷走/送回都可断言。
const composerFocusInput = vi.fn()
class StubWorkbenchComposer extends HTMLElement {
  constructor() {
    super()
    this.attachShadow({ mode: 'open' })
  }
  focusInput(): void {
    composerFocusInput()
    const target = this.shadowRoot?.querySelector('wa-textarea')
    if (target) target.focus()
  }
}
if (!customElements.get('sebas-workbench-composer')) {
  customElements.define('sebas-workbench-composer', StubWorkbenchComposer)
}

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
  wsMocks.clearHandlers()
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

  it('renders the migrated session head as display-only (workbench-live-conversation-flow 4.2)', async () => {
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
    // 头部去交互化：零按钮、零链接、零 mode/model 切换控件。
    expect(head!.querySelector('wa-button')).toBeNull()
    expect(head!.querySelector('a[href="/sessions"]')).toBeNull()
    expect(head!.querySelector('wa-select')).toBeNull()
    // 切换交互归 composer：mode/model 供数到位。
    const composer = el.shadowRoot!.querySelector('sebas-workbench-composer')
    expect(composer).toBeTruthy()
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

  it('focus transitions fire the idempotent activate call (workbench-live-conversation-flow 3.1)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue(detailFixture())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 聚焦即拉起：焦点 key 首次出现时触发 activate；同焦点不重复请求。
    expect(apiMocks.activateSession).toHaveBeenCalledWith('oc_live%00')
    const calls = apiMocks.activateSession.mock.calls.length
    await new Promise((r) => setTimeout(r, 30))
    expect(apiMocks.activateSession.mock.calls.length).toBe(calls)
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

  it('drives the composer from the engine fact turn_engaged, not the display slug (3.1)', async () => {
    // 泊车会话的呈现词是 waiting；引擎事实 turn_engaged=true 判定「在飞」
    // ——slug 判定（只认 working）会把它伪装成直接发送。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      status_slug: 'waiting',
      turn_engaged: true,
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const area = el.shadowRoot!.querySelector('.composer-area')!
    const composer = area.querySelector('sebas-workbench-composer') as HTMLElement & {
      turnInFlight?: boolean
      waitingApproval?: boolean
    }
    expect(composer.turnInFlight).toBe(true)
    const stack = area.querySelector('sebas-pending-stack') as HTMLElement & {
      turnEngaged?: boolean
      waitingApproval?: boolean
    }
    expect(stack.turnEngaged).toBe(true)
    el.remove()
  })

  it('falls back to the working-slug heuristic when the core predates turn_engaged (3.1)', async () => {
    // 旧 core：键缺省 + slug waiting → 不判定在飞（waiting 呈现词不承担
    // 状态判定，design D3）；对照 slug working 的既有 D4 断言。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      status_slug: 'waiting',
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const composer = el
      .shadowRoot!.querySelector('.composer-area')!
      .querySelector('sebas-workbench-composer') as HTMLElement & { turnInFlight?: boolean }
    expect(composer.turnInFlight).toBe(false)
    el.remove()
  })

  it('lifts the parked fact from review-cards into composer and stack (3.2)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), turn_engaged: true })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const area = el.shadowRoot!.querySelector('.composer-area')!
    const review = area.querySelector('sebas-review-cards') as HTMLElement
    const composer = area.querySelector('sebas-workbench-composer') as HTMLElement & {
      waitingApproval?: boolean
    }
    const stack = area.querySelector('sebas-pending-stack') as HTMLElement & {
      waitingApproval?: boolean
    }
    review.dispatchEvent(
      new CustomEvent('review-pending-changed', {
        detail: { count: 1 },
        bubbles: true,
        composed: true,
      }),
    )
    await el.updateComplete
    expect(composer.waitingApproval).toBe(true)
    expect(stack.waitingApproval).toBe(true)
    review.dispatchEvent(
      new CustomEvent('review-pending-changed', {
        detail: { count: 0 },
        bubbles: true,
        composed: true,
      }),
    )
    await el.updateComplete
    expect(composer.waitingApproval).toBe(false)
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

describe('creation focus chain (workbench-rail-polish 3.2)', () => {
  /** 收敛一轮 refetch 链（mock 全部即时 resolve，一个宏任务轮即够）。 */
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  /** 替身 composer 的输入框：focusInput 的真实落点，焦点核对的数据源。 */
  function mountInput(composer: StubWorkbenchComposer): HTMLElement {
    const input = document.createElement('wa-textarea')
    input.setAttribute('tabindex', '-1')
    composer.shadowRoot!.appendChild(input)
    return input
  }

  it('routes the focus request into the composer once the placeholder is focused', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const composer = el.shadowRoot!.querySelector(
      'sebas-workbench-composer',
    ) as StubWorkbenchComposer
    const input = mountInput(composer)
    expect(composerFocusInput).not.toHaveBeenCalled()

    // rail 创建成功派发的请求（事件名与源码共用常量词表）。
    window.dispatchEvent(new CustomEvent('sebas:composer-focus'))
    await settle(el)
    expect(composerFocusInput).toHaveBeenCalledTimes(1)
    expect(composer.shadowRoot!.activeElement).toBe(input)
    el.remove()
  })

  it('holds the request until the placeholder becomes the focused session', async () => {
    const el = await mount() // summaryBase：active_session_key = null
    await settle(el)
    const composer = el.shadowRoot!.querySelector(
      'sebas-workbench-composer',
    ) as StubWorkbenchComposer
    const input = mountInput(composer)
    window.dispatchEvent(new CustomEvent('sebas:composer-focus'))
    await settle(el)
    // 占位还没成为聚焦会话：焦点按兵不动（摘要往返尚未带回 key）。
    expect(composerFocusInput).not.toHaveBeenCalled()

    // summary 到达（WS / sebas:refetch 同一路径）→ 挂起请求在渲染后落下；
    // 焦点落进输入框后 hasFocus 短路，重试节拍不再重复送焦。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    ;(el as unknown as { refetch: () => void }).refetch()
    await settle(el)
    expect(composerFocusInput).toHaveBeenCalledTimes(1)
    expect(composer.shadowRoot!.activeElement).toBe(input)
    el.remove()
  })

  it('the composer binding stays intact under the stub (sessionKey flows)', async () => {
    // 守护用例：替身元素不得破坏既有 composer 供数绑定（纯属性直填）。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const composer = el.shadowRoot!.querySelector('sebas-workbench-composer') as HTMLElement & {
      sessionKey?: string | null
    }
    expect(composer.sessionKey).toBe('oc_live%00')
    el.remove()
  })

  it('re-claims focus when an async focus() steals it within the short window', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const composer = el.shadowRoot!.querySelector(
      'sebas-workbench-composer',
    ) as StubWorkbenchComposer
    const input = mountInput(composer)

    window.dispatchEvent(new CustomEvent('sebas:composer-focus'))
    await settle(el)
    // 首次落焦真的进了输入框。
    expect(composerFocusInput).toHaveBeenCalledTimes(1)
    expect(composer.shadowRoot!.activeElement).toBe(input)

    // 模拟 wa-dialog 关闭动画尾部的 trigger.focus() 插队：焦点被偷出 composer。
    const thief = document.createElement('button')
    document.body.appendChild(thief)
    thief.focus()
    expect(composer.shadowRoot!.activeElement).toBeNull()

    // 短窗内的下一个节拍把焦点重新送回输入框（这才是 spec 场景的终态）。
    await new Promise((r) => setTimeout(r, 300))
    expect(composerFocusInput.mock.calls.length).toBeGreaterThanOrEqual(2)
    expect(composer.shadowRoot!.activeElement).toBe(input)
    el.remove()
  })

  it('treats focus deep inside the textarea shadow as already in place (no re-send)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const composer = el.shadowRoot!.querySelector(
      'sebas-workbench-composer',
    ) as StubWorkbenchComposer
    // 真实几何（tasks.md 4.1 的 focusChain 终态）：composer shadow 里的
    // wa-textarea 宿主 + 它自己 shadow 里的原生 textarea——键盘焦点真身
    // 落在最内层。
    const host = document.createElement('wa-textarea')
    host.attachShadow({ mode: 'open' })
    const native = document.createElement('textarea')
    host.shadowRoot!.appendChild(native)
    composer.shadowRoot!.appendChild(host)
    native.focus()

    window.dispatchEvent(new CustomEvent('sebas:composer-focus'))
    await settle(el)
    // hasFocus 穿透两层 shadow 认出焦点已在输入框内：短窗不重送
    // （focusInput 对已聚焦元素是幂等 no-op，这里的断言是"一次都不必发"）。
    expect(composerFocusInput).not.toHaveBeenCalled()
    el.remove()
  })

  it('gives up after the deadline and does not steal focus back', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const composer = el.shadowRoot!.querySelector(
      'sebas-workbench-composer',
    ) as StubWorkbenchComposer
    const input = mountInput(composer)

    window.dispatchEvent(new CustomEvent('sebas:composer-focus'))
    await settle(el)
    // 窗内偷一次：会被纠正（重送焦点）。
    const thief = document.createElement('button')
    document.body.appendChild(thief)
    thief.focus()
    await new Promise((r) => setTimeout(r, 300))
    expect(composerFocusInput.mock.calls.length).toBeGreaterThanOrEqual(2)
    expect(composer.shadowRoot!.activeElement).toBe(input)

    // 熬过 ~1s 截止（派发起已耗 ~300ms，再候 900ms 到窗外）。
    await new Promise((r) => setTimeout(r, 900))
    const callsAtExpiry = composerFocusInput.mock.calls.length

    // 截止后焦点再被偷走也不回收——绝不抢别的焦点。
    thief.focus()
    await new Promise((r) => setTimeout(r, 300))
    expect(composerFocusInput.mock.calls.length).toBe(callsAtExpiry)
    expect(composer.shadowRoot!.activeElement).toBeNull()
    el.remove()
  })
})

/**
 * fix-pending-queue-liveness：静默工作窗（stall 场景）内 refetch 链的组件级
 * 钉死。契约问题：「turn 开轮 → 首帧内容」产生的 session.updated 与
 * turn.append 帧到达后，dashboard 的 refetch（任意 WS 帧都触发，读的是
 * 实时 summary/detail）能否把引擎事实 turn_engaged 送进 composer 的五态机
 * ——送达即呈 stop/queued，静默窗全程不回退 disabled。
 *
 * mock 数据形状对齐 wire 契约：turn_engaged 只在 true 时上 wire（键缺省 =
 * false），所以消费面的回退链（detail ?? summary ?? slug）用「键缺省」表达。
 */
describe('sebas-dashboard (silent working window refetch chain)', () => {
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  function composerOf(el: SebasDashboard): StubWorkbenchComposer & {
    waitingApproval?: boolean
  } {
    return el.shadowRoot!.querySelector('sebas-workbench-composer') as unknown as
      StubWorkbenchComposer & { waitingApproval?: boolean }
  }

  function stackOf(el: SebasDashboard): HTMLElement & { turnEngaged?: boolean } {
    return el.shadowRoot!.querySelector('sebas-pending-stack') as unknown as
      HTMLElement & { turnEngaged?: boolean }
  }

  it('a session.updated frame mid-window refetches and drives the composer to turn-in-flight', async () => {
    // 开轮瞬间（SEED）：turn_engaged 键不上 wire，slug queued——composer 禁用。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      status_slug: 'queued',
    })
    const el = await mount()
    expect(composerOf(el).turnInFlight).toBe(false)

    // 引擎在首个内容帧翻 WORKING 并发布 Updated；WS 帧触发 refetch，
    // 实时读回 working + turn_engaged=true（stall 场景静默窗的稳态）。
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: { ...focusedSummary().active_session!, turn_engaged: true },
    })
    apiMocks.session.mockResolvedValue({ ...detailFixture(), turn_engaged: true })
    wsMocks.emit({ type: 'session.updated', session_id: 'oc_live%00', status: 'working' })
    await settle(el)

    expect(composerOf(el).turnInFlight).toBe(true)
    expect(stackOf(el).turnEngaged).toBe(true)
    el.remove()
  })

  it('a turn.append frame alone keeps the view fresh through the silent window', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), status_slug: 'queued' })
    const el = await mount()
    expect(composerOf(el).turnInFlight).toBe(false)

    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: { ...focusedSummary().active_session!, turn_engaged: true },
    })
    apiMocks.session.mockResolvedValue({ ...detailFixture(), turn_engaged: true })
    wsMocks.emit({
      type: 'turn.append',
      session_id: 'oc_live%00',
      entries: [],
      seq: 3,
    })
    await settle(el)

    expect(composerOf(el).turnInFlight).toBe(true)
    el.remove()
  })

  it('a stale detail payload without the key does not mask a fresh summary fact (fallback chain)', async () => {
    // detail 在途/滞回（无键，旧形状）不应遮蔽 summary 已到的新事实——
    // 消费链 detail ?? summary ?? slug 的 `??` 半边。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), status_slug: 'queued' })
    const el = await mount()
    expect(composerOf(el).turnInFlight).toBe(false)

    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: { ...focusedSummary().active_session!, turn_engaged: true },
    })
    // detail 故意停在旧形状（无 turn_engaged 键）。
    wsMocks.emit({ type: 'session.updated', session_id: 'oc_live%00', status: 'working' })
    await settle(el)

    expect(composerOf(el).turnInFlight).toBe(true)
    el.remove()
  })

  it('an API-created working session that is NOT focused leaves the composer unbound (sessionKey null)', async () => {
    // 观测症状的机制钉死：API 创建会话（web_spawn）不动 web 焦点指针——
    // 浏览器停在 `/` 且未聚焦该会话时，composer 的 sessionKey 为 null，
    // submitState 第一分支即 disabled，与 turn 状态无关。深链/点击聚焦后
    // 才进入上一组用例的 refetch 链。
    apiMocks.summary.mockResolvedValue({
      ...summaryBase,
      active_session: null,
      active_session_key: null,
      recent_sessions: [row({ encoded_key: 'oc_live%00', chat_id: 'chat-live', status_slug: 'working' })],
    })
    const el = await mount()
    await settle(el)

    const composer = composerOf(el)
    expect(composer.sessionKey ?? null).toBeNull()
    expect(composer.turnInFlight).toBe(false)
    // detail 从未被拉（无焦点键可拉）。
    expect(apiMocks.session).not.toHaveBeenCalled()
    el.remove()
  })
})
