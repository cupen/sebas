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
  archiveDetail: vi.fn(),
  restoreSession: vi.fn(),
  switchSession: vi.fn(),
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
    archiveDetail: apiMocks.archiveDetail,
    restoreSession: apiMocks.restoreSession,
    switchSession: apiMocks.switchSession,
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
// RAIL_FOCUS_EVENT：rail 切换成功的窗口级聚焦事件（4.1，design D3）。
import { RAIL_FOCUS_EVENT } from './project-rail.js'
// PROJECT_FOLLOW_EVENT / focusedProjectPath：聚焦反投影项目上下文（本 change 4.1）。
import { PROJECT_FOLLOW_EVENT, focusedProjectPath, receiptPhaseActive } from './dashboard.js'
// writeFocusAnchor：焦点处立读锚的既有锚点写入（round3 3.1）。
import { writeFocusAnchor } from './unread-cursor.js'
import type { SebasDashboard } from './dashboard.js'
import { SebasDashboard as DashboardImpl } from './dashboard.js'

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
    turn_engaged: false,
    desired_mode: 'ask',
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
    turn_engaged: false,
    desired_mode: 'ask',
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
      project_id: 'proj-sebas',
      pending: [],
      turn_engaged: false,
      desired_mode: 'ask',
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


describe('receipt phase gating (fix-webui-qa-defects-round4 review)', () => {
  it('prompt-tail with no agent entry and a non-terminal slug is a live receipt phase', () => {
    expect(receiptPhaseActive([{ kind: 'prompt' }], 'working')).toBe(true)
    expect(receiptPhaseActive([{ kind: 'prompt' }], 'starting')).toBe(true)
    expect(receiptPhaseActive([{ kind: 'prompt' }], null)).toBe(true)
  })
  it('terminal slugs expire the receipt fact (zero-output completed turn)', () => {
    expect(receiptPhaseActive([{ kind: 'prompt' }], 'done')).toBe(false)
    expect(receiptPhaseActive([{ kind: 'prompt' }], 'failed')).toBe(false)
  })
  it('agent output entries end the receipt phase regardless of slug', () => {
    expect(receiptPhaseActive([{ kind: 'prompt' }, { kind: 'content' }], 'working')).toBe(false)
    expect(receiptPhaseActive([], 'working')).toBe(false)
    expect(receiptPhaseActive(undefined, 'working')).toBe(false)
  })
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

  it('renders the project header without session-count or active/idle copies (3.6, D6b)', async () => {
    const el = await mount()
    el.selectedPath = '/home/me/sebas'
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // （3.6，D6b）project-header 只保留真导航信息（项目名 + 节点 chip +
    // 分支 pill）；「X sessions」计数与 active/idle 活跃度徽标下线——会话
    // 状态只挂 rail 行首圆点一处。
    const header = el.shadowRoot!.querySelector('.project-header')
    expect(header?.textContent).toContain('sebas')
    expect(header?.querySelector('.branch-pill')?.textContent).toBe('feat/webui')
    expect(header?.textContent).not.toContain('sessions')
    expect(header?.querySelector('.active-dot')).toBeNull()
    expect(header?.querySelector('.meta-item.is-active')).toBeNull()
    el.remove()
  })

  it('shows the preview-style empty state when no session is focused', async () => {
    const el = await mount()
    const empty = el.shadowRoot!.querySelector('.empty-stream')
    expect(empty?.querySelector('.glyph')).toBeTruthy()
    expect(empty?.textContent).toContain('未聚焦任何会话')
    expect(empty?.textContent).toContain('项目树')
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
    // （3.6，D6b）focused-link 保留 chat_id 锚点，不再复述状态 slug。
    expect(link?.querySelector('sebas-status-badge')).toBeNull()
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
    // （3.6，D6b）session-head 卡片不再有状态徽标与状态边框属性——会话
    // 状态只挂 rail 行首圆点；卡片保留 chat / agent / model / mode / actions。
    expect(head!.getAttribute('data-status')).toBeNull()
    expect(head!.querySelector('sebas-status-badge')).toBeNull()
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
    expect(area?.textContent).toContain('还没有对话')
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
    expect(area?.textContent).toContain('会话不可得')
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
      turn_engaged: true,
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
    // D4 + （2.1）：聚焦会话在飞（turn_engaged=true 随详情下发）。
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

  it('never derives turn-in-flight from the display slug (2.1, D2)', async () => {
    // （2.1，D2）waiting 呈现词不承担状态判定，且 `status_slug === 'working'`
    // 的字符串回退分支已删除：turn_engaged=false（哪怕 slug 是 working）
    // 一律如实读作空闲——状态只来自引擎事实帧/详情。
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

  it('converges the island spacing on the compact space-2 token (3.5, D6)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // 舞台列与输入框列共享同一水平内边距 token（stage|composer 内容边缘
    // 对齐，D6「两列同一 inset」），竖向留缝同收 space-2。
    expect(styleText).toMatch(
      /\.stage-col\s*\{[^}]*padding:\s*0\s+var\(--sebas-space-2\)\s+0\s+var\(--sebas-space-2\)/,
    )
    expect(styleText).toMatch(
      /\.composer-col\s*\{[^}]*padding:\s*0\s+var\(--sebas-space-2\)\s+var\(--sebas-space-2\)/,
    )
    // 收敛不消除拖拽边界：stage|composer 分割缝保持 6px 可达把手。
    expect(styleText).toMatch(/wa-split-panel\.vsplit\s*\{[^}]*--divider-width:\s*6px/)
    el.remove()
  })

  it('keeps the permission mode badge on one line instead of per-character wrapping (round4 3.3)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const styleText = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    // ≤640px 窄视口下 .mode-tag 曾被挤压成逐字竖排——章内禁止断行；横向
    // 溢出由 meta 行 flex-wrap 兜底（整枚章换行）。
    expect(styleText).toMatch(
      /\.mode-tag\s*\{[^}]*white-space:\s*nowrap;/,
    )
    expect(styleText).toMatch(
      /\.session-head \.meta\s*\{[^}]*flex-wrap:\s*wrap;/,
    )
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
    // （polish-workbench-walkthrough-ux 4.2）过渡态 = 中性灰「模式切换中…」，
    // 不再有红色「无法强制」/ UNKNOWN 措辞。
    expect(mismatch!.textContent).toContain('模式切换中…')
    expect(mismatch!.textContent).not.toContain('无法强制')
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
    expect(mode!.textContent).toContain('逐次询问')
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
    // （4.2）auto 与 ungated 章合一：mode 章显示「自动执行」，英文 UNGATED 章删除。
    const ungated = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-ungated"]')
    expect(ungated).toBeNull()
    const mode = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-mode"]')
    expect(mode!.textContent).toContain('自动执行')
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
    // （3.6，D6b）会话头不再渲染状态属性；泊车事实由横幅与 rail 圆点表达。
    const head = el.shadowRoot!.querySelector<HTMLElement>('.session-head')
    expect(head!.getAttribute('data-status')).toBeNull()
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
    // D2：status 变化随增量响应同行（聚焦详情 slug 翻转，entries 只增不重拉）。
    expect(
      (el as unknown as { focusedDetail: { status_slug: string } }).focusedDetail.status_slug,
    ).toBe('done')
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
    // 序列与游标原样保留：transcript 还在、内容完整，不落「会话不可得」。
    const transcript = transcriptOf(el)!
    expect(transcript.entries.map((e) => e.position)).toEqual([0, 1, 2])
    expect(el.shadowRoot!.textContent).not.toContain('会话不可得')
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
 * mock 数据形状对齐 wire 契约：（2.1，D2）五键帧每次必带，turn_engaged
 * 不再有「只在 true 时上 wire」的缺省形态，消费面无字符串回退链——帧事实
 * 即时就地补丁（onWsEvent），refetch 只是收敛兜底。
 */
describe('sebas-dashboard (silent working window refetch chain)', () => {
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  function composerOf(el: SebasDashboard): StubWorkbenchComposer & {
    waitingApproval?: boolean
    turnInFlight?: boolean
    sessionKey?: string | null
  } {
    return el.shadowRoot!.querySelector('sebas-workbench-composer') as unknown as
      StubWorkbenchComposer & {
        waitingApproval?: boolean
        turnInFlight?: boolean
        sessionKey?: string | null
      }
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

    // 引擎在首个内容帧翻 WORKING 并发布 Updated：帧（2.1，D2 五键）携带
    // turn_engaged=true，refetch 链实时读回一致事实（stall 场景静默窗的
    // 稳态）——composer/pending-stack 翻转。
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: { ...focusedSummary().active_session!, turn_engaged: true },
    })
    apiMocks.session.mockResolvedValue({ ...detailFixture(), turn_engaged: true })
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_live%00',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 2,
      pending: [],
      label: null,
    })
    await settle(el)

    expect(composerOf(el).turnInFlight).toBe(true)
    expect(stackOf(el).turnEngaged).toBe(true)
    el.remove()
  })

  it('a turn.append frame updates the focused buffer directly and issues no refetch (3.2 dispatch)', async () => {
    // D4 分流：流式正文帧自带增量——dashboard 只把它并入聚焦会话缓冲，
    // 不发任何 HTTP 请求（旧实现每帧全量 refetch 的放大回路已拆除）。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), status_slug: 'working' })
    const el = await mount()
    await settle(el)
    apiMocks.session.mockClear()
    apiMocks.summary.mockClear()

    wsMocks.emit({
      type: 'turn.append',
      session_id: 'oc_live%00',
      entries: [
        { position: 3, kind: 'content', element_type: 'markdown', content: 'streamed live', created_at_unix: 1_700_000_300 },
      ],
      seq: 3,
    })
    await settle(el)

    // 缓冲就地推进：transcript 拿到 4 条（帧内条目并入），零请求。
    const transcript = el.shadowRoot!.querySelector(
      'sebas-transcript-view',
    ) as unknown as { entries: ConversationEntryView[] }
    expect(transcript.entries.map((e) => e.position)).toEqual([0, 1, 2, 3])
    expect(transcript.entries[3].content).toBe('streamed live')
    expect(apiMocks.session).not.toHaveBeenCalled()
    expect(apiMocks.summary).not.toHaveBeenCalled()
    el.remove()
  })

  it('the frame alone patches the focused detail synchronously (2.2, D2)', async () => {
    // 帧真源（2.2）：session.updated 五键帧到达的**同步**时刻，focusedDetail
    // 已被就地补丁（slug/turn_engaged/msg_count/pending）——composer 等表面
    // 的实时翻转不等下一次 HTTP 详情取回；随后的 refetch 只做收敛兜底。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), status_slug: 'queued' })
    const el = await mount()
    expect(composerOf(el).turnInFlight).toBe(false)

    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_live%00',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 3,
      pending: [{ id: 1, text: 'x', position: 0, disposition: 'turn', priority: false }],
      label: null,
    })

    const detail = (el as unknown as { focusedDetail: { status_slug: string; turn_engaged: boolean; msg_count: number; pending: unknown[] } }).focusedDetail
    expect(detail.status_slug).toBe('working')
    expect(detail.turn_engaged).toBe(true)
    expect(detail.msg_count).toBe(3)
    expect(detail.pending).toHaveLength(1)
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

describe('ws event dispatch, throttling and resync (fix-webui-streaming-liveness 3.2/5.3)', () => {
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
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

  it('session.* events coalesce into throttled list refreshes (≥500ms window)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const base = apiMocks.summary.mock.calls.length
    // 节流窗内的三个会话事件：只产生一轮刷新。（2.1，D2）相位帧键必带。
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_live%00',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 1,
      pending: [],
      label: null,
    })
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_live%00',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 1,
      pending: [],
      label: null,
    })
    wsMocks.emit({
      type: 'session.created',
      session_id: 'oc_new%00',
      status_slug: 'starting',
      turn_engaged: true,
      msg_count: 0,
      pending: [],
      label: null,
    })
    await settle(el)
    expect(apiMocks.summary.mock.calls.length).toBe(base + 1)
    // 窗口过后的下一事件触发新一轮（尾沿计时器已排定，≥500ms 后落地）。
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_live%00',
      status_slug: 'done',
      turn_engaged: false,
      msg_count: 1,
      pending: [],
      label: null,
    })
    await new Promise((r) => setTimeout(r, 650))
    await settle(el)
    expect(apiMocks.summary.mock.calls.length).toBe(base + 2)
    el.remove()
  })

  it('a stale detail response never paints over a newer fetch (fetchSeq guard)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    let releaseStale!: (d: SessionDetail) => void
    const stale = new Promise<SessionDetail>((resolve) => (releaseStale = resolve))
    // 首拉（代际 A）挂起；随后的 refetch（代际 B）立即返回含新条目的快照。
    apiMocks.session.mockImplementationOnce(() => stale)
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      entries: [
        ...detailFixture().entries,
        { position: 3, kind: 'content', element_type: 'markdown', content: 'from newer fetch', created_at_unix: 1_700_000_300 },
      ],
    })
    const el = await mount()
    await settle(el)
    window.dispatchEvent(new Event('sebas:refetch'))
    await settle(el)
    expect(transcriptOf(el)!.entries.map((e) => e.content)).toContain('from newer fetch')
    // 迟到的 A 响应（旧快照）到达：不得回退 B 的缓冲。
    releaseStale(detailFixture())
    await settle(el)
    const entries = transcriptOf(el)!.entries
    expect(entries).toHaveLength(4)
    expect(entries[3].content).toBe('from newer fetch')
    el.remove()
  })

  it('session.resync drops every cursor and refetches the focused session in full (5.3)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    // 先建立本地游标（增量一轮：0..2 → 3）。
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      entries: [
        { position: 3, kind: 'content', element_type: 'markdown', content: 'incremental', created_at_unix: 1_700_000_300 },
      ],
    })
    window.dispatchEvent(new Event('sebas:refetch'))
    await settle(el)
    let calls = apiMocks.session.mock.calls
    // 增量请求携带推进前的游标 2（响应并入后本地序列到 3）。
    expect(calls[calls.length - 1]).toEqual(['oc_live%00', 2])
    // resync：清全部游标 → 聚焦会话全量重取（无 entries_after）。
    wsMocks.emit({ type: 'session.resync' })
    await settle(el)
    calls = apiMocks.session.mock.calls
    expect(calls[calls.length - 1]).toEqual(['oc_live%00'])
    el.remove()
  })

  it('a recycled position with new content (core restart) resets the cursor and refetches full (5.3)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    // 本地游标在 2（0..2 全量）。
    // 重复帧（同 position 同内容）：去重丢弃，绝不触发重取。
    wsMocks.emit({
      type: 'turn.append',
      session_id: 'oc_live%00',
      entries: [
        { position: 2, kind: 'content', element_type: 'markdown', content: 'second entry', created_at_unix: 1_700_000_200 },
      ],
      seq: 2,
    })
    await settle(el)
    expect(apiMocks.session).toHaveBeenCalledTimes(1)
    // 世代回绕（core 重启后 position 从 0 重计、内容不同）：单调性矛盾 →
    // 清缓冲、游标置空、全量重取。
    wsMocks.emit({
      type: 'turn.append',
      session_id: 'oc_live%00',
      entries: [
        { position: 0, kind: 'content', element_type: 'markdown', content: 'REBUILT', created_at_unix: 1_700_009_999 },
      ],
      seq: 0,
    })
    await settle(el)
    const calls = apiMocks.session.mock.calls
    // 单调性矛盾必须强制全量重取。
    expect(calls[calls.length - 1]).toEqual(['oc_live%00'])
    el.remove()
  })

  it('a turn.append for a non-focused session neither buffers nor refetches (3.2)', async () => {
    apiMocks.summary.mockResolvedValue(summaryBase)
    apiMocks.session.mockResolvedValue(detailFixture())
    const el = await mount()
    await settle(el)
    apiMocks.session.mockClear()
    wsMocks.emit({
      type: 'turn.append',
      session_id: 'oc_other%00',
      entries: [
        { position: 9, kind: 'content', element_type: 'markdown', content: 'elsewhere', created_at_unix: 1 },
      ],
      seq: 9,
    })
    await settle(el)
    expect(apiMocks.session).not.toHaveBeenCalled()
    el.remove()
  })
})

