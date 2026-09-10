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
    agentKinds: vi.fn(),
    sendMessage: vi.fn(),
    setSessionModel: vi.fn(),
    agentDefaults: vi.fn(),
    routerProviders: vi.fn(),
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
  if (initial.projectDir !== undefined) el.projectDir = initial.projectDir
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
  ;(api.agentKinds as ReturnType<typeof vi.fn>).mockResolvedValue({
    kinds: [
      { name: 'claude', slug: 'claude', reachable: true, version: 'v1' },
      { name: 'gemini', slug: 'gemini', reachable: true, version: 'v2' },
      { name: 'codex', slug: 'codex', reachable: false, cause: 'command not found' },
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

  it('shows a model dropdown from the latest session and forwards the model', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
      recent_sessions: [
        {
          encoded_key: 'oc_gemini%00',
          chat_id: 'oc_gemini',
          thread_id: null,
          session_id: 's1',
          session_id_short: 's1',
          status: 'active',
          status_label: 'Working',
          status_slug: 'working',
          status_glyph: '▶',
          last_active: '0s ago',
          last_active_unix: 42,
          is_active: false,
          project_dir: null,
          prompt_preview: 'hi',
          current_model: 'pro-model',
          available_models: ['free-model', 'pro-model', 'gemini-2.5'],
        },
      ],
      active_count: 1,
      dormant_count: 0,
      spawning_count: 0,
      total_sessions: 1,
      active_session_key: null,
    } as never)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_m' })
    const el = await mount({ projectDir: null })

    // The dropdown is rendered and preselects the session's current model.
    const sel = el.shadowRoot?.querySelector('wa-select[aria-label="Model"]') as HTMLElement & {
      value: string
    }
    expect(sel).toBeTruthy()
    expect(sel.value).toBe('pro-model')

    // Submit through the model picker value.
    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'use this model'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete
    ;(el.shadowRoot?.querySelector('.send-button') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.createSession).toHaveBeenCalledWith('use this model', null, 'acp', 'pro-model')
  })

  it('hides the model dropdown when no session exposes available_models', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
      recent_sessions: [
        {
          encoded_key: 'oc_claude%00',
          chat_id: 'oc_claude',
          thread_id: null,
          session_id: 's1',
          session_id_short: 's1',
          status: 'active',
          status_label: 'Working',
          status_slug: 'working',
          status_glyph: '▶',
          last_active: '0s ago',
          last_active_unix: 42,
          is_active: false,
          project_dir: null,
          prompt_preview: 'hi',
          current_model: null,
          available_models: null,
        },
      ],
      active_count: 1,
      dormant_count: 0,
      spawning_count: 0,
      total_sessions: 1,
      active_session_key: null,
    } as never)
    const el = await mount({ projectDir: null })
    const sel = el.shadowRoot?.querySelector('wa-select[aria-label="Model"]')
    expect(sel).toBeNull()
  })

  it('submit calls createSession with project_dir null when projectDir prop is null', async () => {
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
    expect(api.createSession).toHaveBeenCalledWith('hello agent', null, 'acp', null)
    expect(created).toHaveBeenCalledTimes(1)
    expect((created.mock.calls[0]![0] as CustomEvent<{ key: string }>).detail.key).toBe('oc_inbox')
  })

  it('submit calls createSession with project_dir=<path> when projectDir prop is set', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_proj' })
    const el = await mount({ projectDir: '/home/me/code/sebas' })

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
    expect(api.createSession).toHaveBeenCalledWith('work on this', '/home/me/code/sebas', 'acp', null)
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

  it('forwards the backend selected in the drop-down (default acp)', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_native' })
    const el = await mount({ projectDir: null })

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'run natively'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    // Flip the execution-backend drop-down to native (5.2).
    const select = el.shadowRoot?.querySelector('wa-select') as unknown as
      | (HTMLElement & { value: string; disabled: boolean })
      | null
    expect(select).toBeTruthy()
    expect(select!.value).toBe('acp')
    select!.value = 'native'
    select!.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    await el.updateComplete

    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith('run natively', null, 'native', null)
  })

  it('lists only reachable agent kinds and forwards the selected acp:<slug> hint', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    ;(api.createSession as ReturnType<typeof vi.fn>).mockResolvedValue({ key: 'oc_gemini' })
    const el = await mount({ projectDir: null })

    // Dropdown: reachable kinds + native; unreachable kinds are omitted.
    const options = Array.from(
      el.shadowRoot?.querySelectorAll('wa-option') ?? [],
    ) as HTMLElement[]
    const values = options.map((o) => o.getAttribute('value'))
    expect(values).toContain('acp:claude')
    expect(values).toContain('acp:gemini')
    expect(values).not.toContain('acp:codex')
    expect(values).toContain('native')

    const ta = el.shadowRoot?.querySelector('wa-textarea') as HTMLElement & { value: string }
    ta.value = 'use gemini'
    ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    const select = el.shadowRoot?.querySelector('wa-select') as unknown as
      | (HTMLElement & { value: string })
      | null
    select!.value = 'acp:gemini'
    select!.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
    await el.updateComplete

    const sendBtn = el.shadowRoot?.querySelector('.send-button')
    ;(sendBtn as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.createSession).toHaveBeenCalledTimes(1)
    expect(api.createSession).toHaveBeenCalledWith('use gemini', null, 'acp:gemini', null)
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
    expect(api.createSession).toHaveBeenCalledWith('enter to send', null, 'acp', null)
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

    it('renders the agent as small read-only text — no backend select anywhere', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      const el = await mount(focus)

      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Execution backend"]')).toBeNull()
      const labels = Array.from(el.shadowRoot?.querySelectorAll('.label') ?? []).map(
        (n) => n.textContent ?? '',
      )
      expect(labels).toContain('claude')
      // 创建模式才有的绑定/供应商提示在跟随模式下不渲染。
      expect(el.shadowRoot?.querySelector('.binding')).toBeNull()
    })

    it('agent kind null falls back to the default-kind label', async () => {
      ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
      const el = await mount({ ...focus, agentKind: null })
      const labels = Array.from(el.shadowRoot?.querySelectorAll('.label') ?? []).map(
        (n) => n.textContent ?? '',
      )
      expect(labels).toContain('acp · default')
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
      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Execution backend"]')).toBeNull()

      const chip = () => el.shadowRoot?.querySelector('.mode-chip') as HTMLElement
      chip().click()
      await el.updateComplete

      // 创建模式：agent 下拉回来了，chips 变为取消。
      const backend = el.shadowRoot?.querySelector(
        'wa-select[aria-label="Execution backend"]',
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
      expect(api.createSession).toHaveBeenCalledWith('brand new session', null, 'acp', null)
      expect(api.sendMessage).not.toHaveBeenCalled()

      chip().click()
      await el.updateComplete
      expect(el.shadowRoot?.querySelector('wa-select[aria-label="Execution backend"]')).toBeNull()
    })
  })

  // ── Reachability 轮询（断连横幅随 core 恢复自动消失）─────────────────────

  // ── wire-webui-sebas-agent-e2e 4.1：后端下拉按执行体可用性渲染 ───────────

  it('renders the native option disabled with its cause when the native body is unavailable', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryNativeUnavailable)
    const el = await mount({ projectDir: null })

    const native = el.shadowRoot?.querySelector(
      'wa-option[value="native"]',
    ) as HTMLElement | null
    expect(native).toBeTruthy()
    expect(native!.hasAttribute('disabled')).toBe(true)
    expect(native?.textContent ?? '').toContain('unavailable')
    expect(native?.textContent ?? '').toContain('no provider credentials')
    // acp 侧不受 native 状态影响。
    const acp = el.shadowRoot?.querySelector('wa-option[value="acp"]') as HTMLElement | null
    expect(acp).toBeTruthy()
    expect(acp!.hasAttribute('disabled')).toBe(false)
  })

  it('keeps the native option selectable when the native body reports available', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryNativeAvailable)
    const el = await mount({ projectDir: null })

    const native = el.shadowRoot?.querySelector(
      'wa-option[value="native"]',
    ) as HTMLElement | null
    expect(native).toBeTruthy()
    expect(native!.hasAttribute('disabled')).toBe(false)
    expect(native?.textContent ?? '').not.toContain('unavailable')
  })

  it('re-enables the native option on the next poll without remounting', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryNativeUnavailable)
    const el = await mount({ projectDir: null })
    let native = el.shadowRoot?.querySelector('wa-option[value="native"]') as HTMLElement | null
    expect(native?.hasAttribute('disabled')).toBe(true)

    // core 恢复（凭据注入）：下一次轮询拉到 ok → 禁选解除，无需重挂载。
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryNativeAvailable)
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


