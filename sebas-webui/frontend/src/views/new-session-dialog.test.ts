// @vitest-environment jsdom
/**
 * sebas-new-session-dialog（workbench-interaction-polish 3.1，design D2）。
 *
 * Covered:
 *   - 禁用：无 agent 可选（catalog 为空）→ 确认禁用；未选 agent → 确认禁用
 *   - 预选：项目 default_agent 预选；目录 default provider/model 预选；
 *     default 不在目录内 → 落目录首项（不伪造选项）
 *   - 取消：dialog-cancel 事件、不创建任何东西（无创建 API 调用）
 *   - 确认：dialog-confirm detail 携带 agent/model/mode；mode 缺省 = null
 *     （wire 上省略），选 edit = 'edit'
 *   - 目录不可得：显式提示，无 provider/model 下拉
 *   - 不可达 agent：禁选并标注 cause
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasNewSessionDialog } from './new-session-dialog.js'
import {
  elementInternalsPolyfillInvoked,
  installWaDomPolyfills,
} from '../test-support/wa-polyfills.js'

installWaDomPolyfills()

if (typeof (globalThis as { ResizeObserver?: unknown }).ResizeObserver === 'undefined') {
  class StubResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof StubResizeObserver }).ResizeObserver =
    StubResizeObserver
}

vi.mock('../api/client.js', () => ({
  api: {
    agents: vi.fn(),
    routerProviders: vi.fn(),
    routerDefaults: vi.fn(),
  },
}))

import './new-session-dialog.js'

import { api } from '../api/client.js'

async function mount(initial: Partial<SebasNewSessionDialog> = {}) {
  const el = document.createElement('sebas-new-session-dialog') as SebasNewSessionDialog
  if (initial.open !== undefined) el.open = initial.open
  if (initial.projectId !== undefined) el.projectId = initial.projectId
  if (initial.projectName !== undefined) el.projectName = initial.projectName
  if (initial.defaultAgent !== undefined) el.defaultAgent = initial.defaultAgent
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

function agentSelect(el: SebasNewSessionDialog): HTMLElement & { value: string; disabled: boolean } {
  return el.shadowRoot?.querySelector(
    '[data-testid="dialog-agent-select"]',
  ) as unknown as HTMLElement & { value: string; disabled: boolean }
}

function confirmButton(el: SebasNewSessionDialog): HTMLButtonElement {
  return el.shadowRoot?.querySelector(
    '[data-testid="dialog-confirm"]',
  ) as unknown as HTMLButtonElement
}

/**
 * jsdom never fires slotchange, so a wa-select whose wa-option children were
 * added after connect keeps an EMPTY option cache (getAllOptions caches the
 * first empty query result) and its `value` getter filters everything to
 * null. Real browsers re-index on slotchange; tests nudge re-indexing
 * explicitly before reading or setting the value.
 */
function syncWaSelect(el: SebasNewSessionDialog, testid: string): unknown {
  const sel = el.shadowRoot?.querySelector(`[data-testid="${testid}"]`) as unknown as {
    processSlotChange?: () => void
    value: string
  }
  sel.processSlotChange?.()
  return sel
}

async function pick(el: SebasNewSessionDialog, testid: string, value: string): Promise<void> {
  const sel = syncWaSelect(el, testid) as HTMLElement & { value: string }
  sel.value = value
  sel.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
  await el.updateComplete
}

