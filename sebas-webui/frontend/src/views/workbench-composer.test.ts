// @vitest-environment jsdom
/**
 * Workbench composer behaviour（workbench-interaction-polish 4.1–4.3：纯跟随
 * 模式 + 模型芯片 + 发送状态机）。Mount the real LitElement with the api
 * client mocked so we can drive:
 *   - 纯跟随化：无创建/设置控件；无聚焦时显式指向 rail 创建入口的提示
 *   - 跟随模式：提交发给聚焦会话；agent 🔒 标签；错误就地呈现且保字
 *   - 模型芯片：目录交叉引用分组 / 未匹配归「会话提供」/ 目录不可得退化
 *     平铺 / 当前模型打勾 / 无可用模型显式提示 / 切换走 setSessionModel
 *   - 发送状态机：disabled → send → sending → stop（cancelSession）→
 *     queued（sendMessage 排队）逐态断言；turn 结束复位
 *
 * jsdom's ElementInternals shim is incomplete (see wa-polyfills) — WA form
 * elements need it to finish their update lifecycle.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasWorkbenchComposer } from './workbench-composer.js'
import type { AvailableCommandInfo, Summary } from '../api/client.js'
import {
  elementInternalsPolyfillInvoked,
  installWaDomPolyfills,
} from '../test-support/wa-polyfills.js'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

// ---- WA 渲染垫片（共享）--------------------------------------------------
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
    agents: vi.fn(),
    sendMessage: vi.fn(),
    cancelSession: vi.fn(),
    setSessionModel: vi.fn(),
    providers: vi.fn(),
    providerDefaults: vi.fn(),
  },
}))

// Import the composer module now that the api is mocked and the
// ElementInternals polyfill is in place.
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

async function mount(initial: Partial<SebasWorkbenchComposer> = {}) {
  const el = document.createElement('sebas-workbench-composer') as SebasWorkbenchComposer
  if (initial.sessionKey !== undefined) el.sessionKey = initial.sessionKey
  if (initial.agentKind !== undefined) el.agentKind = initial.agentKind
  if (initial.sessionModels !== undefined) el.sessionModels = initial.sessionModels
  if (initial.currentModel !== undefined) el.currentModel = initial.currentModel
  if (initial.turnInFlight !== undefined) el.turnInFlight = initial.turnInFlight
  if (initial.waitingApproval !== undefined) el.waitingApproval = initial.waitingApproval
  if (initial.sessionCommands !== undefined) el.sessionCommands = initial.sessionCommands
  if (initial.childStarting !== undefined) el.childStarting = initial.childStarting
  if (initial.currentMode !== undefined) el.currentMode = initial.currentMode
  if (initial.modeEditable !== undefined) el.modeEditable = initial.modeEditable
  if (initial.coreReachability !== undefined) el.coreReachability = initial.coreReachability
  document.body.appendChild(el)
  // LitElement schedules its first update asynchronously; then the
  // composer kicks off an async reachability fetch in connectedCallback.
  // We need to let the WA shadow children upgrade and render before
  // querying them.
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

/** 把文本写进 textarea 并派发 input（跟随模式的输入路径）。 */
async function type(el: SebasWorkbenchComposer, text: string): Promise<void> {
  const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLElement & {
    value: string
  }
  ta.value = text
  ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  await el.updateComplete
}

/** 向 textarea 派发 keydown（命令面板键盘导航与两段式提交的驱动路径）。 */
async function pressKey(
  el: SebasWorkbenchComposer,
  key: string,
  init: KeyboardEventInit = {},
): Promise<void> {
  const ta = el.shadowRoot?.querySelector('wa-textarea')
  ta?.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true, ...init }))
  await el.updateComplete
}

beforeEach(() => {
  vi.clearAllMocks()
  ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
  ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
    agents: [{ id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' }],
  })
  ;(api.sendMessage as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'delivered' })
  ;(api.cancelSession as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'cancelled' })
  ;(api.setSessionModel as ReturnType<typeof vi.fn>).mockResolvedValue({ status: 'ok' })
  // 目录（4.2 分组交叉引用的数据源）：默认空目录。
  ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
  ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
    default_provider: null,
    default_model: null,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
  if (!elementInternalsPolyfillInvoked()) {
    throw new Error('ElementInternals polyfill was never invoked')
  }
})

const focus = {
  sessionKey: 'web%00web-1',
  agentKind: 'claude',
  sessionModels: ['sonnet', 'haiku'],
  currentModel: 'sonnet',
}

