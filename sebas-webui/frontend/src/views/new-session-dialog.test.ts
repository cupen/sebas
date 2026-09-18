// @vitest-environment jsdom
/**
 * sebas-new-session-dialog（workbench-interaction-polish 3.1，design D2；
 * 预选语义改自 preselect-last-used-model 1.2）。
 *
 * Covered:
 *   - 禁用：无 agent 可选（catalog 为空）→ 确认禁用；未选 agent → 确认禁用
 *   - 预选级联：上次确认的 (provider, model) 对仍在目录 → 该对；无记忆/
 *     stale 记忆 → 目录第一对（不伪造选项）；配置 defaults 不再参与
 *   - 确认写记忆：dialog-confirm 时 saveLastUsedPair（唯一写入点）
 *   - 取消：dialog-cancel 事件、不创建任何东西（无创建 API 调用）
 *   - 确认：dialog-confirm detail 携带 agent/model/mode；mode 缺省 = null
 *     （wire 上省略），选 edit = 'edit'
 *   - 目录不可得：显式引导（Settings → Models），无 provider/model 下拉
 *   - 不可达 agent：禁选并标注 cause
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasNewSessionDialog } from './new-session-dialog.js'
import { LAST_USED_PAIR_KEY } from '../api/model-catalog.js'
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
    providers: vi.fn(),
    providerDefaults: vi.fn(),
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
  localStorage.removeItem(LAST_USED_PAIR_KEY)
  ;(api.agents as ReturnType<typeof vi.fn>).mockResolvedValue({
    agents: [
      { id: 'claude', display: 'Claude Code', reachable: true, version: 'v1' },
      { id: 'codex', display: 'Codex', reachable: false, cause: 'command not found' },
      { id: 'native', display: 'Native Kernel', reachable: true },
    ],
  })
  ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
  ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
    default_provider: null,
    default_model: null,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
  localStorage.removeItem(LAST_USED_PAIR_KEY)
  if (!elementInternalsPolyfillInvoked()) {
    throw new Error('ElementInternals polyfill was never invoked')
  }
})

describe('sebas-new-session-dialog', () => {
  it('confirms with agent/model/mode; mode default omits the field (null)', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        {
          name: 'deepseek',
          models: [{ id: 'deepseek-chat', tags: [] }, { id: 'deepseek-reasoner', tags: [] }],
        },
      ],
    })
    const el = await mount({ open: true, projectId: 'proj-a', defaultAgent: 'claude' })
    // 预选：项目 default_agent + 无 last-used 记忆 → 目录第一对（配置的
    // defaults 不再参与预选）。
    expect(agentSelect(el).value).toBe('claude')
    const modelSel = syncWaSelect(el, 'dialog-model-select') as HTMLElement & { value: string }
    expect(modelSel.value).toBe('deepseek-chat')

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
    expect(detail.model).toBe('deepseek-chat')
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

  it('mode dropdown renders the shared MODE_OPTIONS vocabulary (4.1)', async () => {
    // polish-workbench-walkthrough-ux 4.1：创建弹窗与 composer 下拉同源——
    // 选项来自共享 MODE_OPTIONS（值 + 中文解释一致），杜绝「一边裸词一边
    // 带解释」的漂移。
    const { MODE_OPTIONS } = await import('./mode-vocabulary.js')
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const modeSel = el.shadowRoot!.querySelector('[data-testid="dialog-mode-select"]')
    const options = Array.from(modeSel?.querySelectorAll('wa-option') ?? []).map((o) => ({
      value: o.getAttribute('value'),
      label: o.textContent ?? '',
    }))
    // 缺省项（agent 默认，wire 省略 mode）+ 四个共享模式。
    expect(options[0]!.value).toBe('')
    expect(options.slice(1).map((o) => o.value)).toEqual(MODE_OPTIONS.map((m) => m.value))
    expect(options.slice(1).map((o) => o.label)).toEqual(MODE_OPTIONS.map((m) => m.label))
    el.remove()
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
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        { name: 'alpha', models: [{ id: 'a1', tags: [] }] },
        { name: 'beta', models: [{ id: 'b1', tags: [] }, { id: 'b2', tags: [] }] },
      ],
    })
    ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
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

  it('preselect cascade: a remembered pair still in the catalog wins the preselection', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        { name: 'alpha', models: [{ id: 'a1', tags: [] }, { id: 'a2', tags: [] }] },
        { name: 'beta', models: [{ id: 'b1', tags: [] }] },
      ],
    })
    // 上次确认了 beta / b1（全局记忆）→ 重开对话框预选该对，而非目录第一对。
    localStorage.setItem(LAST_USED_PAIR_KEY, JSON.stringify({ provider: 'beta', model: 'b1' }))
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const providerSel = syncWaSelect(el, 'dialog-provider-select') as HTMLElement & { value: string }
    expect(providerSel.value).toBe('beta')
    expect((syncWaSelect(el, 'dialog-model-select') as HTMLElement & { value: string }).value).toBe('b1')
  })

  it('preselect cascade: a vanished remembered pair falls back to the first pair without fabricating it as an option', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [{ name: 'alpha', models: [{ id: 'a1', tags: [] }] }],
    })
    // stale 记忆（provider 已不在目录）→ 落目录第一对，绝不把 stale 对渲染
    // 成选项。
    localStorage.setItem(LAST_USED_PAIR_KEY, JSON.stringify({ provider: 'ghost', model: 'nope' }))
    const el = await mount({ open: true, defaultAgent: 'claude' })
    const providerSel = syncWaSelect(el, 'dialog-provider-select') as HTMLElement & { value: string }
    expect(providerSel.value).toBe('alpha')
    const options = [
      ...(el.shadowRoot?.querySelectorAll('[data-testid="dialog-provider-select"] wa-option') ??
        []),
    ].map((o) => (o as HTMLElement).getAttribute('value'))
    expect(options).toEqual(['alpha'])
  })

  it('confirming a creation writes the last-used memory (the only write point)', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [
        { name: 'alpha', models: [{ id: 'a1', tags: [] }, { id: 'a2', tags: [] }] },
      ],
    })
    const el = await mount({ open: true, defaultAgent: 'claude' })
    expect(localStorage.getItem(LAST_USED_PAIR_KEY)).toBeNull()

    await pick(el, 'dialog-model-select', 'a2')
    const confirmed = vi.fn()
    el.addEventListener('dialog-confirm', confirmed)
    confirmButton(el).click()
    await new Promise((r) => setTimeout(r, 0))

    expect(confirmed).toHaveBeenCalledTimes(1)
    expect(JSON.parse(localStorage.getItem(LAST_USED_PAIR_KEY) ?? 'null')).toEqual({
      provider: 'alpha',
      model: 'a2',
    })
  })

  it('cancel writes no last-used memory', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [{ name: 'alpha', models: [{ id: 'a1', tags: [] }] }],
    })
    const el = await mount({ open: true, defaultAgent: 'claude' })
    ;(el.shadowRoot?.querySelector('[data-testid="dialog-cancel"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(localStorage.getItem(LAST_USED_PAIR_KEY)).toBeNull()
  })

  it('states catalog unavailability honestly instead of empty selects', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('503'), { status: 503 }),
    )
    const el = await mount({ open: true, defaultAgent: 'claude' })
    expect(el.shadowRoot?.querySelector('[data-testid="dialog-provider-select"]')).toBeNull()
    expect(el.shadowRoot?.querySelector('[data-testid="dialog-model-select"]')).toBeNull()
    const hint = el.shadowRoot?.querySelector('[data-testid="dialog-catalog-unavailable"]')
    // 显式引导 + （4.4）不暗示创建被禁：说明仍可用 agent 默认模型创建。
    expect(hint?.textContent).toContain('尚未配置 provider 模型')
    expect(hint?.textContent).toContain('Settings → Models')
    expect(hint?.textContent).toContain('默认模型')
    // 创建按钮不因目录为空而禁用（仅 agent 必选门禁）。
    const confirm = el.shadowRoot?.querySelector(
      '[data-testid="dialog-confirm"]',
    ) as HTMLElement | null
    expect(confirm?.hasAttribute('disabled')).toBe(false)
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
    // （polish-workbench-walkthrough-ux 4.3）默认可见文案 = 操作者语言 +
    // 补救入口；实现性成因（cause）只进 tooltip。
    expect(codex?.textContent ?? '').toContain('不可用 — 到 Settings → Models 检查配置')
    expect((codex as unknown as { title: string }).title).toContain('command not found')
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