// ── add-agent-defaults-catalog：创建模式选择器的 catalog 数据源 ───────────

it('creation mode offers the catalog of the defaults provider before any session', async () => {
  ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
  ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
    recent_sessions: [],
    active_count: 0,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 0,
    active_session_key: null,
  } as never)
  ;(api.agentDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
    provider: 'glm',
    model: 'm2',
  })
  ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
    providers: [
      { name: 'glm', models: ['m1', 'm2'], api_key_configured: true },
    ],
  })

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

  const sel = el.shadowRoot?.querySelector(
    'wa-select[aria-label="Model"]',
  ) as unknown as HTMLSelectElement & { value: string }
  expect(sel).toBeTruthy()
  const options = [...el.shadowRoot!.querySelectorAll('wa-option')].map((o) =>
    o.getAttribute('value'),
  )
  expect(options).toContain('m1')
  expect(options).toContain('m2')
  // defaults.model 命中 catalog 时预选它。
  expect(sel.value).toBe('m2')
})

it('creation mode states catalog unavailability honestly when nothing is available', async () => {
  ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
  ;(api.sessions as ReturnType<typeof vi.fn>).mockResolvedValue({
    recent_sessions: [],
    active_count: 0,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 0,
    active_session_key: null,
  } as never)
  ;(api.agentDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
    provider: null,
    model: null,
  })

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

  const hint = el.shadowRoot?.querySelector('[role="status"]')
  expect(hint?.textContent ?? '').toContain('no model catalog')
})