describe('one-shot input focus (workbench-rail-polish 3.2)', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('focusInput targets the textarea host (delegates into the native input)', async () => {
    const el = await mount(focus)
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLElement
    expect(ta).toBeTruthy()
    const focusSpy = vi.spyOn(ta, 'focus')
    await el.focusInput()
    expect(focusSpy).toHaveBeenCalledTimes(1)
    el.remove()
  })

  it('focusInput waits out the pending render when the placeholder just landed', async () => {
    const el = await mount({}) // 无聚焦：还没有 textarea
    expect(el.shadowRoot?.querySelector('wa-textarea')).toBeNull()
    // 真实时序（dashboard 渲染提交 sessionKey 后才调 focusInput）：update
    // 已排队、textarea 未上屏——await updateComplete 必须等它上屏再查。
    el.sessionKey = 'web%00web-1'
    // WaTextarea.focus 转发到内部原生 textarea：原生侧的探针能接到。
    const focusSpy = vi.spyOn(HTMLElement.prototype, 'focus')
    await el.focusInput()
    expect(el.shadowRoot?.querySelector('wa-textarea')).toBeTruthy()
    expect(focusSpy).toHaveBeenCalled()
    el.remove()
  })
})

// ── 4.1 纯跟随化 ─────────────────────────────────────────────────────────

describe('composer is pure follow-up (4.1)', () => {
  it('renders no creation, settings, or mode controls anywhere in the toolbar', async () => {
    const el = await mount(focus)
    // 创建控件：agent/provider 选择、mode 选择、「+ new session」chip 全部
    // 不存在——创建唯一入口是 rail 的创建对话框。
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Agent"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Provider"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('wa-select[aria-label="Permission mode"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('.mode-chip')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="mode-select"]')).toBeNull()
    // 设置入口归 app shell（侧栏底部）。
    expect(el.shadowRoot?.querySelector('.settings-link')).toBeNull()
    // 会话中 mode 控件不进 composer（留在会话头部）。
    expect(el.shadowRoot?.textContent).not.toContain('+ new session')
  })

  it('with no focused session renders the rail-creation hint and no submit affordance', async () => {
    const el = await mount({ sessionKey: null })
    const hint = el.shadowRoot?.querySelector('[data-testid="composer-no-focus"]')
    expect(hint).toBeTruthy()
    // 显式指向 rail 创建入口（spec：hint directs to the rail's creation entry）。
    expect(hint?.textContent).toContain('+')
    expect(el.shadowRoot?.querySelector('wa-textarea')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="submit-control"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip"]')).toBeNull()
  })

  it('sends to the focused session and never creates a new one', async () => {
    const el = await mount(focus)
    await type(el, 'one more turn')
    const sent = vi.fn()
    el.addEventListener('composer-sent', sent)
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'one more turn')
    expect(api.cancelSession).not.toHaveBeenCalled()
    expect(sent).toHaveBeenCalledTimes(1)
    // 输入清空（提交成功）。
    expect(
      ((el.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLElement & { value: string }).value),
    ).toBe('')
  })

  it('bottom bar has no agent lock — the identity lives in the session head (4.1)', async () => {
    const el = await mount(focus)
    const labels = Array.from(el.shadowRoot?.querySelectorAll('.label') ?? []).map(
      (n) => n.textContent ?? '',
    )
    expect(labels.every((l) => !l.includes('🔒'))).toBe(true)
  })

  it('mode switch renders only when the session is editable (4.1)', async () => {
    // 0-turn 占位（session_id 为空 → modeEditable=false）：无 mode 控件，
    // mode 由创建表单决定。
    const el = await mount(focus)
    expect(el.shadowRoot?.querySelector('[data-testid="mode-switch"]')).toBeNull()
    const editable = await mount({ ...focus, modeEditable: true, currentMode: 'ask' })
    const sel = editable.shadowRoot?.querySelector('[data-testid="mode-switch"]')
    expect(sel).toBeTruthy()
    editable.remove()
  })

  it('plain Enter sends; Shift+Enter does not; empty text is a no-op', async () => {
    const el = await mount(focus)
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLElement & { value: string }

    // 空文本 Enter：不发送。
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true }))
    await new Promise((r) => setTimeout(r, 0))
    expect(api.sendMessage).not.toHaveBeenCalled()

    await type(el, 'enter to send')
    // Shift+Enter：不发送。
    ta.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, composed: true }),
    )
    await new Promise((r) => setTimeout(r, 0))
    expect(api.sendMessage).not.toHaveBeenCalled()

    // 普通 Enter：发送。
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true }))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.sendMessage).toHaveBeenCalledTimes(1)
  })

  it('error path surfaces inline and preserves text', async () => {
    ;(api.sendMessage as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom: 500'))
    const el = await mount(focus)
    await type(el, 'retry me')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    const errorCallout = el.shadowRoot?.querySelector<HTMLElement>('[data-testid="composer-error"]')
    expect(errorCallout?.textContent ?? '').toContain('boom: 500')
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as HTMLElement & { value: string }
    expect(ta.value).toBe('retry me')
  })

  // ── add-core-reachability-ws-push 2.2：提交门消费 shell 下传的推送状态 ──

  it('gates submit on the pushed reachability; the recovery state re-enables without remount', async () => {
    const el = await mount({
      ...focus,
      coreReachability: { ok: false, kind: 'disconnected', cause: 'router down' },
    })
    const ta = el.shadowRoot?.querySelector('wa-textarea')
    expect(ta?.hasAttribute('disabled')).toBe(true)
    const callout = el.shadowRoot?.querySelector<HTMLElement>('.callout-warning')
    expect(callout?.textContent ?? '').toContain('router down')
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).getAttribute(
        'data-state',
      ),
    ).toBe('disabled')

    // 恢复通知（shell 下传 ok=true）：禁用解除，无需重挂载、无任何 fetch。
    el.coreReachability = { ok: true }
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.callout-warning')).toBeNull()
    expect(
      (el.shadowRoot?.querySelector('wa-textarea') as HTMLElement).hasAttribute('disabled'),
    ).toBe(false)
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).getAttribute(
        'data-state',
      ),
    ).toBe('disabled') // 有字才 send——此刻输入为空，退回普通禁用态。
  })

  it('unknown reachability (null) leaves the gate open — the retired fetch leaves no residue', async () => {
    // shell 尚未下传任何状态（get 未应答）：对齐旧轮询器读数前的默认——
    // 不禁用。composer 自身不再发任何 reachability 请求。
    const el = await mount(focus)
    expect(api.summary).not.toHaveBeenCalled()
    expect(
      (el.shadowRoot?.querySelector('wa-textarea') as HTMLElement).hasAttribute('disabled'),
    ).toBe(false)
    el.remove()

    // 结构性守卫：轮询定时器与挂载 fetch 已从源码删除。
    const src = readFileSync(join(here, 'workbench-composer.ts'), 'utf8')
    expect(src).not.toContain('WORKBENCH_REACHABILITY_POLL_MS')
    expect(src).not.toContain('api.summary')
    expect(src).not.toContain('setInterval')
  })
})