// ── polish-workbench-walkthrough-ux：归档只读视图 / 恢复反馈 / 崩溃一致性 ──

import { resetNotices, subscribeNotices, type NoticeItem } from '../notify.js'
import type { ArchiveDetail } from '../api/client.js'

describe('archived view + restore semantics (polish-workbench-walkthrough-ux 2.1–2.4)', () => {
  /** 收敛一轮 refetch 链（与既有 describe 内 helper 同款）。 */
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  const archiveEntry = {
    session_key: 'oc_arch%00',
    project_path: '/home/me/archived-proj',
    label: 'archived chat',
    archived_at: 1_700_000_000,
    retention_deadline: 1_900_000_000,
    transcript: [],
  }
  const archiveDetail: ArchiveDetail = {
    entry: archiveEntry,
    entries: [
      { position: 0, kind: 'prompt', element_type: 'markdown', content: 'old question', created_at_unix: 1 },
      { position: 1, kind: 'content', element_type: 'markdown', content: 'old answer', created_at_unix: 2 },
    ],
  }

  async function mountArchived(): Promise<SebasDashboard> {
    apiMocks.archiveDetail.mockResolvedValue(archiveDetail)
    apiMocks.summary.mockResolvedValue(summaryBase)
    const el = await mount()
    // app-shell 接线（恢复成功派发 archive-view-close → shell 清 archivedEntry）：
    // 单测无 shell，这里用监听器还原同一契约。
    el.addEventListener('archive-view-close', () => {
      el.archivedEntry = null
    })
    el.archivedEntry = archiveEntry
    await settle(el)
    return el
  }

  it('renders a read-only archived view: restore button, project path, transcript, no composer', async () => {
    const el = await mountArchived()
    expect(el.shadowRoot!.querySelector('[data-testid="archived-view"]')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('[data-testid="archived-restore"]')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('[data-testid="archived-project"]')?.textContent).toContain(
      '/home/me/archived-proj',
    )
    // 对话快照可见（只读回看）：transcript-view 在自身 shadow root 内渲染，
    // 直接读组件的 entries 属性断言快照内容（与既有 transcriptOf 同款）。
    const t = el.shadowRoot!.querySelector<HTMLElement & { entries: ConversationEntryView[] }>(
      'sebas-transcript-view',
    )
    expect(t?.entries.map((e) => e.content)).toEqual(['old question', 'old answer'])
    // 只读态：composer 不在场（后端 400 消息门兜底不变）。
    expect(el.shadowRoot!.querySelector('sebas-workbench-composer')).toBeNull()
    el.remove()
  })

  it('restore goes through a confirm dialog stating the project path, then succeeds with a toast', async () => {
    apiMocks.restoreSession.mockResolvedValue({ status: 'restored', entry: archiveEntry })
    const notices: NoticeItem[] = []
    const unsubscribe = subscribeNotices((st) => {
      notices.splice(0, notices.length, ...st.items)
    })
    const el = await mountArchived()
    // 点击 = 只读查看；未确认前绝不触发 restore。
    expect(apiMocks.restoreSession).not.toHaveBeenCalled()
    ;(el.shadowRoot!.querySelector('[data-testid="archived-restore"]') as HTMLElement).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('[data-testid="restore-dialog"]')
    expect(dialog).toBeTruthy()
    expect(dialog!.textContent).toContain('/home/me/archived-proj')
    // fix-webui-qa-defects 2.3：恢复语义如实前置——「将重建会话并保留对话
    // 记录」，条数随对话快照可见。
    const note = el.shadowRoot!.querySelector('[data-testid="restore-rebuild-note"]')
    expect(note?.textContent).toContain('将重建会话并保留对话记录')
    expect(note?.textContent).toContain('2 条消息')
    ;(el.shadowRoot!.querySelector('[data-testid="restore-confirm"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await settle(el)
    expect(apiMocks.restoreSession).toHaveBeenCalledWith('oc_arch%00')
    // 成功 toast 携带落点 project_path（未注册项目也不静默，2.3）。
    const toast = notices.find((n) => n.message.includes('已恢复到'))
    expect(toast?.message).toContain('/home/me/archived-proj')
    // 恢复成功 = 只读视图退出。
    expect(el.shadowRoot!.querySelector('[data-testid="archived-view"]')).toBeNull()
    unsubscribe()
    resetNotices()
    el.remove()
  })

  it('a failed restore surfaces an error toast and keeps the entry archived in place', async () => {
    apiMocks.restoreSession.mockRejectedValue(new Error('HTTP 500: boom'))
    const notices: NoticeItem[] = []
    const unsubscribe = subscribeNotices((st) => {
      notices.splice(0, notices.length, ...st.items)
    })
    const el = await mountArchived()
    ;(el.shadowRoot!.querySelector('[data-testid="archived-restore"]') as HTMLElement).click()
    await el.updateComplete
    ;(el.shadowRoot!.querySelector('[data-testid="restore-confirm"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 失败通知点名会话与成因；条目仍在（归档视图不退场）。
    const toast = notices.find((n) => n.message.includes('恢复会话'))
    expect(toast?.message).toContain('archived chat')
    expect(toast?.message).toContain('boom')
    expect(el.shadowRoot!.querySelector('[data-testid="archived-view"]')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('[data-testid="restore-error"]')).toBeTruthy()
    unsubscribe()
    resetNotices()
    el.remove()
  })
})

describe('focused session termination consistency (polish-workbench-walkthrough-ux 3.5)', () => {
  /** 收敛一轮 refetch 链（与既有 describe 内 helper 同款）。 */
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  it('a session.removed for the focused session exits Working, notifies, keeps the transcript', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    expect(el.shadowRoot!.querySelector('sebas-workbench-composer')).toBeTruthy()
    const notices: NoticeItem[] = []
    const unsubscribe = subscribeNotices((st) => {
      notices.splice(0, notices.length, ...st.items)
    })
    wsMocks.emit({ type: 'session.removed', session_id: 'oc_live%00' })
    await settle(el)
    // Working 幽灵退出：composer（含停止控件）让位给终止说明。
    expect(el.shadowRoot!.querySelector('sebas-workbench-composer')).toBeNull()
    expect(el.shadowRoot!.querySelector('[data-testid="composer-terminated"]')).toBeTruthy()
    // 通知点名会话与成因；已收对话保持只读可见（transcript-view 的 shadow
    // 内渲染，读组件 entries 断言快照仍在）。
    const toast = notices.find((n) => n.message.includes('chat-live'))
    expect(toast?.message).toContain('已终止')
    const t = el.shadowRoot!.querySelector<HTMLElement & { entries: ConversationEntryView[] }>(
      'sebas-transcript-view',
    )
    expect(t?.entries.some((e) => e.content === 'first entry')).toBe(true)
    unsubscribe()
    resetNotices()
    el.remove()
  })
})


// ── fix-webui-qa-defects 4.1：rail 切换的即时聚焦 ─────────────────────────

describe('rail focus follow (fix-webui-qa-defects 4.1, design D3)', () => {
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  it('a rail focus event triggers a list refresh without any WS traffic', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    const callsBefore = apiMocks.summary.mock.calls.length

    window.dispatchEvent(new CustomEvent(RAIL_FOCUS_EVENT, { detail: { key: 'oc_live%00' } }))
    // 节流窗（500ms）内首事件立即触发；给 setTimeout(0) 一拍。
    await new Promise((r) => setTimeout(r, 10))
    await settle(el)

    expect(apiMocks.summary.mock.calls.length).toBeGreaterThan(callsBefore)
    el.remove()
  })

  it('repeated rail focus events for the same session stay idempotent', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const el = await mount()
    await settle(el)
    for (let i = 0; i < 3; i++) {
      window.dispatchEvent(new CustomEvent(RAIL_FOCUS_EVENT, { detail: { key: 'oc_live%00' } }))
    }
    // 节流窗口内三次合并为一轮刷新（首事件立即，其余并入尾沿）。
    await new Promise((r) => setTimeout(r, 10))
    await settle(el)
    await new Promise((r) => setTimeout(r, 550))
    const calls = apiMocks.summary.mock.calls.length
    await new Promise((r) => setTimeout(r, 550))
    expect(apiMocks.summary.mock.calls.length).toBe(calls)
    el.remove()
  })
})

// ── fix-webui-qa-defects 7.3：终止通知使用 rail 同款可读会话名 ─────────────

describe('termination notice naming (fix-webui-qa-defects 7.3)', () => {
  async function settle(el: SebasDashboard): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  it('uses the rail label (prompt preview) instead of the raw web- key', async () => {
    // summary 里该会话行带 prompt_preview——通知必须用它，而非 chat_id /
    // 原始键「web-1789…-1」。
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      recent_sessions: [
        row({
          encoded_key: 'oc_live%00',
          chat_id: 'web-1789abcde-1',
          prompt_preview: 'fix the login bug',
          session_id_short: 'aaaa0009',
        }),
      ],
    })
    const el = await mount()
    await settle(el)
    const notices: NoticeItem[] = []
    const unsubscribe = subscribeNotices((st) => {
      notices.splice(0, notices.length, ...st.items)
    })
    wsMocks.emit({ type: 'session.removed', session_id: 'oc_live%00' })
    await settle(el)
    const toast = notices.find((n) => n.message.includes('已终止'))
    expect(toast, 'a termination notice must fire').toBeTruthy()
    expect(toast?.message).toContain('fix the login bug')
    expect(toast?.message).not.toContain('web-1789abcde-1')
    unsubscribe()
    resetNotices()
    el.remove()
  })

  it('falls back through the rail chain (short id) when no preview exists', async () => {
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      recent_sessions: [
        row({
          encoded_key: 'oc_live%00',
          chat_id: 'web-1789abcde-1',
          prompt_preview: null,
          session_id_short: 'aaaa0009',
        }),
      ],
    })
    const el = await mount()
    await settle(el)
    const notices: NoticeItem[] = []
    const unsubscribe = subscribeNotices((st) => {
      notices.splice(0, notices.length, ...st.items)
    })
    wsMocks.emit({ type: 'session.removed', session_id: 'oc_live%00' })
    await settle(el)
    const toast = notices.find((n) => n.message.includes('已终止'))
    expect(toast?.message).toContain('aaaa0009')
    expect(toast?.message).not.toContain('web-1789abcde-1')
    unsubscribe()
    resetNotices()
    el.remove()
  })
})

