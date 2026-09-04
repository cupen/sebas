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

// ---- ElementInternals polyfill ----------------------------------------
// Wrap attachInternals so WA's willUpdate doesn't blow up. The stubbed
// methods are no-ops (or trivial getters) — we only care about letting
// the WA element reach a stable state so the composer template renders.
// jsdom's `ElementInternals` lacks `setFormValue`/`setValidity`/etc.,
// so we always wrap (the original is preserved on the prototype's
// __wrapped__ flag and called as the base for delegation).

const NOOP_INTERNALS_METHODS = [
  'setFormValue',
  'setValidity',
  'reportValidity',
  'checkValidity',
  'formStateRestoreCallback',
  'formResetCallback',
  'formDisabledCallback',
] as const

const proto = HTMLElement.prototype as unknown as {
  attachInternals?: (this: HTMLElement) => unknown
  __sebasWrappedAttachInternals?: boolean
}
let polyfillInstalled = false
if (!proto.__sebasWrappedAttachInternals) {
  const origAttach = proto.attachInternals
  proto.attachInternals = function (this: HTMLElement): unknown {
    let base: object = {}
    try {
      const r = origAttach?.call(this)
      if (r && typeof r === 'object') base = r as object
    } catch {
      /* non-custom-element or shim rejected */
    }
    const internals: Record<string, unknown> = Object.create(base)
    for (const name of NOOP_INTERNALS_METHODS) {
      if (typeof internals[name] !== 'function') internals[name] = () => {}
    }
    if (!('validity' in internals)) {
      internals.validity = { valid: true, valueMissing: false, customError: false }
    }
    if (!('willValidate' in internals)) internals.willValidate = false
    if (!('labels' in internals)) internals.labels = []
    if (!('form' in internals)) internals.form = null
    if (!('validationMessage' in internals)) internals.validationMessage = ''
    polyfillInstalled = true
    return internals
  }
  proto.__sebasWrappedAttachInternals = true
}

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
  reachability: { ok: false, cause: 'gateway down' },
}

async function mount(initial: Partial<SebasWorkbenchComposer> = {}) {
  const el = document.createElement('sebas-workbench-composer') as SebasWorkbenchComposer
  if (initial.projectDir !== undefined) el.projectDir = initial.projectDir
  if (initial.providerLabel !== undefined) el.providerLabel = initial.providerLabel
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
    gateway: {
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
  if (!polyfillInstalled) {
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
    expect(callout?.textContent ?? '').toContain('gateway down')
    expect(callout?.textContent ?? '').toContain('core not connected')
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
})