// ── 4.2 模型芯片 ─────────────────────────────────────────────────────────

describe('model chip (4.2, design D3)', () => {
  function seedCatalog() {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'deepseek',
          models: [{ id: 'deepseek-chat', tags: [] }],
        },
        {
          name: 'anthropic',
          models: [{ id: 'sonnet', tags: [] }, { id: 'haiku', tags: [] }],
        },
      ],
    })
  }

  it('groups session models by provider via the catalog; current model is marked', async () => {
    seedCatalog()
    const el = await mount(focus)
    const chip = el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement
    expect(chip).toBeTruthy()
    expect(chip.textContent).toContain('sonnet')
    chip.click()
    await el.updateComplete

    const menu = el.shadowRoot?.querySelector('[data-testid="model-menu"]')
    expect(menu).toBeTruthy()
    const groupLabels = [
      ...(el.shadowRoot?.querySelectorAll('[data-testid="model-group"]') ?? []),
    ].map((n) => n.textContent?.trim())
    // 目录交叉引用：sonnet/haiku 归 anthropic；deepseek-chat 不在会话模型里
    // 不出现。
    expect(groupLabels).toEqual(['anthropic'])
    const items = [
      ...(el.shadowRoot?.querySelectorAll('.menu-item') ?? []),
    ] as HTMLElement[]
    expect(items.map((i) => i.getAttribute('data-model'))).toEqual(['sonnet', 'haiku'])
    expect(items[0]!.getAttribute('aria-selected')).toBe('true')
    expect(items[1]!.getAttribute('aria-selected')).toBe('false')
  })

  it('puts ids the catalog cannot place into the explicit session-provided group at the bottom', async () => {
    seedCatalog()
    const el = await mount({ ...focus, sessionModels: ['sonnet', 'custom-local'] })
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete

    const groupLabels = [
      ...(el.shadowRoot?.querySelectorAll('[data-testid="model-group"]') ?? []),
    ].map((n) => n.textContent?.trim())
    expect(groupLabels).toEqual(['anthropic', '会话提供'])
    const items = [
      ...(el.shadowRoot?.querySelectorAll('.menu-item') ?? []),
    ] as HTMLElement[]
    expect(items.map((i) => i.getAttribute('data-model'))).toEqual(['sonnet', 'custom-local'])
  })

  it('degrades to a flat list when the catalog is unavailable (no fabricated groups)', async () => {
    // 目录读取失败 → 芯片仍可用，单层平铺，无组头。
    ;(api.providers as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('503'), { status: 503 }),
    )
    const el = await mount(focus)
    await new Promise((r) => setTimeout(r, 0))
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete

    expect(el.shadowRoot?.querySelectorAll('[data-testid="model-group"]').length).toBe(0)
    const items = [
      ...(el.shadowRoot?.querySelectorAll('.menu-item') ?? []),
    ] as HTMLElement[]
    expect(items.map((i) => i.getAttribute('data-model'))).toEqual(['sonnet', 'haiku'])
  })

  it('states honest unavailability when the session exposes no models', async () => {
    const el = await mount({ ...focus, sessionModels: [], currentModel: null })
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip"]')).toBeNull()
    const note = el.shadowRoot?.querySelector('[data-testid="model-chip-unavailable"]')
    expect(note).toBeTruthy()
  })

  it('switches via setSessionModel and closes the menu', async () => {
    seedCatalog()
    const el = await mount(focus)
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot?.querySelector('.menu-item[data-model="haiku"]') as HTMLElement
    item.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.setSessionModel).toHaveBeenCalledWith('web%00web-1', 'haiku')
    expect(el.shadowRoot?.querySelector('[data-testid="model-menu"]')).toBeNull()
  })

  it('switching a session model goes through setSessionModel only (preselect-last-used-model 2.1)', async () => {
    seedCatalog()
    const el = await mount(focus)
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot?.querySelector('.menu-item[data-model="haiku"]') as HTMLElement
    item.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.setSessionModel).toHaveBeenCalledWith('web%00web-1', 'haiku')
    // 会话级切换不走创建记忆的写入点——LAST_USED_PAIR_KEY 只属于创建
    // 对话框的确认动作（写入点唯一性的正半边在此断言）。
  })

  it('closes the menu on outside click and on Escape', async () => {
    seedCatalog()
    const el = await mount(focus)
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('[data-testid="model-menu"]')).toBeTruthy()

    // Escape 关闭。
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, composed: true }))
    const menu = el.shadowRoot?.querySelector('[data-testid="model-menu"]')
    menu?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('[data-testid="model-menu"]')).toBeNull()

    // 重新打开后点击组件外 → 关闭。
    ;(el.shadowRoot?.querySelector('[data-testid="model-chip"]') as HTMLElement).click()
    await el.updateComplete
    document.body.click()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('[data-testid="model-menu"]')).toBeNull()
  })
})