describe('slash command palette stacking (polish-workbench-walkthrough-ux 2.5)', () => {
  it('composer-area does not clip the palette: overflow is visible, not auto', () => {
    // 根因：面板 DOM 与 sessionCommands 数据都在，但 .composer-area 的
    // overflow-y:auto 把越出 composer 壳顶的面板裁到仅剩 ~2px 缝。CSS 修复
    // = 列级 overflow 可见（浮层探出），审批卡/堆叠区各自限高。断言样式表
    // 里对应规则，防回归。
    const styles = DashboardImpl.styles
    const css = (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
    const areaRule = css.match(/\.composer-area\s*\{[^}]*\}/)?.[0] ?? ''
    expect(areaRule).toContain('overflow: visible')
    expect(areaRule).not.toContain('overflow-y: auto')
  })
})

// ── fix-webui-approval-restore-and-session-identity ──────────────────────

describe('focused session drives project context (4.1)', () => {
  it('focusedProjectPath resolves by project id and misses honestly', () => {
    const projects = [
      { id: 'proj-sebas', path: '/home/me/sebas' },
      { id: 'proj-x', path: '/home/me/x' },
    ]
    expect(focusedProjectPath(projects, 'proj-x')).toBe('/home/me/x')
    expect(focusedProjectPath(projects, 'proj-ghost')).toBeNull()
    expect(focusedProjectPath(projects, null)).toBeNull()
  })

  it('a focus change projects the session project onto shell selectedPath (4.1)', async () => {
    apiMocks.summary.mockResolvedValue(focusedSummary())
    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener(PROJECT_FOLLOW_EVENT, listener)
    try {
      const el = await mount()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      const follow = events.find((e) => (e.detail as { path?: string })?.path === '/home/me/sebas')
      expect(follow, 'the focused session must project its project path').toBeTruthy()
      // 同一聚焦 key 的重复刷新不再派发（幂等）。
      const before = events.length
      await (el as any).refreshLists()
      await new Promise((r) => setTimeout(r, 0))
      expect(events.length).toBe(before)
      el.remove()
    } finally {
      window.removeEventListener(PROJECT_FOLLOW_EVENT, listener)
    }
  })

  it('a session whose project is not registered projects nothing (4.1 未命中)', async () => {
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: { ...focusedSummary().active_session!, project_id: 'proj-ghost' },
    })
    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener(PROJECT_FOLLOW_EVENT, listener)
    try {
      const el = await mount()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(
        events.find((e) => (e.detail as { path?: string })?.path !== undefined),
        'an unregistered project must keep 未选择项目',
      ).toBeUndefined()
      el.remove()
    } finally {
      window.removeEventListener(PROJECT_FOLLOW_EVENT, listener)
    }
  })
})

