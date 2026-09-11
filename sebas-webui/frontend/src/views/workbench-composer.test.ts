// @vitest-environment jsdom
/**
 * Workbench composer behaviour. Mount the real LitElement with the api
 * client mocked so we can drive the four core paths:
 *   - unreachable gate disables submit and surfaces the cause
 *   - submit forwards (text, projectDir) and emits composer-created
 *   - empty text is a no-op
 *   - submit failure surfaces the error inline and preserves the text
 *
 * jsdom's ElementInternals shim is incomplete: it lacks `setFormValue`,
 * `setValidity`, etc., which the Web Awesome form-associated components
 * call in their update lifecycle. Without a polyfill the WA elements
 * throw during update and prevent the host LitElement from finishing
 * its render. We patch `Element.prototype.attachInternals` to wrap the
 * returned object with the missing no-op methods; WA's calls become
 * harmless and the composer renders normally.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasPendingStack } from '../components/pending-stack.js'
import '../components/pending-stack.js'
import type { SebasWorkbenchComposer } from './workbench-composer.js'
import type { Summary } from '../api/client.js'
import {
  elementInternalsPolyfillInvoked,
  installWaDomPolyfills,
} from '../test-support/wa-polyfills.js'

// ---- WA 渲染垫片（共享）--------------------------------------------------
// 包装 attachInternals 等 jsdom 缺失的 DOM API，让 WA 组件走到稳定状态以便
// 模板渲染。实现见 test-support/wa-polyfills.ts（幂等安装）。
installWaDomPolyfills()

// jsdom does not implement ResizeObserver; WA's textarea reaches for it
// in `updated` to track auto-resize. A no-op implementation is enough
// for the composer tests — the textarea only needs to render, not to
// react to resize events.
if (typeof (globalThis as { ResizeObserver?: unknown }).ResizeObserver === 'undefined') {
  class StubResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof StubResizeObserver }).ResizeObserver =
    StubResizeObserver
}

// ---- api client mock ----------------------------------------------------

vi.mock('../api/client.js', () => ({
  api: {
    summary: vi.fn(),
    settings: vi.fn(),
    sessions: vi.fn(),
    createSession: vi.fn(),
    agents: vi.fn(),
    sendMessage: vi.fn(),
    setSessionModel: vi.fn(),
    agentDefaults: vi.fn(),
    routerProviders: vi.fn(),
    routerDefaults: vi.fn(),
  },
}))

// Import the composer module now that the api is mocked and the
// ElementInternals polyfill is in place. The WA module side-effects
// (registering wa-textarea/wa-select/wa-option) and the @customElement decorator
// on SebasWorkbenchComposer both run at this point.
import './workbench-composer.js'

import { api } from '../api/client.js'

const summaryReachable: Summary = {
  active_count: 0,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: 0,
  uptime: '0s',
  recent_sessions: [],
  active_session: null,
  active_session_key: null,
  reachability: { ok: true },
}

const summaryUnreachable: Summary = {
  ...summaryReachable,
  reachability: { ok: false, cause: 'router down' },
}

// wire-webui-sebas-agent-e2e 4.1：逐执行体可用性——native 缺凭据时后端下拉
// 中该选项禁选并标注 cause。
const summaryNativeUnavailable: Summary = {
  ...summaryReachable,
  execution_bodies: [
    { name: 'acp', ok: true, cause: null },
    { name: 'native', ok: false, cause: 'no provider credentials' },
  ],
}

const summaryNativeAvailable: Summary = {
  ...summaryReachable,
  execution_bodies: [
    { name: 'acp', ok: true, cause: null },
    { name: 'native', ok: true, cause: null },
  ],
}

async function mount(initial: Partial<SebasWorkbenchComposer> = {}) {
  const el = document.createElement('sebas-workbench-composer') as SebasWorkbenchComposer
  if (initial.projectId !== undefined) el.projectId = initial.projectId
  if (initial.projectDir !== undefined) el.projectDir = initial.projectDir
  if (initial.projectDefaultAgent !== undefined) el.projectDefaultAgent = initial.projectDefaultAgent
  if (initial.providerLabel !== undefined) el.providerLabel = initial.providerLabel
  if (initial.sessionKey !== undefined) el.sessionKey = initial.sessionKey
  if (initial.agentKind !== undefined) el.agentKind = initial.agentKind
  if (initial.sessionModels !== undefined) el.sessionModels = initial.sessionModels
  if (initial.currentModel !== undefined) el.currentModel = initial.currentModel
  document.body.appendChild(el)
  // LitElement schedules its first update asynchronously; then the
  // composer kicks off an async reachability fetch in connectedCallback.
  // We need to let the WA shadow children upgrade and render before
  // querying them. A couple of microtask drains and a fresh updateComplete
  // cycle is enough.
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

beforeEach(() => {
  vi.clearAllMocks()
  ;(api.settings as ReturnType<typeof vi.fn>).mockResolvedValue({
    card_config: {
      theme_color: '#000',
      fold_long_output: false,
      thinking_display: 'auto',
      max_user_text_chars: 0,
      max_tool_output_chars: 0,
    },
    router: {
      listen: null,
      provider_count: 0,
      debug: false,
      has_auth: false,
      providers: [],
    },
  })
  ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
    agents: [
      { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
      { id: 'gemini', display: 'Gemini', reachable: true, version: 'v2' },
      { id: 'codex', display: 'Codex', reachable: false, cause: 'command not found' },
      { id: 'native', display: 'Native Kernel', reachable: true },
    ],
  })
  ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
    recent_sessions: [],
    active_count: 0,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 0,
    active_session_key: null,
  })
  // 创建模式的 Settings 目录（workbench-conversation-view 4.2/4.3）：默认
  // 空目录（model 预选保持 null，供既有提交用例稳定）；两级选择用例按需覆写。
  ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
  ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
    default_provider: null,
    default_model: null,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
  if (!elementInternalsPolyfillInvoked()) {
    // Sanity-check: the polyfill must have been invoked at least once
    // (otherwise WA's form-associated internals still point at the raw
    // jsdom object and tests would silently lose their polyfill).
    throw new Error('ElementInternals polyfill was never invoked')
  }
})

describe('sebas-workbench-composer', () => {
  it('renders disabled with cause when reachability is unreachable', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryUnreachable)
    const el = await mount({ projectDir: null })
    const textarea = el.shadowRoot?.querySelector('wa-textarea')
    expect(textarea?.hasAttribute('disabled')).toBe(true)
    const callout = el.shadowRoot?.querySelector<HTMLElement>('.callout-warning')
    expect(callout?.textContent ?? '').toContain('router down')
    expect(callout?.textContent ?? '').toContain('core not connected')
  })


  it('gates submit when the summary poll itself fails (server unreachable)', async () => {
    // add-webui-allowed-roots D6：summary 请求失败 = 服务不可达，与
    // reachability.ok=false 同款禁用提交门（轮询恢复后自动解除）。
    ;(api.summary as ReturnType<typeof vi.fn>).mockRejectedValue(new TypeError('Failed to fetch'))
    const el = await mount({ projectDir: null })
    const textarea = el.shadowRoot?.querySelector('wa-textarea')
    expect(textarea?.hasAttribute('disabled')).toBe(true)
    const callout = el.shadowRoot?.querySelector<HTMLElement>('.callout-warning')
    expect(callout?.textContent ?? '').toContain('无法获取服务状态')
  })

  it('creation mode offers the two-level Settings catalog and forwards the picked model (4.3)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'deepseek',
          base_url_anthropic: null,
          base_url_openai_chat: 'https://api.deepseek.example',
          base_url_openai_responses: null,
          api_key_env: 'DEEPSEEK_API_KEY',
          api_key_configured: true,
          models: [{ id: 'deepseek-chat', tags: [] }, { id: 'deepseek-reasoner', tags: [] }],
        },
        {
          name: 'anthropic',
          base_url_anthropic: 'https://api.anthropic.example',
          base_url_openai_chat: null,
          base_url_openai_responses: null,
          api_key_env: 'ANTHROPIC_API_KEY',
          api_key_configured: true,
          models: [{ id: 'claude-sonnet', tags: ['vision'] }],
        },
      ],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'deepseek',
      default_model: 'deepseek-reasoner',
    })
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_m' })
    const el = await mount({ projectDir: null })

    // 一级 = provider，默认预选配置的 default provider。
    const providerSel = el.shadowRoot?.querySelector(
      'wa-select[aria-label="Provider"]',
    ) as unknown as HTMLElement & { value: string }
    expect(providerSel).toBeTruthy()
    expect(providerSel.value).toBe('deepseek')
    // 二级 = model，默认预选配置的 default model。
    const modelSel = el.shadowRoot?.querySelector(
      'wa-select[aria-label="Model"]',
    ) as unknown as HTMLElement & { value: string }
    expect(modelSel.value).toBe('deepseek-reasoner')

    // 两级联动：切 provider → model 列表换到该 provider、预选其首项。
    providerSel.value = 'anthropic'
    providerSel.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(modelSel.value).toBe('claude-sonnet')

    // 提交把选定的 model 一并下发。
    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'use this model'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete
    ;(el.shadowRoot?.querySelector('.send-button') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'use this model',
      projectId: null,
      agent: expect.any(String),
      model: 'claude-sonnet',
    })
  })

  it('a configured default outside the catalog cannot preselect (no fabricated options, 4.3)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'alpha',
          base_url_anthropic: null,
          base_url_openai_chat: null,
          base_url_openai_responses: null,
          api_key_env: null,
          api_key_configured: true,
          models: [{ id: 'a1', tags: [] }],
        },
      ],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'ghost',
      default_model: 'nope',
    })
    const el = await mount({ projectDir: null })
    const providerSel = el.shadowRoot?.querySelector(
      'wa-select[aria-label="Provider"]',
    ) as unknown as HTMLElement & { value: string }
    // 目录里只有 alpha：预选落到目录第一项，绝不伪造 ghost 选项。
    expect(providerSel.value).toBe('alpha')
    const providerOptions = [
      ...el.shadowRoot!.querySelectorAll('wa-select[aria-label="Provider"] wa-option'),
    ].map((o) => o.getAttribute('value'))
    expect(providerOptions).toEqual(['alpha'])
    const modelSel = el.shadowRoot?.querySelector(
      'wa-select[aria-label="Model"]',
    ) as unknown as HTMLElement & { value: string }
    expect(modelSel.value).toBe('a1')
  })

  it('follow mode: no session models means no dropdown and no error (D8)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    const el = await mount({
      sessionKey: 'web%00web-1',
      agentKind: 'claude',
      sessionModels: [],
      currentModel: null,
    })
    // 会话内不给模型下拉（acp-model-selection 语义），也不显示目录不可用
    // （跟随模式与创建目录正交）。
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Model"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="catalog-unavailable"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Provider"]')).toBeNull()
  })

  it('existing sessions keep session-sourced options, not the catalog (D8)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    // 目录里只有 deepseek，但会话自己的模型面才是权威。
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'deepseek',
          base_url_anthropic: null,
          base_url_openai_chat: null,
          base_url_openai_responses: null,
          api_key_env: null,
          api_key_configured: true,
          models: [{ id: 'deepseek-chat', tags: [] }],
        },
      ],
    })
    const el = await mount({
      sessionKey: 'web%00web-1',
      agentKind: 'claude',
      sessionModels: ['sonnet', 'haiku'],
      currentModel: 'sonnet',
    })
    const sel = el.shadowRoot?.querySelector(
      'wa-select[aria-label="Model"]',
    ) as unknown as HTMLElement & { value: string }
    expect(sel).toBeTruthy()
    expect(sel.value).toBe('sonnet')
    const options = [...el.shadowRoot!.querySelectorAll('wa-select[aria-label="Model"] wa-option')].map(
      (o) => o.getAttribute('value'),
    )
    expect(options).toEqual(['sonnet', 'haiku'])
    // 一级 provider 选择器只在创建模式出现。
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Provider"]')).toBeNull()
  })

  it('submit calls createSession with project_id null when projectId prop is null', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_inbox' })
    const el = await mount({ projectDir: null })

    // Type into the textarea by simulating an `input` event on the
    // WA shadow-DOM textarea. We reach into its shadow root directly
    // because that's the only path the real WA component supports too.
    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & {
      value: string
    }
    expect(ta).toBeTruthy()
    ta.value = 'hello agent'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    const created = vi.fn()
    el.addEventListener('composer-created', created)
    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    expect(sendBtn).toBeTruthy()
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'hello agent',
      projectId: null,
      agent: 'claude', // 预选 = 首个可达 agent（mock catalog 的第一项）
      model: null,
    })
    expect(created).toHaveBeenCalledTimes(1)
    expect((created.mock.calls[0]![0] as CustomEvent<{ key: string }>).detail.key).toBe('oc_inbox')
  })

  it('submit calls createSession with project_id=<id> when projectId prop is set', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_proj' })
    const el = await mount({ projectId: 'proj-sebas', projectDir: '/home/me/code/sebas' })

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & {
      value: string
    }
    ta.value = 'work on this'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    const created = vi.fn()
    el.addEventListener('composer-created', created)
    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'work on this',
      projectId: 'proj-sebas',
      agent: 'claude',
      model: null,
    })
    expect((created.mock.calls[0]![0] as CustomEvent<{ key: string }>).detail.key).toBe('oc_proj')
    // Binding caption shows the trailing path segment.
    expect(el.shadowRoot?.textContent ?? '').toContain('sebas')
  })

  it('submit is no-op when text is empty', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    const el = await mount({ projectDir: null })
    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(api.createSession).not.toHaveBeenCalled()
  })

  it('project switch preselects the remembered default agent; no record keeps the pick (wire D5)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    const el = await mount({ projectDir: null })
    // 初始预选 = 首个可达 agent（catalog 兜底）。
    expect(el.agent).toBe('claude')

    // 项目带着记住的 default_agent 切入 → 预选它（wire D5）。
    el.projectDefaultAgent = 'gemini'
    await el.updateComplete
    expect(el.agent).toBe('gemini')

    // 无记录（null）不改动现选——「first visit falls back honestly」的另一面。
    el.projectDefaultAgent = null
    await el.updateComplete
    expect(el.agent).toBe('gemini')
  })

  it('forwards the agent selected in the drop-down (default = first reachable)', async () => {    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_native' })
    const el = await mount({ projectDir: null })

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'run natively'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    // Flip the agent drop-down to native.
    const select = el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]') as unknown as
      | (HTMLElement & { value: string; disabled: boolean })
      | null
    expect(select).toBeTruthy()
    expect(select!.value).toBe('claude')
    select!.value = 'native'
    select!.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    await el.updateComplete

    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'run natively',
      projectId: null,
      agent: 'native',
      model: null,
    })
  })

  it('lists reachable agents and forwards the selected agent id (D2 wire)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_gemini' })
    const el = await mount({ projectDir: null })

    // Dropdown: reachable kinds + native; unreachable kinds are omitted.
    const options = Array.from(
      el.shadowRoot?.querySelectorAll('wa-option') ?? [],
    ) as HTMLElement[]
    const values = options.map((o) => o.getAttribute('value'))
    expect(values).toContain('claude')
    expect(values).toContain('gemini')
    expect(values).toContain('native')
    // 不可达 agent 仍在列表（disabled + cause），但值就是 agent id——
    // 没有 acp: 前缀（D2：wire 词汇 = agent id）。
    expect(values).toContain('codex')

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'use gemini'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    const select = el.shadowRoot?.querySelector('wa-select') as unknown as
      | (HTMLElement & { value: string })
      | null
    select!.value = 'gemini'
    select!.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    await el.updateComplete

    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'use gemini',
      projectId: null,
      agent: 'gemini',
      model: null,
    })
  })

  it('error path surfaces inline and preserves text', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom: 500'))
    const el = await mount({ projectDir: null })

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & {
      value: string
    }
    ta.value = 'retry me'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    const errorCallout = el.shadowRoot?.querySelector<HTMLElement>('.callout-error')
    expect(errorCallout?.textContent ?? '').toContain('boom: 500')
    // Text preserved for retry — the composer does not clear on failure.
    expect(ta.value).toBe('retry me')
  })

  // ── Enter 发送路径（@keydown on wa-textarea）─────────────────────────────

  it('plain Enter sends; Shift+Enter does not', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_enter' })
    const el = await mount({ projectDir: null })
    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'enter to send'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    // Shift+Enter：不发送。
    ta.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, composed: true }),
    )
    await new Promise((r) => setTimeout(r, 0))
    expect(api.createSession).not.toHaveBeenCalled()

    // 普通 Enter：发送。
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true }))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith({
      prompt: 'enter to send',
      projectId: null,
      agent: 'claude',
      model: null,
    })
  })

  it('Enter is a no-op while reachability is unreachable', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryUnreachable)
    const el = await mount({ projectDir: null })
    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'should not send'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true }))
    await new Promise((r) => setTimeout(r, 0))
    expect(api.createSession).not.toHaveBeenCalled()
  })

  // ── 跟随模式（add-composer-agent-binding）：聚焦会话的输入框 ────────────

  describe('follow-up mode (focused session)', () => {
    const focus = {
      sessionKey: 'web%00web-1',
      agentKind: 'claude',
      sessionModels: ['sonnet', 'haiku'],
      currentModel: 'sonnet',
    }

    it('sends to the focused session and never creates a new one', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      ;(api.sendMessage as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'delivered' })
      const el = await mount(focus)

      const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
      ta.value = 'one more turn'
      ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
      await el.updateComplete

      const sent = vi.fn()
      el.addEventListener('composer-sent', sent)
      ;(el.shadowRoot?.querySelector('.send-button') as HTMLElement).click()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete

      expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'one more turn')
      expect(api.createSession).not.toHaveBeenCalled()
      expect(sent).toHaveBeenCalledTimes(1)
    })

    it('renders the agent as small read-only text — no agent select anywhere', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      const el = await mount(focus)

      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]')).toBeNull()
      const labels = Array.from(el.shadowRoot?.querySelectorAll('.label') ?? []).map(
        (n) => n.textContent ?? '',
      )
      // 跟随模式 agent 标签 = catalog display（workbench-agent-wire-fix 3.1：
      // kind slug 'claude' → display 'Claude Code'）。
      expect(labels).toContain('🔒 Claude Code')
      // 创建模式才有的绑定/供应商提示在跟随模式下不渲染。
      expect(el.shadowRoot?.querySelector('.binding')).toBeNull()
    })

    it('agent kind null falls back to the default-agent label', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      const el = await mount({ ...focus, agentKind: null })
      const labels = Array.from(el.shadowRoot?.querySelectorAll('.label') ?? []).map(
        (n) => n.textContent ?? '',
      )
      expect(labels).toContain('🔒 default agent')
    })

    it('model dropdown lists the focused session models and switches via setSessionModel', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      ;(api.setSessionModel as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'ok' })
      const el = await mount(focus)

      const sel = el.shadowRoot?.querySelector(
        'wa-select[aria-label="Model"]',
      ) as unknown as HTMLElement & { value: string }
      expect(sel.value).toBe('sonnet')
      sel.value = 'haiku'
      sel.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete

      expect(api.setSessionModel).toHaveBeenCalledWith('web%00web-1', 'haiku')
      expect(api.createSession).not.toHaveBeenCalled()
    })

    it('"+ new session" chip flips to creation mode and back', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_new' })
      const el = await mount(focus)
      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]')).toBeNull()

      const chip = () => el.shadowRoot?.querySelector('.mode-chip') as HTMLElement
      chip().click()
      await el.updateComplete

      // 创建模式：agent 下拉回来了，chips 变为取消。
      const backend = el.shadowRoot?.querySelector(
        'wa-select[aria-label="Agent"]',
      ) as unknown as HTMLElement & { value: string }
      expect(backend).toBeTruthy()
      expect(chip().textContent?.trim()).toBe('cancel')

      const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
      ta.value = 'brand new session'
      ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
      await el.updateComplete
      ;(el.shadowRoot?.querySelector('.send-button') as HTMLElement).click()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(api.createSession).toHaveBeenCalledWith({
        prompt: 'brand new session',
        projectId: null,
        agent: 'claude',
        model: null,
      })
      expect(api.sendMessage).not.toHaveBeenCalled()

      chip().click()
      await el.updateComplete
      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]')).toBeNull()
    })
  })

  // ── Reachability 轮询（断连横幅随 core 恢复自动消失）─────────────────────

  // ── wire-webui-sebas-agent-e2e 4.1：后端下拉按执行体可用性渲染 ───────────

  it('renders the native option disabled with its cause when the catalog reports native unreachable', async () => {
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
      agents: [
        { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
        { id: 'native', display: 'Native Kernel', reachable: false, cause: 'no provider credentials' },
      ],
    })
    const el = await mount({ projectDir: null })

    const native = el.shadowRoot?.querySelector(
      'wa-option[value="native"]',
    ) as HTMLElement | null
    expect(native).toBeTruthy()
    expect(native!.hasAttribute('disabled')).toBe(true)
    expect(native?.textContent ?? '').toContain('unavailable')
    expect(native?.textContent ?? '').toContain('no provider credentials')
    // acp 侧不受 native 状态影响。
    const claude = el.shadowRoot?.querySelector('wa-option[value="claude"]') as HTMLElement | null
    expect(claude).toBeTruthy()
    expect(claude!.hasAttribute('disabled')).toBe(false)
  })

  it('keeps the native option selectable when the catalog reports native reachable', async () => {
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
      agents: [
        { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
        { id: 'native', display: 'Native Kernel', reachable: true },
      ],
    })
    const el = await mount({ projectDir: null })

    const native = el.shadowRoot?.querySelector(
      'wa-option[value="native"]',
    ) as HTMLElement | null
    expect(native).toBeTruthy()
    expect(native!.hasAttribute('disabled')).toBe(false)
    expect(native?.textContent ?? '').not.toContain('unavailable')
  })

  it('re-enables the native option when the next catalog poll reports reachable', async () => {
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
      agents: [
        { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
        { id: 'native', display: 'Native Kernel', reachable: false, cause: 'no provider credentials' },
      ],
    })
    const el = await mount({ projectDir: null })
    let native = el.shadowRoot?.querySelector('wa-option[value="native"]') as HTMLElement | null
    expect(native?.hasAttribute('disabled')).toBe(true)

    // core 恢复（凭据注入）：下一次轮询拉到 ok → 禁选解除，无需重挂载。
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
      agents: [
        { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
        { id: 'native', display: 'Native Kernel', reachable: true },
      ],
    })
    await (el as unknown as { loadReachability(): Promise<void> }).loadReachability()
    await el.updateComplete
    native = el.shadowRoot?.querySelector('wa-option[value="native"]') as HTMLElement | null
    expect(native?.hasAttribute('disabled')).toBe(false)
  })

  it('reachability recovers on poll without remounting', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryUnreachable)
    const el = await mount({ projectDir: null })
    expect(el.shadowRoot?.querySelector('.callout-warning')).toBeTruthy()
    // connectedCallback 安装了轮询定时器。
    const timer = (el as unknown as { reachabilityTimer: number | undefined }).reachabilityTimer
    expect(timer).not.toBeUndefined()

    // core 恢复：下一次轮询拉到 ok → 横幅消失、composer 重新可用。
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    await (el as unknown as { loadReachability(): Promise<void> }).loadReachability()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.callout-warning')).toBeNull()
    const textarea = el.shadowRoot?.querySelector('wa-textarea')
    expect(textarea?.hasAttribute('disabled')).toBe(false)

    // 卸载后定时器被清理。
    el.remove()
    await el.updateComplete
    expect(
      (el as unknown as { reachabilityTimer: number | undefined }).reachabilityTimer,
    ).toBeUndefined()
  })
})


// ── workbench-conversation-view 4.3/4.4：创建模式目录的诚实降级 ───────────

it('creation mode states model unavailability honestly when no catalog exists', async () => {
  // 空目录（无 provider 目录）：显式不可用提示，不显示空下拉、不伪造选项。
  ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
  const el = await mount({
    sessionKey: 'web%00web-1',
    agentKind: 'claude',
    sessionModels: ['sonnet', 'haiku'],
    currentModel: 'sonnet',
  })
  const chip = () => el.shadowRoot?.querySelector('.mode-chip') as HTMLElement
  chip().click()
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete

  const sel = el.shadowRoot?.querySelector('wa-select[aria-label="Model"]')
  expect(sel).toBeNull()
  const hint = el.shadowRoot?.querySelector('[data-testid="catalog-unavailable"]')
  expect(hint?.textContent ?? '').toContain('model catalog unavailable')
})

it('creation mode states catalog unavailability when the directory read fails (4.4)', async () => {
  ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
  // router/core 状态库不在跑：providers 读取失败 → 显式不可用，不是空列表。
  const failure = Object.assign(new Error('core 状态库不可达'), { status: 503 })
  ;(api.routerProviders as ReturnType<typeof vi.fn>).mockRejectedValue(failure)
  ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockRejectedValue(failure)
  const el = await mount({ projectDir: null })
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete

  expect(el.shadowRoot?.querySelector('wa-select[aria-label="Provider"]')).toBeNull()
  expect(el.shadowRoot?.querySelector('wa-select[aria-label="Model"]')).toBeNull()
  const hint = el.shadowRoot?.querySelector<HTMLElement>('[data-testid="catalog-unavailable"]')
  expect(hint).toBeTruthy()
  expect(hint!.textContent).toContain('model catalog unavailable')
  // 提交门不因目录不可用而锁死——模型是正交维度（agent 仍可选）。
  const agentSel = el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]')
  expect(agentSel).toBeTruthy()
})

describe('submission appends to a non-empty stack (workbench-turn-queue 7.5)', () => {
  it('follow-mode submit appends: sendMessage carries the new text and no stack entry is altered', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
      recent_sessions: [],
      active_count: 0,
      dormant_count: 0,
      spawning_count: 0,
      total_sessions: 0,
      active_session_key: 'web%00web-1',
    } as never)
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
      agents: [{ id: 'claude', display: 'Claude', reachable: true }],
    })
    ;(api.sendMessage as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'ok' })

    // 堆叠区已有两条待执行提交（一条在跑回合后的排队回合 + 一条优先项）。
    const el = document.createElement('sebas-pending-stack') as SebasPendingStack
    el.sessionKey = 'web%00web-1'
    el.pending = [
      { id: 1, text: 'queued one', position: 0, disposition: 'turn', priority: false },
      { id: 2, text: 'urgent /btw', position: 1, disposition: 'turn', priority: true },
    ]
    document.body.appendChild(el)
    await el.updateComplete

    const composer = await mount({
      sessionKey: 'web%00web-1',
      agentKind: 'claude',
    })
    const textarea = () =>
      composer.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLTextAreaElement
    textarea().value = 'a brand new submission'
    textarea().dispatchEvent(new Event('input', { bubbles: true }))
    await composer.updateComplete
    const send = composer.shadowRoot?.querySelector('.send-button') as HTMLElement
    send.click()
    await new Promise((r) => setTimeout(r, 0))
    await composer.updateComplete

    // 提交是追加：sendMessage 收到完整的新文本。
    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'a brand new submission')
    // 既有条目文本不被覆盖、不被合并：堆叠区仍逐字保留原文本。
    const stackText = el.shadowRoot?.textContent ?? ''
    expect(stackText).toContain('queued one')
    expect(stackText).toContain('urgent /btw')
    expect(stackText).not.toContain('a brand new submission')
    el.remove()
  })
})

describe('native availability fixtures (wire-webui-sebas-agent-e2e 4.1)', () => {
  it('the fixtures model one available and one unavailable execution body', () => {
    expect(summaryNativeAvailable.execution_bodies?.every((b) => b.ok)).toBe(true)
    expect(summaryNativeUnavailable.execution_bodies?.find((b) => b.name === 'native')?.ok).toBe(
      false,
    )
  })
})

/**
 * add-remote-execution-node 8.2：项目节点不可用时 composer 在**提交前**就
 * 阻止（提交只会 bounce），并把节点与成因说清楚。
 */