// ── 4.3 发送状态机 ────────────────────────────────────────────────────────

describe('submit control state machine (4.3, design D4)', () => {
  function stateOf(el: SebasWorkbenchComposer): string | null {
    return (
      el.shadowRoot
        ?.querySelector('[data-testid="submit-control"]')
        ?.getAttribute('data-state') ?? null
    )
  }

  it('empty input with an idle focused session is disabled', async () => {
    const el = await mount(focus)
    expect(stateOf(el)).toBe('disabled')
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLButtonElement)
        .disabled,
    ).toBe(true)
  })

  it('typed input enables the send affordance', async () => {
    const el = await mount(focus)
    await type(el, 'ready')
    expect(stateOf(el)).toBe('send')
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLButtonElement)
        .disabled,
    ).toBe(false)
  })

  it('a POST in flight shows the spinner and ignores clicks', async () => {
    let resolveSend!: (v: { status: string }) => void
    ;(api.sendMessage as ReturnType<typeof vi.fn>).mockReturnValue(
      new Promise<{ status: string }>((r) => (resolveSend = r)),
    )
    const el = await mount(focus)
    await type(el, 'in flight')
    const btn = el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement
    btn.click()
    await el.updateComplete
    expect(stateOf(el)).toBe('sending')
    expect(btn.querySelector('.spinner')).toBeTruthy()
    // 在途点击不重复提交。
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    expect(api.sendMessage).toHaveBeenCalledTimes(1)

    resolveSend({ status: 'delivered' })
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 文本已清、POST 结束 → 回到 disabled（无字）。
    expect(stateOf(el)).toBe('disabled')
  })

  it('streaming with empty input offers stop; clicking it cancels via the cancel chain', async () => {
    const el = await mount({ ...focus, turnInFlight: true })
    expect(stateOf(el)).toBe('stop')
    const btn = el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLButtonElement
    expect(btn.disabled).toBe(false)
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.cancelSession).toHaveBeenCalledWith('web%00web-1')
    expect(api.sendMessage).not.toHaveBeenCalled()

    // turn 结束（WS 推送 turnInFlight=false）→ 自动回到 send/disabled 态。
    el.turnInFlight = false
    await el.updateComplete
    expect(stateOf(el)).toBe('disabled')
  })

  it('streaming with text offers the queued affordance; submitting enqueues via sendMessage', async () => {
    const el = await mount({ ...focus, turnInFlight: true })
    await type(el, 'line up behind the turn')
    expect(stateOf(el)).toBe('queued')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // 排队复用既有 turn-queue 提交路径：sendMessage 原文下发，不改栈。
    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'line up behind the turn')
    expect(api.cancelSession).not.toHaveBeenCalled()
  })

  it('cancel failure surfaces the typed rejection in the callout', async () => {
    ;(api.cancelSession as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('HTTP 409: 会话空闲（无在飞回复，无需取消）'),
    )
    const el = await mount({ ...focus, turnInFlight: true })
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const err = el.shadowRoot?.querySelector('[data-testid="composer-error"]')
    expect(err?.textContent).toContain('会话空闲')
  })
})

// ── fix-pending-queue-liveness 3.2 泊车指示与 waiting/starting 形态 ──────

