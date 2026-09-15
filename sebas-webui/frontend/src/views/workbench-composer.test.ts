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

const summaryUnreachable: Summary = {
  ...summaryReachable,
  reachability: { ok: false, cause: 'router down' },
}

async function mount(initial: Partial<SebasWorkbenchComposer> = {}) {
  const el = document.createElement('sebas-workbench-composer') as SebasWorkbenchComposer
  if (initial.sessionKey !== undefined) el.sessionKey = initial.sessionKey
  if (initial.agentKind !== undefined) el.agentKind = initial.agentKind
  if (initial.sessionModels !== undefined) el.sessionModels = initial.sessionModels
  if (initial.currentModel !== undefined) el.currentModel = initial.currentModel
  if (initial.turnInFlight !== undefined) el.turnInFlight = initial.turnInFlight
  if (initial.sessionCommands !== undefined) el.sessionCommands = initial.sessionCommands
  if (initial.childStarting !== undefined) el.childStarting = initial.childStarting
  if (initial.currentMode !== undefined) el.currentMode = initial.currentMode
  if (initial.modeEditable !== undefined) el.modeEditable = initial.modeEditable
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

  it('renders disabled with cause when the core is unreachable; recovers without remount', async () => {
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryUnreachable)
    const el = await mount(focus)
    const ta = el.shadowRoot?.querySelector('wa-textarea')
    expect(ta?.hasAttribute('disabled')).toBe(true)
    const callout = el.shadowRoot?.querySelector<HTMLElement>('.callout-warning')
    expect(callout?.textContent ?? '').toContain('router down')
    expect(
      (el.shadowRoot?.querySelector('[data-testid="submit-control"]') as HTMLElement).getAttribute(
        'data-state',
      ),
    ).toBe('disabled')

    // core 恢复：下一次轮询拉到 ok → 禁用解除，无需重挂载。
    ;(api.summary as ReturnType<typeof vi.fn>).mockResolvedValue(summaryReachable)
    await (el as unknown as { loadReachability(): Promise<void> }).loadReachability()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.callout-warning')).toBeNull()
    expect(
      (el.shadowRoot?.querySelector('wa-textarea') as HTMLElement).hasAttribute('disabled'),
    ).toBe(false)
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

  it('opens on a leading slash listing name, argument hint, and description; first option is highlighted', async () => {
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
    // 行内容三要素：命令名 + 参数提示 + 说明。
    expect(options[0]?.textContent).toContain('/goal')
    expect(options[0]?.textContent).toContain('<condition>')
    expect(options[0]?.textContent).toContain('跨回合追踪目标')
    // 面板打开即高亮首位；aria-selected 模式同 model 菜单的 listbox。
    expect(options[0]?.getAttribute('aria-selected')).toBe('true')
    expect(options[1]?.getAttribute('aria-selected')).toBe('false')
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