beforeEach(() => {
  vi.clearAllMocks()
  ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
    agents: [
      { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
      { id: 'codex', display: 'Codex', reachable: false, cause: 'command not found' },
      { id: 'native', display: 'Native Kernel', reachable: true },
    ],
  })
  ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
  ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
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

describe('sebas-new-session-dialog', () => {
  it('confirms with agent/model/mode; mode default omits the field (null)', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'deepseek',
          models: [{ id: 'deepseek-chat', tags: [] }, { id: 'deepseek-reasoner', tags: [] }],
        },
      ],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'deepseek',
      default_model: 'deepseek-reasoner',
    })
    const el = await mount({ open: true, projectId: 'proj-a', defaultAgent: 'claude' })
    // 预选：项目 default_agent + 目录 default model。
    expect(agentSelect(el).value).toBe('claude')
    const modelSel = syncWaSelect(el, 'dialog-model-select') as HTMLElement & { value: string }
    expect(modelSel.value).toBe('deepseek-reasoner')

    const confirmed = vi.fn()
    el.addEventListener('dialog-confirm', confirmed)
    confirmButton(el).click()
    await new Promise((r) => setTimeout(r, 0))

    expect(confirmed).toHaveBeenCalledTimes(1)
    const detail = (confirmed.mock.calls[0]![0] as CustomEvent).detail as Record<
      string,
      unknown
    >
    expect(detail.agent).toBe('claude')
    expect(detail.model).toBe('deepseek-reasoner')
    expect(detail.mode).toBeNull()
  })

  it('choosing a permission mode rides on the confirm detail', async () => {
    const el = await mount({ open: true, defaultAgent: 'claude' })
    await pick(el, 'dialog-mode-select', 'edit')
    const confirmed = vi.fn()
    el.addEventListener('dialog-confirm', confirmed)
    confirmButton(el).click()
    await new Promise((r) => setTimeout(r, 0))
    expect((confirmed.mock.calls[0]![0] as CustomEvent).detail).toMatchObject({
      agent: 'claude',
      mode: 'edit',
    })
  })

  it('preselects the project default agent; first reachable agent with no record', async () => {
    const el = await mount({ open: true, defaultAgent: null })
    // 无记录 → 首个可达 agent 兜底（spec「first visit falls back honestly」）。
    expect(agentSelect(el).value).toBe('claude')

    el.defaultAgent = 'native'
    await el.updateComplete
    expect(agentSelect(el).value).toBe('native')
  })

  it('disables confirm until an agent is chosen; empty catalog disables it entirely', async () => {
    // catalog 全空 → 无 agent 可选 → 确认禁用（spec「requires an explicit agent」）。
    ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({ agents: [] })
    const el = await mount({ open: true })
    expect(confirmButton(el).disabled).toBe(true)
    expect(agentSelect(el).disabled).toBe(true)
  })

  it('two-level selection: switching provider moves the model list to that provider', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        { name: 'alpha', models: [{ id: 'a1', tags: [] }] },
        { name: 'beta', models: [{ id: 'b1', tags: [] }, { id: 'b2', tags: [] }] },
      ],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: null,
      default_model: null,
    })
    const el = await mount({ open: true, defaultAgent: 'claude' })
    // 预选目录第一对（无 default 配置）。
    const providerSel = syncWaSelect(el, 'dialog-provider-select') as HTMLElement & { value: string }
    expect(providerSel.value).toBe('alpha')
    const modelSel = syncWaSelect(el, 'dialog-model-select') as HTMLElement & { value: string }
    expect(modelSel.value).toBe('a1')

    await pick(el, 'dialog-provider-select', 'beta')
    expect((syncWaSelect(el, 'dialog-model-select') as HTMLElement & { value: string }).value).toBe('b1')

    await pick(el, 'dialog-model-select', 'b2')
    const confirmed = vi.fn()
    el.addEventListener('dialog-confirm', confirmed)
    confirmButton(el).click()
    await new Promise((r) => setTimeout(r, 0))
    expect((confirmed.mock.calls[0]![0] as CustomEvent).detail).toMatchObject({
      model: 'b2',
    })
  })

  it('a configured default outside the catalog cannot preselect (no fabricated options)', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [{ name: 'alpha', models: [{ id: 'a1', tags: [] }] }],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'ghost',
      default_model: 'nope',
    })
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const providerSel = syncWaSelect(el, 'dialog-provider-select') as HTMLElement & { value: string }
    expect(providerSel.value).toBe('alpha')
    const options = [
      ...(el.shadowRoot?.querySelectorAll('[data-testid="dialog-provider-select"] wa-option') ??
        []),
    ].map((o) => (o as HTMLElement).getAttribute('value'))
    expect(options).toEqual(['alpha'])
  })

  it('states catalog unavailability honestly instead of empty selects', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('503'), { status: 503 }),
    )
    const el = await mount({ open: true, defaultAgent: 'claude' })
    expect(el.shadowRoot?.querySelector('[data-testid="dialog-provider-select"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="dialog-model-select"]')).toBeNull()
    const hint = el.shadowRoot?.querySelector('[data-testid="dialog-catalog-unavailable"]')
    expect(hint?.textContent).toContain('模型目录不可用')
    // agent 选择不受目录影响。
    expect(agentSelect(el).value).toBe('claude')
  })

  it('cancel dispatches dialog-cancel and creates nothing', async () => {
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const cancelled = vi.fn()
    const confirmed = vi.fn()
    el.addEventListener('dialog-cancel', cancelled)
    el.addEventListener('dialog-confirm', confirmed)
    ;(el.shadowRoot?.querySelector('[data-testid="dialog-cancel"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(cancelled).toHaveBeenCalledTimes(1)
    expect(confirmed).not.toHaveBeenCalled()
  })

  it('unreachable agents are listed disabled with their cause', async () => {
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const codex = el.shadowRoot?.querySelector(
      'wa-option[value="codex"]',
    ) as HTMLElement | null
    expect(codex).toBeTruthy()
    expect(codex!.hasAttribute('disabled')).toBe(true)
    expect(codex?.textContent ?? '').toContain('unavailable')
    expect(codex?.textContent ?? '').toContain('command not found')
  })

  it('reopen resets the mode to the agent-default default', async () => {
    const el = await mount({ open: true, defaultAgent: 'claude' })
    await pick(el, 'dialog-mode-select', 'auto')
    const modeSel = () =>
      el.shadowRoot?.querySelector('[data-testid="dialog-mode-select"]') as unknown as {
        value: string
      }
    expect(modeSel().value).toBe('auto')
    // 关闭再打开（rail 复用同一实例）：mode 回到缺省。
    el.open = false
    await el.updateComplete
    el.open = true
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    expect(modeSel().value).toBe('')
  })
})