describe('sebas-workbench-composer (parked / spawn-window submit control)', () => {
  function stateOf(el: SebasWorkbenchComposer): string | null {
    return (
      el.shadowRoot
        ?.querySelector('[data-testid="submit-control"]')
        ?.getAttribute('data-state') ?? null
    )
  }

  it('parked with empty input still offers stop and shows the waiting-on-operator hint', async () => {
    const el = await mount({ ...focus, turnInFlight: true, waitingApproval: true })
    // 停止在泊车态可达（不用去找审批卡）。
    expect(stateOf(el)).toBe('stop')
    const hint = el.shadowRoot?.querySelector('[data-testid="parked-hint"]')
    expect(hint).toBeTruthy()
    expect(hint?.textContent).toContain('等待你的审批')
  })

  it('parked with text queues visibly together with the waiting-on-operator indication', async () => {
    const el = await mount({ ...focus, turnInFlight: true, waitingApproval: true })
    await type(el, 'after you decide')
    expect(stateOf(el)).toBe('queued')
    const hint = el.shadowRoot?.querySelector('[data-testid="parked-hint"]')
    expect(hint?.textContent).toContain('等待你的审批')
    // 提交仍走排队路径（语义不变：泊车下提交 = 排在可回答的提问后面）。
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'after you decide')
  })

  it('running turn without the parked fact renders no hint', async () => {
    const el = await mount({ ...focus, turnInFlight: true, waitingApproval: false })
    await type(el, 'queue behind')
    expect(stateOf(el)).toBe('queued')
    expect(el.shadowRoot?.querySelector('[data-testid="parked-hint"]')).toBeNull()
  })

  it('spawn window (starting) with text offers the queued affordance', async () => {
    // spawn 窗口经 turn_engaged 折算成 turnInFlight=true（dashboard 供数）；
    // composer 的形态判定对 starting 与 working 一致——都是「回合占用」。
    const el = await mount({ ...focus, turnInFlight: true, childStarting: true })
    await type(el, 'first message staged')
    expect(stateOf(el)).toBe('queued')
    expect(el.shadowRoot?.querySelector('[data-testid="parked-hint"]')).toBeNull()
    el.turnInFlight = false
    await el.updateComplete
    expect(stateOf(el)).toBe('send')
  })
})

// ── session-slash-commands 3.1/3.2 命令面板 ──────────────────────────────

/** claude 形态的广告表（goal 带参数提示；compact 恰好也被 claude 广告）。 */
const claudeCommands: AvailableCommandInfo[] = [
  { name: 'goal', description: '跨回合追踪目标', hint: '<condition>' },
  { name: 'compact', description: '压缩会话历史', hint: null },
  { name: 'review', description: '复查最近改动' },
]

/** opencode 形态：有命令表面，但既不广告 goal 也不广告 compact。 */
const opencodeCommands: AvailableCommandInfo[] = [
  { name: 'init', description: '分析代码库并生成 AGENTS.md' },
  { name: 'help', description: '列出可用命令' },
]