describe('session header agent identity (3.3)', () => {
  it('the restored identity rides the detail into the header; legacy falls back honestly', async () => {
    // 恢复带身份：detail.agent_kind 原样上头（来源 = SessionInfo 身份四项）。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), agent_kind: 'codex' })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(
      el.shadowRoot!.querySelector('[data-testid="agent-lock"]')?.textContent,
    ).toContain('codex')
    el.remove()

    // 旧归档条目（无身份字段）：header 如实回退 "default agent"，不编造。
    apiMocks.session.mockResolvedValue({ ...detailFixture(), agent_kind: null })
    const el2 = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el2.updateComplete
    expect(
      el2.shadowRoot!.querySelector('[data-testid="agent-lock"]')?.textContent,
    ).toContain('default agent')
    el2.remove()
  })
})

// ── fix-webui-qa-defects-round3：焦点处立读锚（3.1）+ 深链项目标题（4.2）──

describe('focus establishes the read anchor (round3 3.1)', () => {
  const KEY = 'oc_live%00'
  const anchorKey = `sebas:seen:${KEY}`

  beforeEach(() => {
    localStorage.removeItem(anchorKey)
  })
  afterEach(() => {
    localStorage.removeItem(anchorKey)
  })

  it('a session focused without a rail click gets its anchor at detail arrival', async () => {
    // 创建 set_focus / 深链 / 恢复聚焦走进焦点的会话此前永远没有本地锚——
    // 无锚 = fully read，之后的非聚焦新回复推不出未读徽章（QA 缺陷 3）。
    // 详情到达时按真实段数立锚。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue(detailFixture()) // msg_count: 2
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(JSON.parse(localStorage.getItem(anchorKey)!)).toEqual({ anchor_count: 2 })
    el.remove()
  })

  it('an existing anchor is never overwritten by the establishment pass', async () => {
    // 锚已存在（rail 点击建立 / 流式推进）：立锚不回写，单调归游标模块。
    writeFocusAnchor(KEY, 1)
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue(detailFixture())
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(JSON.parse(localStorage.getItem(anchorKey)!)).toEqual({ anchor_count: 1 })
    el.remove()
  })

  it('a detail load in a hidden tab does not establish the anchor', async () => {
    // design 决策 4 的「看着」边界：后台 tab 里的装载不算看着——锚留空，
    // 回到页面的下一次详情装载再立。否则后台轮询到的段数会把没看过的内容
    // 全部标成已读，未读徽章在另一个 tab 里静默丢失。
    Object.defineProperty(document, 'visibilityState', {
      value: 'hidden',
      configurable: true,
    })
    try {
      apiMocks.summary.mockResolvedValue(focusedSummary())
      apiMocks.session.mockResolvedValue(detailFixture())
      const el = await mount()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(localStorage.getItem(anchorKey)).toBeNull()
      el.remove()
    } finally {
      // 摘掉实例遮蔽，恢复 Document.prototype 上的原生 getter。
      delete (document as unknown as { visibilityState?: string }).visibilityState
    }
  })
})