describe('execution node gate (add-remote-execution-node 8.2)', () => {
  it('disables submit and states the node cause when the project node is unavailable', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    const el = await mount({ projectId: 'proj-r', projectDir: '/srv/repo' })
    el.nodeBlocked = { nodeId: 'dev-box', status: 'offline', cause: '节点离线（上次在线 5m ago）' }
    await el.updateComplete

    const send = el.shadowRoot!.querySelector<HTMLButtonElement>('button.send-button')
    expect(send!.disabled).toBe(true)
    const textarea = el.shadowRoot!.querySelector('wa-textarea')
    expect(textarea?.hasAttribute('disabled')).toBe(true)

    const callout = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="node-blocked"]')
    expect(callout).toBeTruthy()
    expect(callout!.textContent).toContain('dev-box')
    expect(callout!.textContent).toContain('节点离线')

    // 提交被拦下：createSession 不会被调用。
    ;(el as unknown as { submit: () => Promise<void> }).submit()
    await el.updateComplete
    expect(api.createSession).not.toHaveBeenCalled()
    el.remove()
  })

  it('leaves submit enabled when the node is reachable', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    const el = await mount({ projectId: 'proj-r', projectDir: '/srv/repo' })
    expect(el.nodeBlocked).toBeNull()
    const send = el.shadowRoot!.querySelector<HTMLButtonElement>('button.send-button')
    expect(send!.disabled).toBe(false)
    expect(el.shadowRoot!.querySelector('[data-testid="node-blocked"]')).toBeNull()
    el.remove()
  })
})