describe('command palette (session-slash-commands 3.1/3.2, design D4)', () => {
  function paletteOf(el: SebasWorkbenchComposer) {
    return el.shadowRoot?.querySelector('[data-testid="command-palette"]')
  }
  function optionsOf(el: SebasWorkbenchComposer): HTMLElement[] {
    return [
      ...(el.shadowRoot?.querySelectorAll('[data-testid="command-palette"] [role="option"]') ??
        []),
    ] as HTMLElement[]
  }
  function selectedCommand(el: SebasWorkbenchComposer): string | null | undefined {
    return optionsOf(el)
      .find((o) => o.getAttribute('aria-selected') === 'true')
      ?.getAttribute('data-command')
  }

  it('opens on a leading slash listing single-line rows (name + hint); first option is highlighted', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/')

    const palette = paletteOf(el)
    expect(palette).toBeTruthy()
    expect(palette?.getAttribute('role')).toBe('listbox')
    const options = optionsOf(el)
    expect(options.map((o) => o.getAttribute('data-command'))).toEqual([
      'goal',
      'compact',
      'review',
    ])
    // （input-polish 3.1）行收敛：命令名 + 参数提示单行；描述不再内联
    // 铺开（描述只经 hover/高亮气泡呈现，见气泡 describe）。
    expect(options[0]?.textContent).toContain('/goal')
    expect(options[0]?.textContent).toContain('<condition>')
    expect(options[0]?.textContent).not.toContain('跨回合追踪目标')
    // 面板打开即高亮首位；aria-selected 模式同 model 菜单的 listbox。
    expect(options[0]?.getAttribute('aria-selected')).toBe('true')
    expect(options[1]?.getAttribute('aria-selected')).toBe('false')
  })

  it('keeps the two-phase completion and highlight behavior after the row collapse (3.1 regression)', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/RE')
    expect(optionsOf(el).map((o) => o.getAttribute('data-command'))).toEqual(['review'])
    await pressKey(el, 'Tab')
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as { value: string }
    expect(ta.value).toBe('/review ')
    expect(api.sendMessage).not.toHaveBeenCalled()
    // 行收敛后的结构守卫：行内描述的渲染与样式已从源码删除。
    const src = readFileSync(join(here, 'workbench-composer.ts'), 'utf8')
    expect(src).not.toContain('cmd-desc')
  })

  it('narrows live by a case-insensitive prefix as the operator types', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/CO')
    expect(optionsOf(el).map((o) => o.getAttribute('data-command'))).toEqual(['compact'])
  })

  it('renders no palette when nothing matches, for a non-leading slash, or once args begin', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })

    // 无匹配：空态不渲染。
    await type(el, '/zzz')
    expect(paletteOf(el)).toBeNull()

    // 非首字符 `/`（如 path/to）不触发。
    await type(el, 'path/to')
    expect(paletteOf(el)).toBeNull()

    // 命令名段结束（出现空白）= 参数阶段，面板随之关闭。
    await type(el, '/goal clear')
    expect(paletteOf(el)).toBeNull()
  })

  it('moves the highlight with ArrowUp/ArrowDown and clamps at the bounds', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/')
    expect(selectedCommand(el)).toBe('goal')
    await pressKey(el, 'ArrowDown')
    expect(selectedCommand(el)).toBe('compact')
    await pressKey(el, 'ArrowDown')
    expect(selectedCommand(el)).toBe('review')
    await pressKey(el, 'ArrowDown')
    expect(selectedCommand(el)).toBe('review') // 底部钳位
    await pressKey(el, 'ArrowUp')
    expect(selectedCommand(el)).toBe('compact')
  })

  it('dismisses on Escape and re-opens when the input changes again', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/')
    expect(paletteOf(el)).toBeTruthy()

    await pressKey(el, 'Escape')
    expect(paletteOf(el)).toBeNull()

    // 文本一变即重新获得开启资格：继续输入的前缀实时重过滤。
    await type(el, '/re')
    expect(paletteOf(el)).toBeTruthy()
    expect(optionsOf(el).map((o) => o.getAttribute('data-command'))).toEqual(['review'])
  })

  it('two-phase Enter: completes name + space without sending, then a later Enter submits', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/go')
    await pressKey(el, 'Enter')

    // 第一段：补全 `name + 空格`、面板关闭、焦点留输入框——面板开着时
    // Enter 绝不触发发送（两段式语义）。
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as { value: string }
    expect(ta.value).toBe('/goal ')
    expect(paletteOf(el)).toBeNull()
    expect(api.sendMessage).not.toHaveBeenCalled()

    // 第二段：参数补完后再次 Enter 正常提交。
    await type(el, '/goal keep tests green')
    await pressKey(el, 'Enter')
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', '/goal keep tests green')
  })

  it('Tab completes the highlighted command without sending', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/')
    await pressKey(el, 'ArrowDown') // 高亮移到 compact
    await pressKey(el, 'Tab')

    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as { value: string }
    expect(ta.value).toBe('/compact ')
    expect(api.sendMessage).not.toHaveBeenCalled()
  })

  it('clicking a palette row completes that command', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands })
    await type(el, '/')
    ;(el.shadowRoot?.querySelector('[data-command="review"]') as HTMLElement).click()
    await el.updateComplete

    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as { value: string }
    expect(ta.value).toBe('/review ')
    expect(api.sendMessage).not.toHaveBeenCalled()
  })
})

// ── input-polish 3.2 描述气泡（hover ∨ 高亮同源，markdown sanitize）──────

/** 混合形态：bare 无 description——收泡路径（移开/高亮到无描述行）的载体。 */
const mixedCommands: AvailableCommandInfo[] = [
  { name: 'goal', description: '跨回合追踪目标', hint: '<condition>' },
  { name: 'compact', description: '压缩**会话**历史' },
  { name: 'bare', description: '' },
]