describe('fresh placeholder first exchange never badges (round3 6.1)', () => {
  const KEY = 'oc_live%00'
  const anchorKey = `sebas:seen:${KEY}`

  beforeEach(() => {
    localStorage.removeItem(anchorKey)
  })
  afterEach(() => {
    localStorage.removeItem(anchorKey)
  })

  it('the empty-state placeholder registers the session so its first snapshot exchange establishes the anchor', async () => {
    // QA round5 复现链路：创建即聚焦的占位会话（detail 0 回合）渲染的是
    // dashboard 自己的空态占位——transcript 未挂载，组件内的空流登记永不
    // 执行；首交换经快照到达后 transcript 才挂载，锚推不动，徽标 + 「~1
    // new」缝驻留。loadFocused 渲染空态即登记（registerEmptyStreamSession），
    // 首交换按「看着到达」消费：锚从空流建立，不再闪缝。
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      msg_count: 0,
      entries: [],
      status_slug: 'done',
      status: 'done',
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 空态占位在场：transcript 未挂载；立锚一拍写入 0（创建基线）。
    expect(el.shadowRoot!.querySelector('.empty-stream')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('sebas-transcript-view')).toBeNull()
    expect(JSON.parse(localStorage.getItem(anchorKey)!)).toEqual({ anchor_count: 0 })

    // 首交换经快照到达（prompt + 回复，msg_count 0→2），sebas:refetch 驱动
    // 详情重取——transcript 随内容挂载并消费空流登记。
    apiMocks.session.mockResolvedValue(detailFixture()) // msg_count: 2, entries: 3
    window.dispatchEvent(new Event('sebas:refetch'))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const transcript = el.shadowRoot!.querySelector('sebas-transcript-view')
    expect(transcript).toBeTruthy()
    await new Promise((r) => setTimeout(r, 300)) // 盖过 MARK_SEEN_DEBOUNCE_MS
    await el.updateComplete
    // 锚 = max(服务端段数 2, 本地已渲染 2) = 2：首交换不产生 seam/徽标水位。
    expect(JSON.parse(localStorage.getItem(anchorKey)!)).toEqual({ anchor_count: 2 })
    const seam = (transcript as unknown as { shadowRoot: ShadowRoot }).shadowRoot.querySelector('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })
})

describe('fresh placeholder first exchange via live streaming (fix-unread-fresh-exchange)', () => {
  // GUI 真实时序的 dashboard 级钉（session-unread-badge「first focused
  // exchange of a fresh placeholder」）：创建写锚 0 → 空详情（登记）→
  // composer 提交后的乐观重取只含 [prompt]（transcript 以 0 可见段挂载）
  // → 回复经 turn.append 到达。锚必须在回复的到达帧**同步**推进——徽章水位
  // （msg_count − 锚）恒 0、seam 从未画出——不依赖 250ms 防抖窗或后续重取。
  const KEY = 'oc_live%00'
  const anchorKey = `sebas:seen:${KEY}`

  beforeEach(() => {
    localStorage.removeItem(anchorKey)
  })
  afterEach(() => {
    localStorage.removeItem(anchorKey)
  })

  it('prompt-only 挂载后回复经 turn.append 到达：锚即时推进，seam 不画', async () => {
    // rail confirmNewSession 的创建即写锚 0（「创建即见过」基线）。
    writeFocusAnchor(KEY, 0)
    apiMocks.summary.mockResolvedValue(focusedSummary())
    apiMocks.session.mockResolvedValue({ ...detailFixture(), msg_count: 0, entries: [] })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 空态在场：transcript 未挂载，dashboard 侧空流登记已完成。
    expect(el.shadowRoot!.querySelector('.empty-stream')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('sebas-transcript-view')).toBeNull()

    // composer 提交完成（composer-sent → loadFocused）：此刻服务端只有 prompt。
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      msg_count: 0,
      turn_engaged: true,
      entries: [
        {
          position: 0,
          kind: 'prompt',
          element_type: 'markdown',
          content: 'hello',
          created_at_unix: 1_700_000_000,
        },
      ],
    })
    ;(el as unknown as { onComposerSent: () => void }).onComposerSent()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const transcript = el.shadowRoot!.querySelector('sebas-transcript-view')
    expect(transcript).toBeTruthy()

    // 回复流式到达（turn.append：dashboard 合并与 transcript 自身订阅同帧）。
    wsMocks.emit({
      type: 'turn.append',
      session_id: KEY,
      seq: 2,
      entries: [
        {
          position: 1,
          kind: 'content',
          element_type: 'thinking',
          content: 'hmm',
          created_at_unix: 1_700_000_100,
        },
        {
          position: 2,
          kind: 'content',
          element_type: 'markdown',
          content: 'hello world',
          created_at_unix: 1_700_000_100,
        },
      ],
    })
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 到达帧同步结算：锚 = 已渲染段数 1（= msg_count，徽章水位 0）。
    expect(JSON.parse(localStorage.getItem(anchorKey)!)).toEqual({ anchor_count: 1 })
    const seam = (transcript as unknown as { shadowRoot: ShadowRoot }).shadowRoot.querySelector('.seam')
    expect(seam?.hasAttribute('hidden')).toBe(true)
    el.remove()
  })
})

describe('review-cards phase wiring (round3 2.2)', () => {
  it('the focused session phase is passed to sebas-review-cards for reconciliation', async () => {
    // 相位对账的另一半在 dashboard：卡片组件自己不订阅相位帧，靠这里把
    // 聚焦会话的 status_slug 下传。绑定一旦脱落，丢帧自愈就静默失效——
    // 钉住 dashboard → 卡片的接线。
    apiMocks.summary.mockResolvedValue({
      ...focusedSummary(),
      active_session: {
        ...focusedSummary().active_session!,
        status: 'waiting',
        status_label: 'Waiting',
        status_slug: 'waiting',
      },
    })
    apiMocks.session.mockResolvedValue({
      ...detailFixture(),
      status: 'waiting',
      status_label: 'Waiting',
      status_slug: 'waiting',
    })
    const el = await mount()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const review = el.shadowRoot!.querySelector('sebas-review-cards') as unknown as {
      sessionKey: string | null
      sessionPhase: string | null
    }
    expect(review).toBeTruthy()
    expect(review.sessionKey).toBe('oc_live%00')
    expect(review.sessionPhase).toBe('waiting')
    el.remove()
  })
})

describe('deep link binds the project title (round3 4.2)', () => {
  it('a /sessions/:key deep link projects the session project before summary catches up', async () => {
    // 深链窗口：summary 的焦点指针尚未落位（active_session 为空，读 detail
    // 才设服务端焦点）——此前归属解析只能等下一次无关刷新，主区标题一直
    // 「未选择项目」。detail 到达后的核对一拍经 focusedDetail.project_id
    // 完成投影。
    apiMocks.summary.mockResolvedValue(summaryBase) // active_session: null
    apiMocks.session.mockResolvedValue({ ...detailFixture(), project_id: 'proj-sebas' })

    const el = document.createElement('sebas-dashboard') as SebasDashboard
    el.deepLinkKey = 'oc_live%00'
    document.body.appendChild(el)

    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener(PROJECT_FOLLOW_EVENT, listener)
    try {
      await el.updateComplete
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      const follow = events.find(
        (e) => (e.detail as { path?: string })?.path === '/home/me/sebas',
      )
      expect(follow, 'deep link must bind the project context immediately').toBeTruthy()
    } finally {
      window.removeEventListener(PROJECT_FOLLOW_EVENT, listener)
      el.remove()
    }
  })

  it('projects the session project when the project list lands after the detail (deep-link race)', async () => {
    // 4.2 修订的时序复现：深链刷新下 detail 常先于 projects.list 落地——
    // 旧实现以 path=null 空转一次即记账（lastFollowedFocusKey），列表到达
    // 后的核对一拍全部早退，标题永远「未选择项目」。修订后「项目 id 有值
    // 而路径未解析」不记账，列表到达的 refetch 核对补上投影。
    let resolveProjects!: (v: unknown) => void
    apiMocks.projectsList.mockReturnValue(
      new Promise((resolve) => {
        resolveProjects = resolve
      }),
    )
    apiMocks.summary.mockResolvedValue(summaryBase) // active_session: null
    apiMocks.session.mockResolvedValue({ ...detailFixture(), project_id: 'proj-sebas' })

    const el = document.createElement('sebas-dashboard') as SebasDashboard
    el.deepLinkKey = 'oc_live%00'
    document.body.appendChild(el)

    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener(PROJECT_FOLLOW_EVENT, listener)
    try {
      await el.updateComplete
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      // detail 已到而项目列表未到：不投影，也不记账。
      expect(
        events.find((e) => (e.detail as { path?: string })?.path === '/home/me/sebas'),
      ).toBeUndefined()
      resolveProjects({
        projects: [{ id: 'proj-sebas', path: '/home/me/sebas', name: 'sebas', added_at: 0 }],
      })
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(
        events.find((e) => (e.detail as { path?: string })?.path === '/home/me/sebas'),
        'the late project list must still bind the project context',
      ).toBeTruthy()
    } finally {
      window.removeEventListener(PROJECT_FOLLOW_EVENT, listener)
      el.remove()
    }
  })
})