describe('command description bubble (workbench-composer-input-polish 3.2, design D1-D3)', () => {
  function bubbleOf(el: SebasWorkbenchComposer): HTMLElement | null {
    return el.shadowRoot?.querySelector<HTMLElement>('[data-testid="command-bubble"]') ?? null
  }
  function hoverRow(el: SebasWorkbenchComposer, name: string, kind: 'enter' | 'leave'): void {
    ;(
      el.shadowRoot?.querySelector(`[data-command="${name}"]`) as unknown as HTMLElement
    )?.dispatchEvent(new Event(kind === 'enter' ? 'mouseenter' : 'mouseleave'))
  }

  it('shows a markdown bubble for the keyboard-highlighted row and moves it with ↑/↓', async () => {
    const el = await mount({ ...focus, sessionCommands: mixedCommands })
    await type(el, '/')
    // 高亮态出泡（面板打开即高亮首位 = goal）。
    let bubble = bubbleOf(el)
    expect(bubble).toBeTruthy()
    expect(bubble?.textContent).toContain('跨回合追踪目标')
    expect(bubble?.getAttribute('role')).toBe('tooltip')
    // 高亮移动 → 气泡跟随（同源渲染，即时切换）。
    await pressKey(el, 'ArrowDown')
    bubble = bubbleOf(el)
    expect(bubble?.textContent).toContain('压缩')
    expect(bubble?.textContent).toContain('会话')
    expect(bubble?.textContent).not.toContain('跨回合追踪目标')
    // 高亮到无描述行 → 收泡。
    await pressKey(el, 'ArrowDown')
    expect(bubbleOf(el)).toBeNull()
  })

  it('hover shows the hovered row bubble and takes precedence over the highlight', async () => {
    const el = await mount({ ...focus, sessionCommands: mixedCommands })
    await type(el, '/')
    // hover compact：气泡切到 hover 行（即便高亮仍在 goal）。
    hoverRow(el, 'compact', 'enter')
    await el.updateComplete
    expect(bubbleOf(el)?.textContent).toContain('压缩')
    expect(bubbleOf(el)?.textContent).not.toContain('跨回合追踪目标')
    // 移开（悬停到无描述的 bare 再离开）→ 回落键盘高亮行（goal，有描述）
    // 的同一渲染——鼠标路径与键盘路径天然同源（D3）。
    hoverRow(el, 'bare', 'enter')
    await el.updateComplete
    expect(bubbleOf(el)).toBeNull()
    hoverRow(el, 'bare', 'leave')
    await el.updateComplete
    expect(bubbleOf(el)?.textContent).toContain('跨回合追踪目标')
  })

  it('collapses with the palette on Escape and never renders without a command surface', async () => {
    const el = await mount({ ...focus, sessionCommands: mixedCommands })
    await type(el, '/')
    expect(bubbleOf(el)).toBeTruthy()
    await pressKey(el, 'Escape')
    expect(el.shadowRoot?.querySelector('[data-testid="command-palette"]')).toBeNull()
    expect(bubbleOf(el)).toBeNull()

    // 无命令表面：无面板即无气泡（诚实退化不因气泡而破）。
    const bare = await mount({ ...focus, sessionCommands: [] })
    await type(bare, '/')
    expect(bubbleOf(bare)).toBeNull()
    bare.remove()
  })

  it('reopens cleanly after Escape: the next keystroke restores palette and bubble (no dismissed/hover leak)', async () => {
    const el = await mount({ ...focus, sessionCommands: mixedCommands })
    await type(el, '/')
    // 悬停到 compact 再 Esc 收面板——hoverIndex 与 paletteDismissed 都必须
    // 随重开路径复位（任何重开都经过文本变化）。
    hoverRow(el, 'compact', 'enter')
    await el.updateComplete
    await pressKey(el, 'Escape')
    expect(el.shadowRoot?.querySelector('[data-testid="command-palette"]')).toBeNull()
    // 继续输入重开：面板回来、高亮复位到过滤后首位（goal），气泡同源跟随。
    await type(el, '/g')
    expect(el.shadowRoot?.querySelector('[data-testid="command-palette"]')).toBeTruthy()
    const bubble = bubbleOf(el)
    expect(bubble).toBeTruthy()
    expect(bubble?.textContent).toContain('跨回合追踪目标')
    expect(bubble?.textContent).not.toContain('压缩')
  })

  it('clamps a stale hover index in-bounds when the command surface shrinks mid-palette', async () => {
    const el = await mount({ ...focus, sessionCommands: mixedCommands })
    await type(el, '/')
    // 悬停中行 compact（下标 1），随后 agent 重新广告了更短的命令表（文本
    // 未动——不走 text 复位路径）：hoverIndex 瞬时越界，气泡必须钳位到收
    // 缩后候选范围内，绝不渲染越界/陈旧内容。
    hoverRow(el, 'compact', 'enter')
    await el.updateComplete
    expect(bubbleOf(el)?.textContent).toContain('压缩')
    el.sessionCommands = [{ name: 'goal', description: '新表的目标描述', hint: null }]
    await el.updateComplete
    // 唯一候选是 goal：钳位下标 0 → 气泡是 goal 的新描述，不是悬停残留。
    const bubble = bubbleOf(el)
    expect(bubble).toBeTruthy()
    expect(bubble?.textContent).toContain('新表的目标描述')
    expect(bubble?.textContent).not.toContain('压缩')
    expect(
      el.shadowRoot?.querySelectorAll('[data-testid="command-palette"] [role="option"]').length,
    ).toBe(1)
  })

  it('sanitizes markdown and keeps the 360×240 internally-scrollable caps (D2)', async () => {
    const hostile: AvailableCommandInfo[] = [
      {
        name: 'evil',
        description: 'do **bold** things\n\n<script>alert(1)</script> <img src=x onerror="alert(2)">',
      },
    ]
    const el = await mount({ ...focus, sessionCommands: hostile })
    await type(el, '/')
    const bubble = bubbleOf(el)
    expect(bubble).toBeTruthy()
    // markdown 经共享 renderMarkdown() 管线渲染（sanitize 语义同源）。
    expect(bubble?.innerHTML).toContain('<strong>bold</strong>')
    expect(bubble?.innerHTML).not.toContain('<script')
    expect(bubble?.innerHTML).not.toContain('alert(1)')
    expect(bubble?.innerHTML).not.toContain('onerror')
    // 尺寸上限与内部滚动是 CSS 契约（jsdom 无布局，做结构守卫）。
    const src = readFileSync(join(here, 'workbench-composer.ts'), 'utf8')
    expect(src).toContain('max-width: 360px')
    expect(src).toContain('max-height: 240px')
    expect(src).toContain('overflow-y: auto')
  })
})

// ── input-polish 2.4 只读 current 芯片 ───────────────────────────────────

describe('read-only current model chip (workbench-composer-input-polish 2.4)', () => {
  it('a session with an observed current but no options renders the model read-only', async () => {
    const el = await mount({ ...focus, sessionModels: [], currentModel: 'claude-sonnet-4-5' })
    // 只读展示当前模型——不再是「无可用模型」误导占位。
    const chip = el.shadowRoot?.querySelector('[data-testid="model-chip-readonly"]')
    expect(chip).toBeTruthy()
    expect(chip?.textContent).toContain('claude-sonnet-4-5')
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-unavailable"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip"]')).toBeNull()
    // 无菜单可开（role=status，纯状态展示）。
    expect(chip?.getAttribute('role')).toBe('status')
  })

  it('no options and no observed current keeps the honest unavailability note', async () => {
    const el = await mount({ ...focus, sessionModels: [], currentModel: null })
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-readonly"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-unavailable"]')).toBeTruthy()
  })

  it('the switchable chip still wins when options exist alongside a current model', async () => {
    const el = await mount(focus) // sessionModels + currentModel
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip"]')).toBeTruthy()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-readonly"]')).toBeNull()
  })

  it('the starting placeholder wins over the read-only chip while the child is spawning', async () => {
    // 优先级次序（input-polish 2.4）：子进程拉起中（模型表尚未上报）先于
    // 只读 current——此刻快照里的 current 还是 spawn 拼装的暂态值，展示
    // 「启动中…」比把暂态值钉成只读状态更诚实。
    const el = await mount({
      ...focus,
      sessionModels: [],
      currentModel: 'default',
      childStarting: true,
    })
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-starting"]')).toBeTruthy()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-readonly"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="model-chip-unavailable"]')).toBeNull()
  })
})

// ── session-slash-commands 4.1/4.2 拦截与诚实退化 ─────────────────────────

describe('interception and honest degradation (session-slash-commands 4.1/4.2, design D3)', () => {
  it('blocks an unadvertised command inline without sending (opencode shape: /goal not in surface)', async () => {
    const el = await mount({ ...focus, sessionCommands: opencodeCommands })
    await type(el, '/goal clear the board')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // 就地点名提示、不发请求、文本保留可改。
    const notice = el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')
    expect(notice?.textContent).toContain('/goal')
    expect(api.sendMessage).not.toHaveBeenCalled()
    const ta = el.shadowRoot?.querySelector('wa-textarea') as unknown as { value: string }
    expect(ta.value).toBe('/goal clear the board')
  })

  it('lets the universal built-in /compact through verbatim even when unadvertised', async () => {
    const el = await mount({ ...focus, sessionCommands: opencodeCommands })
    await type(el, '/compact')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', '/compact')
    expect(el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')).toBeNull()
  })

  it('clears the notice after the operator edits the text and allows resubmission', async () => {
    const el = await mount({ ...focus, sessionCommands: opencodeCommands })
    await type(el, '/goal nope')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')).toBeTruthy()

    // 改字 → 提示就地清除，可再提交。
    await type(el, 'plain follow-up')
    expect(el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')).toBeNull()
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', 'plain follow-up')
  })

  it('empty surface (native): no palette on / and /anything passes through as ordinary text (4.2)', async () => {
    const el = await mount({ ...focus, sessionCommands: [] })
    await type(el, '/')
    expect(el.shadowRoot?.querySelector('[data-testid="command-palette"]')).toBeNull()

    await type(el, '/anything at all')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.sendMessage).toHaveBeenCalledWith('web%00web-1', '/anything at all')
    expect(el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')).toBeNull()
  })

  it('advertised command passes verbatim with args; the busy path queues it unchanged (D5)', async () => {
    const el = await mount({ ...focus, sessionCommands: claudeCommands, turnInFlight: true })
    await type(el, '/goal keep tests green until CI passes')
    // turn 在飞 + 有字 = 排队形态：slash 提交与普通消息同一条 sendMessage
    // 路径（D5 不特判），原文下发。
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).getAttribute(
        'data-state',
      ),
    ).toBe('queued')
    ;(el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(api.sendMessage).toHaveBeenCalledWith(
      'web%00web-1',
      '/goal keep tests green until CI passes',
    )
    expect(el.shadowRoot?.querySelector('[data-testid="slash-unsupported"]')).toBeNull()
  })
})
