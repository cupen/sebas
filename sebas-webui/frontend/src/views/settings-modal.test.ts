// @vitest-environment jsdom
/**
 * Settings modal (IA v3, revamp-settings-nav-and-models-editor)：左侧分区
 * 导航 + 右侧内容。五个分区（顺序即规约）——
 *   - generic    通用偏好与杂项：原 Env 分区的环境变量只读表（值一律
 *                "managed by core config"）；语言切换仅预留信息架构位置
 *   - appearance 主题三态（system/dark/light，走真实 theme.ts）
 *   - services   watchdog 受管子进程（/api/admin/services 经 adminServicesSafe：
 *                name / desired / actual / uptime + /api/admin/events 最近错误；
 *                im→「飞书 IM」显示映射；无 adapter 时「无 watchdog 控制面」
 *                横幅而非空列表冒充——router 行的 desired/actual/uptime 只在
 *                此分区呈现）。停止被拒（400/409 + active_routed_sessions +
 *                count，unify-router-process-shape D4）→ 二层强制出口对话框：
 *                计数 + 流式中断后果，Force stop 以 force 重发、取消不发请求；
 *                其余失败仍走既有内联错误
 *   - models     provider 管理列表（/router/api/providers，条目携带能力
 *                标记；不再渲染 Router 网关卡、不再请求 /api/router）
 *   - about      INSTANCE 段在上（原 Settings 总览三只读项：工作区根目录 +
 *                复制、default agent kind、default provider/model + 跳转
 *                Models），BUILD 段在下（/api/about 真实字段）
 * 原 `Settings` 总览分区移除：「全部进程重启」「重置 Settings」两个高危
 * 动作随之删除（任何分区都不得再出现入口）。
 * 缺省首项 generic；上次分区记忆走 localStorage `lastSettingsSection`
 * （缺值/非法值——含旧值 `settings`/`env`——回退 generic）。关闭交互
 * （按钮 / Esc / 遮罩）一并覆盖；子 `<wa-select>` 冒泡的 `wa-hide` 不得
 * 连带关闭对话框（来源守卫）。
 * fetch 交互（revamp-settings-nav-and-models-editor，取代 add-fetch-models
 * 的行内 🔍 + 结果列表挑选流）：入口在编辑器「Models」区块标题旁（仅编辑
 * 既有 provider 且有可用 base URL 时渲染）；成功整单替换草稿列表（按 id
 * 去重、同 id 保留人工 tags），保存与否走普通编辑流；失败保留草稿并内联
 * 呈现净化原因。
 * api client 全量 mock（含前序 agent 留下的 admin stubs，本轮沿用）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ---- WA 渲染垫片（共享）--------------------------------------------------
// 本文件渲染 WA 表单/弹窗组件：jsdom 的 ElementInternals 缺 setValidity、
// HTMLDialogElement 缺 showModal、Element 缺 getAnimations，都会抛未处理
// rejection。共享实现见 test-support/wa-polyfills.ts。
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

installWaDomPolyfills()

// jsdom 这里不提供 localStorage（about:blank origin），沿用仓库的内存
// polyfill 约定（见 transcript-view.test.ts）；Appearance 分区的主题
// 持久化走真实 theme.ts，分区记忆走 `lastSettingsSection`，都需要它真实可读写。
const themeStore = new Map<string, string>()
const themeLs = {
  getItem: (k: string) => themeStore.get(k) ?? null,
  setItem: (k: string, v: string) => void themeStore.set(k, v),
  removeItem: (k: string) => void themeStore.delete(k),
  clear: () => themeStore.clear(),
  key: () => null,
  get length() {
    return themeStore.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: themeLs, configurable: true })
beforeEach(() => themeStore.clear())

const apiMocks = vi.hoisted(() => ({
  router: vi.fn(),
  about: vi.fn(),
  routerProviders: vi.fn(),
  routerPresets: vi.fn(),
  routerProviderCreate: vi.fn(),
  routerProviderUpdate: vi.fn(),
  routerProviderDelete: vi.fn(),
  fetchProviderModels: vi.fn(),
  adminServices: vi.fn(),
  adminEvents: vi.fn(),
  adminServicesSafe: vi.fn(),
  adminEventsSafe: vi.fn(),
  enableService: vi.fn(),
  disableService: vi.fn(),
  restartService: vi.fn(),
  fsBrowseDirs: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  // 与真 ApiError 同形（client.ts）：status + 机器可读拒绝码 code + 数值
  // 载荷 count（unify-router-process-shape 2.3 的 400 拒绝经此携带）。
  ApiError: class ApiError extends Error {
    readonly status: number
    readonly code: string | null
    readonly count: number | null
    constructor(
      status: number,
      message: string,
      code: string | null = null,
      count: number | null = null,
    ) {
      super(message)
      this.status = status
      this.code = code
      this.count = count
    }
  },
  api: {
    router: apiMocks.router,
    about: apiMocks.about,
    routerProviders: apiMocks.routerProviders,
    routerPresets: apiMocks.routerPresets,
    routerProviderCreate: apiMocks.routerProviderCreate,
    routerProviderUpdate: apiMocks.routerProviderUpdate,
    routerProviderDelete: apiMocks.routerProviderDelete,
    fetchProviderModels: apiMocks.fetchProviderModels,
    adminServices: apiMocks.adminServices,
    adminEvents: apiMocks.adminEvents,
    adminServicesSafe: apiMocks.adminServicesSafe,
    adminEventsSafe: apiMocks.adminEventsSafe,
    enableService: apiMocks.enableService,
    disableService: apiMocks.disableService,
    restartService: apiMocks.restartService,
    fsBrowseDirs: apiMocks.fsBrowseDirs,
  },
}))

import './settings-modal.js'
import type { SebasSettingsModal } from './settings-modal.js'
// mocked 模块里的 ApiError 类——与组件内 instanceof 同一构造器。
import { ApiError } from '../api/client.js'
import { applyThemeMode } from '../theme.js'

async function mount(open = true): Promise<SebasSettingsModal> {
  const el = document.createElement('sebas-settings-modal') as SebasSettingsModal
  el.open = open
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

/** Drain the microtask queue so child-component fetches settle. */
async function settle(el: SebasSettingsModal): Promise<void> {
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
}

function navItems(el: SebasSettingsModal): HTMLElement[] {
  return [...el.shadowRoot!.querySelectorAll<HTMLElement>('.nav .nav-item')]
}

/** Click a nav entry by index and wait for its lazy loads to settle. */
async function goto(el: SebasSettingsModal, index: number): Promise<void> {
  navItems(el)[index]!.click()
  await el.updateComplete
  await settle(el)
}

function waButtons(el: SebasSettingsModal): HTMLElement[] {
  return [...el.shadowRoot!.querySelectorAll<HTMLElement>('wa-button')]
}

beforeEach(() => {
  // 调用计数跨 test 累积会污染“未调用”断言（如 Services 不得调 /api/router），
  // 先清计数再装实现（clear 只清 calls，不动实现）。
  vi.clearAllMocks()
  apiMocks.router.mockResolvedValue({
    router: {
      listen: '127.0.0.1:8787',
      provider_count: 2,
      debug: false,
      has_auth: true,
      providers: [],
    },
  })
  apiMocks.routerProviders.mockResolvedValue({
    providers: [
      {
        name: 'alpha',
        preset: 'deepseek',
        base_url_anthropic: 'https://a.example/anthropic',
        base_url_openai_chat: 'https://a.example/v1',
        base_url_openai_responses: null,
        api_key_env: 'DEEPSEEK_API_KEY',
        api_key_configured: true,
        models: [
          { id: 'm1', tags: [] },
          { id: 'm2', tags: ['vision'] },
        ],
      },
      {
        name: 'beta',
        preset: null,
        base_url_anthropic: null,
        base_url_openai_chat: 'https://b.example/v1',
        base_url_openai_responses: null,
        api_key_env: null,
        api_key_configured: false,
        models: [],
      },
    ],
  })
  apiMocks.adminServices.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEvents.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.enableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.disableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.restartService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.fsBrowseDirs.mockResolvedValue({ path: '/tmp/test-work', entries: [] })
  apiMocks.routerPresets.mockResolvedValue({
    presets: [
      {
        name: 'deepseek',
        base_url_anthropic: 'https://api.deepseek.com/anthropic',
        base_url_openai_chat: 'https://api.deepseek.com',
        base_url_openai_responses: null,
        api_key_env: 'DEEPSEEK_API_KEY',
        models: [{ id: 'deepseek-chat', tags: [] }],
      },
    ],
  })
  apiMocks.about.mockResolvedValue({
    uptime: '3h 12m',
    version: '0.4.2',
    rustc_version: '1.88',
    router_listen: '127.0.0.1:8787',
    provider_count: 2,
  })
})

afterEach(() => {
  document.body.innerHTML = ''
  localStorage.removeItem('sebas:theme')
  localStorage.removeItem('lastSettingsSection')
  document.documentElement.classList.remove('wa-dark')
})

describe('sebas-settings-modal sections', () => {
  it('renders the left nav with exactly Generic/Appearance/Services/Models/About', async () => {
    const el = await mount()
    const labels = navItems(el).map((b) => b.textContent?.trim())
    expect(labels).toEqual(['Generic', 'Appearance', 'Services', 'Models', 'About'])
    el.remove()
  })

  it('separates the nav into groups: a break before Services and a tail break pinning About to the bottom', async () => {
    const el = await mount()
    const nav = el.shadowRoot!.querySelector('.nav')!
    const seps = [...nav.querySelectorAll<HTMLElement>('.nav-sep')]
    expect(seps.length).toBe(2)
    // About 上方的分隔线带 .tail（margin-top: auto 压底），且紧贴 About 项。
    const tail = seps.find((s) => s.classList.contains('tail'))!
    expect(tail).toBeTruthy()
    expect(tail.nextElementSibling?.classList.contains('nav-item')).toBe(true)
    expect(tail.nextElementSibling?.textContent?.trim()).toBe('About')
    // 另一条在 Services 项之前（appearance|services 组间线）。
    const plain = seps.find((s) => !s.classList.contains('tail'))!
    expect(plain.nextElementSibling?.textContent?.trim()).toBe('Services')
    expect(plain.previousElementSibling?.textContent?.trim()).toBe('Appearance')
    el.remove()
  })

  it('defaults to the Generic section rendering the env reference table', async () => {
    const el = await mount()
    expect(el.section).toBe('generic')
    await settle(el)
    const rows = [...el.shadowRoot!.querySelectorAll('.env-table tbody tr')]
    expect(rows.length).toBeGreaterThan(0)
    expect(el.shadowRoot!.textContent).toContain('SEBAS_ROUTER_LISTEN')
    // Every row's value is the "not exposed" marker — no fabricated data.
    for (const row of rows) {
      expect(row.querySelector('.value')?.textContent).toBe('managed by core config')
    }
    el.remove()
  })

  it('renders no maintenance actions anywhere (restart-all and reset retired)', async () => {
    const el = await mount()
    for (const index of [0, 1, 2, 3, 4]) {
      await goto(el, index)
      const buttons = waButtons(el).map((b) => b.textContent?.trim())
      expect(buttons).not.toContain('全部进程重启')
      expect(buttons).not.toContain('重置 Settings')
      expect(el.shadowRoot!.querySelector('.danger-zone')).toBeNull()
    }
    el.remove()
  })

  it('remembers lastSettingsSection; legacy settings/env values fall back to generic', async () => {
    localStorage.setItem('lastSettingsSection', 'services')
    const el = await mount()
    await settle(el)
    expect(el.section).toBe('services')
    // 打开期间切换分区会写回记忆。
    await goto(el, 3)
    expect(localStorage.getItem('lastSettingsSection')).toBe('models')
    el.remove()

    // 本变更前的合法分区名（Settings 总览、Env）现在都是非法值 → 回退缺省。
    for (const legacy of ['settings', 'env', 'nope']) {
      localStorage.setItem('lastSettingsSection', legacy)
      const el2 = await mount()
      await settle(el2)
      expect(el2.section).toBe('generic')
      el2.remove()
    }
  })

  it('Services section renders rows from /api/admin/services with im→飞书 IM mapping', async () => {
    apiMocks.router.mockClear()
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'im', status: 'running', desired: 'running', uptime_secs: 3725 },
        { name: 'router', status: 'stopped', desired: 'stopped', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({
      adapter_ok: true,
      events: [{ seq: 1, operation_id: 'op-1', kind: 'service_error', message: 'im worker boom' }],
    })
    const el = await mount()
    await goto(el, 2)
    expect(el.section).toBe('services')
    // 真源是 adminServicesSafe；renderServices 不得再读 /api/router。
    expect(apiMocks.adminServicesSafe).toHaveBeenCalled()
    expect(apiMocks.adminEventsSafe).toHaveBeenCalled()
    expect(apiMocks.router).not.toHaveBeenCalled()
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    expect(cards.length).toBe(2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('飞书 IM')
    // 内部名锚点保留，供与 /api/admin/services 对账。
    const ids = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-id')].map((s) =>
      s.textContent?.trim(),
    )
    expect(ids).toEqual(['im', 'router'])
    expect(text).toContain('desired running · status running · up 1h 2m')
    expect(text).toContain('Recent errors')
    expect(text).toContain('im worker boom')
    // 每行带 enable/disable/restart 动作钮。
    const enables = [...el.shadowRoot!.querySelectorAll('button[title="Enable service"]')]
    expect(enables.length).toBe(2)
    el.remove()
  })

  it('core row offers only restart — no enable/disable (always started)', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'core', status: 'running', desired: 'running', uptime_secs: 60 },
        { name: 'router', status: 'stopped', desired: 'stopped', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, 2)
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    const coreCard = cards.find((c) => c.querySelector('.service-id')?.textContent === 'core')!
    expect(coreCard).toBeTruthy()
    expect(coreCard.querySelector('button[title="Enable service"]')).toBeNull()
    expect(coreCard.querySelector('button[title="Disable service"]')).toBeNull()
    expect(coreCard.querySelector('button[title="Restart service"]')).not.toBeNull()
    el.remove()
  })

  it('Services section shows the no-adapter banner without rows or actions', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, 2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('无 watchdog 控制面')
    expect(el.shadowRoot!.querySelectorAll('.service-card').length).toBe(0)
    expect(el.shadowRoot!.querySelectorAll('.service-actions button').length).toBe(0)
    el.remove()
  })

  it('Services section is the only place that renders router desired/actual/uptime (4.1)', async () => {
    // 4.1：router 行在 Services 呈现 desired / actual / uptime；Models 分区
    // 不出现任何 router 运行状态。adapter_ok:false 时横幅呈现而非空列表冒充
    // （专属用例见下）。
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status: 'running', desired: 'running', uptime_secs: 95 }],
    })
    const el = await mount()
    await goto(el, 2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('router')
    expect(text).toContain('desired running · status running · up 1m')
    el.remove()
  })

  it('Models section renders the provider list only — no gateway card, no /api/router, no row-level fetch', async () => {
    const el = await mount()
    apiMocks.router.mockClear()
    await goto(el, 3)
    expect(el.section).toBe('models')
    // 3.4：Models 渲染不再发起 /api/router 请求，也不出现网关卡。
    expect(apiMocks.router).not.toHaveBeenCalled()
    expect(apiMocks.routerProviders).toHaveBeenCalled()
    expect(apiMocks.routerPresets).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).not.toContain('Router 路由网关')
    expect(text).not.toContain('127.0.0.1:8787')
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    expect(rows.length).toBe(2)
    expect(text).toContain('alpha')
    expect(text).toContain('deepseek · code')
    expect(text).toContain('custom')
    expect(text).toContain('https://a.example/anthropic')
    expect(text).toContain('beta')
    // 列表呈现模型条目与能力标记（条目 = id + tags；text 隐含）。
    const chips = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.model-chip')]
    expect(chips.map((c) => (c.textContent ?? '').replace(/\s+/g, ''))).toContain('m2vision')
    // revamp…4.1：provider 行不再带 🔍（入口移进编辑器）。
    expect(el.shadowRoot!.querySelector('button[data-testid="fetch-models"]')).toBeNull()
    el.remove()
  })

  it('About section shows INSTANCE (overview items) above BUILD (/api/about)', async () => {
    const el = await mount()
    await goto(el, 4)
    expect(el.section).toBe('about')
    expect(apiMocks.about).toHaveBeenCalled()
    expect(apiMocks.fsBrowseDirs).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    // INSTANCE 段：原 Settings 总览三只读项。
    expect(text).toContain('Instance')
    expect(text).toContain('Workspace root')
    expect(text).toContain('/tmp/test-work')
    expect(text).toContain('Default agent kind')
    expect(text).toContain('acp')
    expect(text).toContain('Default provider / model')
    expect(text).toContain('— (set one in Models)')
    // BUILD 段：/api/about 真实字段。
    expect(text).toContain('Build')
    expect(text).toContain('0.4.2')
    expect(text).toContain('3h 12m')
    expect(text).toContain('1.88')
    expect(text).toContain('127.0.0.1:8787')
    // 两段dl分开：instance 在前、build 在后（DOM 顺序即呈现顺序）。
    const lists = [...el.shadowRoot!.querySelectorAll('dl.about-list')]
    expect(lists.length).toBe(2)
    expect(lists[0]!.classList.contains('about-instance')).toBe(true)
    expect(lists[1]!.classList.contains('about-build')).toBe(true)
    // 工作区根目录带复制按钮。
    const copy = el.shadowRoot!.querySelector('button[title="Copy workspace root"]')
    expect(copy).toBeTruthy()
    el.remove()
  })

  it('aria-current tracks the active section', async () => {
    const el = await mount()
    expect(navItems(el)[0]!.getAttribute('aria-current')).toBe('true')
    navItems(el)[1]!.click()
    await el.updateComplete
    expect(navItems(el)[0]!.getAttribute('aria-current')).toBe('false')
    expect(navItems(el)[1]!.getAttribute('aria-current')).toBe('true')
    el.remove()
  })
})

describe('unify-router-process-shape：router 停止被拒的强制出口（D4）', () => {
  /** 装好带 router 行的 Services 分区，并在 confirm 弹窗里点掉 Disable。 */
  async function confirmDisableRouter(el: SebasSettingsModal): Promise<void> {
    await goto(el, 2)
    const routerCard = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')].find(
      (c) => c.querySelector('.service-id')?.textContent === 'router',
    )!
    routerCard.querySelector<HTMLElement>('button[title="Disable service"]')!.click()
    await el.updateComplete
    const confirm = el.shadowRoot!.querySelector(
      'wa-dialog.service-action-confirm',
    ) as HTMLElement
    confirm.querySelector<HTMLElement>('wa-button[variant="danger"]')!.click()
    await settle(el)
  }

  function forceDialog(el: SebasSettingsModal): HTMLElement {
    return el.shadowRoot!.querySelector('wa-dialog.service-force-stop') as HTMLElement
  }

  /**
   * 对话框开合的真源断言：组件 state `forceStop`。wa-dialog 的 open 属性
   * 回落走异步 requestClose 动画链（jsdom 无动画完成事件，时序不保证），
   * 与本文件既有 `editor` 断言同款读组件 state。
   */
  function forceStopState(el: SebasSettingsModal): { name: string; count: number | null } | null {
    return (el as unknown as { forceStop: { name: string; count: number | null } | null })
      .forceStop
  }

  /** 拒绝载荷（wire 合同主形态的解析产物：ApiError 携带 code + count）。 */
  function rejection(count: number): Error {
    return new ApiError(400, 'router has active routed sessions', 'active_routed_sessions', count)
  }

  beforeEach(() => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status: 'running', desired: 'running', uptime_secs: 30 }],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
  })

  it('直接成功：首次 disable 不带 force，成功后刷新列表、不弹强制出口', async () => {
    const el = await mount()
    await confirmDisableRouter(el)
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    expect(apiMocks.disableService).toHaveBeenCalledWith('router', false)
    expect(forceStopState(el)).toBeNull()
    // 成功 → loadServices 重取列表（初载 + 动作后 = 2 次）。
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('拒绝后强制：二层对话框呈现计数与后果，Force stop 以 force 重发并刷新', async () => {
    apiMocks.disableService
      .mockRejectedValueOnce(rejection(2))
      .mockResolvedValueOnce({ operation_id: 'op-force', status: 'accepted', message: 'accepted' })
    const el = await mount()
    await confirmDisableRouter(el)
    // 二层对话框打开：拒绝驱动，携带目标与计数；拒绝不落内联错误。
    expect(forceStopState(el)).toEqual({ name: 'router', count: 2 })
    expect(forceDialog(el).textContent).toContain('2')
    expect(forceDialog(el).textContent).toContain('streaming')
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeNull()
    // 「强制停止」= 同一停止请求带 force: true 重发；成功后刷新列表。
    forceDialog(el).querySelector<HTMLElement>('wa-button[variant="danger"]')!.click()
    await settle(el)
    expect(apiMocks.disableService).toHaveBeenCalledTimes(2)
    expect(apiMocks.disableService).toHaveBeenLastCalledWith('router', true)
    expect(forceStopState(el)).toBeNull()
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('拒绝后取消：不发任何请求，router 行保持原状', async () => {
    apiMocks.disableService.mockRejectedValueOnce(rejection(5))
    const el = await mount()
    await confirmDisableRouter(el)
    expect(forceStopState(el)).toEqual({ name: 'router', count: 5 })
    forceDialog(el).querySelector<HTMLElement>('wa-button[appearance="plain"]')!.click()
    await settle(el)
    // 取消 = 无第二次请求、列表不刷新、对话框关闭且行保持原状。
    expect(forceStopState(el)).toBeNull()
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(1)
    expect(el.shadowRoot!.textContent ?? '').toContain('status running')
    el.remove()
  })

  it('400 但无 active_routed_sessions 的失败仍走既有内联错误，不弹强制出口', async () => {
    apiMocks.disableService.mockRejectedValueOnce(new ApiError(400, 'watchdog rejected it'))
    const el = await mount()
    await confirmDisableRouter(el)
    expect(forceStopState(el)).toBeNull()
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeTruthy()
    expect(el.shadowRoot!.textContent ?? '').toContain('watchdog rejected it')
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    el.remove()
  })
})

describe('sebas-settings-modal appearance section', () => {
  beforeEach(() => {
    // jsdom 对 matchMedia 的支持不保证存在/一致；显式钉死为深色 OS，
    // 让 "System 解析为 dark" 成为确定性断言。
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function themeOptions(el: SebasSettingsModal): HTMLElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLElement>('.theme-option')]
  }

  it('offers System/Dark/Light with System pressed by default (stubbed OS → dark)', async () => {
    // 页面启动时由 main.ts 应用一次主题 class；测试环境里手动补上。
    applyThemeMode()
    const el = await mount()
    await goto(el, 1)
    const options = themeOptions(el)
    expect(options.map((b) => b.querySelector('.theme-option-label')?.textContent)).toEqual([
      'System',
      'Dark',
      'Light',
    ])
    expect(options[0]!.getAttribute('aria-pressed')).toBe('true')
    expect(options[1]!.getAttribute('aria-pressed')).toBe('false')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    el.remove()
  })

  it('choosing Light unsets wa-dark and persists sebas:theme=light', async () => {
    const el = await mount()
    await goto(el, 1)
    themeOptions(el)[2]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('light')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
    expect(themeOptions(el)[2]!.getAttribute('aria-pressed')).toBe('true')
    expect(el.shadowRoot!.textContent).toContain('Applied immediately, saved for this browser.')
    el.remove()
  })

  it('choosing Dark sets wa-dark and persists; System returns to following the OS', async () => {
    const el = await mount()
    await goto(el, 1)
    themeOptions(el)[1]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('dark')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    // 回到 system：jsdom 无 matchMedia → 跟随解析为 dark。
    themeOptions(el)[0]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('system')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    expect(el.shadowRoot!.textContent).toContain('Your OS currently asks for dark')
    el.remove()
  })
})

describe('sebas-settings-modal closing', () => {
  it('the close button shuts it and bubbles the close event', async () => {
    const el = await mount()
    const closed = vi.fn()
    el.addEventListener('close', closed)
    el.shadowRoot!.querySelector<HTMLButtonElement>('.close')!.click()
    await el.updateComplete
    expect(el.open).toBe(false)
    expect(closed).toHaveBeenCalledTimes(1)
    expect(el.shadowRoot!.querySelector('.panel')).toBeNull()
    el.remove()
  })

  it('Escape closes it', async () => {
    const el = await mount()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await el.updateComplete
    expect(el.open).toBe(false)
    el.remove()
  })

  it('clicking the backdrop (not the panel) closes it', async () => {
    const el = await mount()
    const overlay = el.shadowRoot!.querySelector('.overlay') as HTMLElement
    overlay.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(el.open).toBe(false)
    el.remove()
  })

  it('a closed modal renders nothing', async () => {
    const el = await mount(false)
    expect(el.shadowRoot!.querySelector('.panel')).toBeNull()
    el.remove()
  })
})

it('states honestly that no global default is set (agent-defaults retired)', async () => {
  // workbench-agent-wire-fix 3.3：/api/agent-defaults 端点退役——provider
  // 列表不再渲染全局 default 徽章（默认 agent 改为项目级记忆）。

  const el = await mount()
  await goto(el, 3)

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('no default set')
  expect(el.shadowRoot?.querySelector('.provider-badge.default')).toBeNull()
  el.remove()
})

describe('revamp-settings-nav-and-models-editor：编辑器内 fetch 整单替换', () => {
  /** Models 分区里指定 provider 的行。 */
  function rowFor(el: SebasSettingsModal, name: string): HTMLElement {
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    const row = rows.find(
      (r) => r.querySelector('.provider-row-name')?.textContent?.trim() === name,
    )
    expect(row).toBeTruthy()
    return row!
  }

  /** 打开指定 provider 的编辑器（行内 ✎）。 */
  async function openEditorFor(el: SebasSettingsModal, name: string): Promise<HTMLElement> {
    await goto(el, 3)
    rowFor(el, name).querySelector<HTMLButtonElement>('button[title="Edit"]')!.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
    expect(dialog).toBeTruthy()
    return dialog
  }

  /** 新建（preset/custom）编辑器。 */
  async function openCreateEditor(el: SebasSettingsModal, label: string): Promise<HTMLElement> {
    await goto(el, 3)
    waButtons(el)
      .find((b) => b.textContent?.includes(label))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  function editorFetchButton(dialog: HTMLElement): HTMLButtonElement | null {
    return dialog.querySelector<HTMLButtonElement>('button[data-testid="fetch-models"]')
  }

  function entryRows(dialog: HTMLElement): HTMLElement[] {
    return [...dialog.querySelectorAll<HTMLElement>('[data-testid="model-entry"]')]
  }

  function entryIds(dialog: HTMLElement): string[] {
    return entryRows(dialog).map(
      (r) => (r.querySelector('wa-input') as unknown as { value: string }).value,
    )
  }

  async function saveEditor(el: SebasSettingsModal): Promise<void> {
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
  }

  beforeEach(() => {
    apiMocks.fetchProviderModels.mockResolvedValue({ provider: 'beta', models: ['m-pro', 'm-flash'] })
    apiMocks.routerProviderUpdate.mockResolvedValue({ updated: 'beta' })
  })

  // 4.1 验收：入口从 provider 行内挪进编辑器——行上无 🔍，编辑器（preset
  // 派生与 custom 皆然）有，且点击即调 core 的抓取 op。
  it('moves the fetch entry into the editor for preset and custom providers alike', async () => {
    const el = await mount()
    await goto(el, 3)
    expect(apiMocks.fetchProviderModels).not.toHaveBeenCalled()
    // 行内没有任何 fetch 按钮（含有可用 base URL 的 alpha/beta）。
    expect(el.shadowRoot!.querySelector('button[data-testid="fetch-models"]')).toBeNull()

    for (const name of ['beta', 'alpha']) {
      const dialog = await openEditorFor(el, name)
      const btn = editorFetchButton(dialog)
      expect(btn).toBeTruthy()
      btn!.click()
      await settle(el)
      expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith(name)
      // 关掉再开下一个。
      ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
      await el.updateComplete
    }
    el.remove()
  })

  // 4.2/D3 验收：成功 → 整单替换编辑器草稿列表；抓取零写请求；保存才落库。
  it('replaces the editor draft wholesale; storage moves only through the normal save', async () => {
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    expect(entryRows(dialog).length).toBe(0)

    editorFetchButton(dialog)!.click()
    await settle(el)

    expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith('beta')
    expect(apiMocks.routerProviderUpdate).not.toHaveBeenCalled()
    expect(apiMocks.routerProviderCreate).not.toHaveBeenCalled()
    expect(entryIds(dialog)).toEqual(['m-pro', 'm-flash'])

    await saveEditor(el)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledTimes(1)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledWith('beta', {
      protocol: 'auto',
      base_url_anthropic: undefined,
      base_url_openai_chat: 'https://b.example/v1',
      base_url_openai_responses: undefined,
      api_key_env: undefined,
      default_model: undefined,
      model_map: undefined,
      models: [
        { id: 'm-pro', tags: [] },
        { id: 'm-flash', tags: [] },
      ],
    })
    el.remove()
  })

  // 4.2 验收：同 id 保留人工 capability tags、按 id 去重（上游可能回重复）。
  it('keeps manual capability tags for surviving ids and dedupes by id', async () => {
    apiMocks.fetchProviderModels.mockResolvedValue({
      provider: 'alpha',
      models: ['m2', 'm3', 'm2'],
    })
    const el = await mount()
    const dialog = await openEditorFor(el, 'alpha')
    // alpha 存量目录：m1、m2(vision)。抓取后骨架 = m2、m3（重复 m2 折叠），
    // m2 的人工 vision 保留。
    expect(entryIds(dialog)).toEqual(['m1', 'm2'])

    editorFetchButton(dialog)!.click()
    await settle(el)

    expect(entryIds(dialog)).toEqual(['m2', 'm3'])
    const vision = entryRows(dialog)[0]!.querySelector(
      'input[data-testid="tag-vision"]',
    ) as HTMLInputElement
    expect(vision.checked).toBe(true)

    await saveEditor(el)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledWith('alpha', {
      protocol: 'auto',
      preset: 'deepseek',
      models: [
        { id: 'm2', tags: ['vision'] },
        { id: 'm3', tags: [] },
      ],
    })
    el.remove()
  })

  // 4.3 验收：取消编辑器 = 丢弃抓取结果，存储不变。
  it('cancelling the editor discards the fetch', async () => {
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    editorFetchButton(dialog)!.click()
    await settle(el)
    expect(entryIds(dialog)).toEqual(['m-pro', 'm-flash'])

    ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).toBeNull()
    expect(apiMocks.routerProviderUpdate).not.toHaveBeenCalled()

    // 重开编辑器：草稿回到存量目录（空），不是抓取结果。
    const dialog2 = await openEditorFor(el, 'beta')
    expect(entryRows(dialog2).length).toBe(0)
    el.remove()
  })

  // 3.3 验收（继承）：失败如实呈现净化原因、绝不冒充空成功列表；草稿不动。
  it('keeps the draft and reports the sanitized reason on failure', async () => {
    apiMocks.fetchProviderModels.mockRejectedValue(
      new ApiError(502, "fetch_models: provider 'beta' 上游抓取失败: HTTP 401 Unauthorized"),
    )
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    // 先手工加一条，证明失败后草稿原样保留。
    ;(dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement).click()
    await el.updateComplete
    const manual = entryRows(dialog)[0]!.querySelector('wa-input') as unknown as {
      value: string
      dispatchEvent: (e: Event) => boolean
    }
    manual.value = 'manual-1'
    manual.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    editorFetchButton(dialog)!.click()
    await settle(el)

    const err = dialog.querySelector('.fetch-error[role="alert"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('HTTP 401')
    expect(entryIds(dialog)).toEqual(['manual-1'])
    expect(apiMocks.routerProviderUpdate).not.toHaveBeenCalled()
    el.remove()
  })

  // spec「no base URL means no fetch entry」：三槽位全空的 provider，编辑器
  // 不渲染 fetch 动作。
  it('renders no fetch action in the editor for a provider without a usable base URL', async () => {
    apiMocks.routerProviders.mockResolvedValue({
      providers: [
        {
          name: 'urlless',
          preset: null,
          base_url_anthropic: null,
          base_url_openai_chat: null,
          base_url_openai_responses: null,
          api_key_env: null,
          api_key_configured: false,
          models: [],
        },
      ],
    })
    const el = await mount()
    const dialog = await openEditorFor(el, 'urlless')
    expect(editorFetchButton(dialog)).toBeNull()
    el.remove()
  })

  // D4 边界：新建模式没有已存储的 provider 可探测（probe op 按 name 寻址），
  // 即便 presetDef 有 code-table URL 也不渲染 fetch 动作。
  it('renders no fetch action in create editors', async () => {
    const el = await mount()
    for (const label of ['New (preset)', 'New (custom)']) {
      const dialog = await openCreateEditor(el, label)
      expect(editorFetchButton(dialog)).toBeNull()
      ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
      await el.updateComplete
    }
    el.remove()
  })

  // 4.3 验收：fetch 在飞时按钮 disabled。
  it('disables the fetch button while the request is in flight', async () => {
    let release!: (v: { provider: string; models: string[] }) => void
    apiMocks.fetchProviderModels.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    const btn = editorFetchButton(dialog)!
    btn.click()
    await el.updateComplete
    expect(btn.disabled).toBe(true)
    release({ provider: 'beta', models: ['m-pro'] })
    await settle(el)
    expect(btn.disabled).toBe(false)
    expect(entryIds(dialog)).toEqual(['m-pro'])
    el.remove()
  })
})

describe('redesign-provider-models-settings 2.1：wa-hide 来源守卫', () => {
  async function openEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, 3)
    const newBtn = waButtons(el).find((b) => b.textContent?.includes('New (preset)'))!
    expect(newBtn).toBeTruthy()
    newBtn.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
    expect(dialog).toBeTruthy()
    return dialog
  }

  it('a wa-hide bubbling from the inner wa-select does NOT close the editor', async () => {
    const el = await mount()
    const dialog = await openEditor(el)
    const select = dialog.querySelector('wa-select[label="Preset"]')
    expect(select).toBeTruthy()
    // 模拟 WA <wa-select> 收起列表框：从子控件派发 composed wa-hide。
    select!.dispatchEvent(new CustomEvent('wa-hide', { bubbles: true, composed: true }))
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).not.toBeNull()
    expect(dialog.hasAttribute('open')).toBe(true)
    el.remove()
  })

  it('a wa-hide from the dialog itself still closes it', async () => {
    const el = await mount()
    const dialog = await openEditor(el)
    dialog.dispatchEvent(new CustomEvent('wa-hide', { bubbles: true, composed: true }))
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).toBeNull()
    el.remove()
  })
})

describe('redesign-provider-models-settings 3.1：预制最小表单', () => {
  /** 打开预制编辑器并等它渲染。 */
  async function openPresetEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, 3)
    waButtons(el)
      .find((b) => b.textContent?.includes('New (preset)'))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  /** 设 WA 表单控件的值并派发 input 事件（组件 @input 读 target.value）。 */
  function setWaValue(host: Element, selector: string, value: string): void {
    const input = host.querySelector(selector) as unknown as HTMLInputElement
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }

  it('preset create needs only preset + key + model entries; name defaults to the preset name', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    apiMocks.routerProviderCreate.mockResolvedValue({ created: 'deepseek' })

    // 最小路径里没有实例名输入（D5：默认取 preset 名；改名在 Advanced）。
    expect(dialog.querySelector('wa-input[label="Name"]')).toBeNull()

    // API key。
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-test')
    // 两个模型条目：第一个纯文本，第二个带 vision。
    const addBtn = dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement
    addBtn.click()
    addBtn.click()
    await el.updateComplete
    const rows = [...dialog.querySelectorAll('[data-testid="model-entry"]')]
    expect(rows.length).toBe(2)
    const idInputs = rows.map(
      (r) => r.querySelector('wa-input') as unknown as { value: string; dispatchEvent: (e: Event) => boolean },
    )
    idInputs[0].value = 'model-a'
    idInputs[0].dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    idInputs[1].value = 'model-b'
    idInputs[1].dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    const visionBox = rows[1].querySelector(
      'input[data-testid="tag-vision"]',
    ) as HTMLInputElement
    visionBox.click()
    await el.updateComplete

    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)

    expect(apiMocks.routerProviderCreate).toHaveBeenCalledTimes(1)
    const payload = apiMocks.routerProviderCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.preset).toBe('deepseek')
    expect(payload.api_key).toBe('sk-test')
    // 实例名缺省取预设名（D5）。
    expect(payload.name).toBe('deepseek')
    // 条目与能力标记原样提交（text 隐含不写）。
    expect(payload.models).toEqual([
      { id: 'model-a', tags: [] },
      { id: 'model-b', tags: ['vision'] },
    ])
    // 3.1 验收：不含 base_url_* 与 api_key_env。
    expect(payload.base_url_anthropic).toBeUndefined()
    expect(payload.base_url_openai_chat).toBeUndefined()
    expect(payload.base_url_openai_responses).toBeUndefined()
    expect(payload.api_key_env).toBeUndefined()
    el.remove()
  })

  it('untouched preset entries stay unsubmitted (catalog keeps following the code table)', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    apiMocks.routerProviderCreate.mockResolvedValue({ created: 'deepseek' })
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-only')
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
    const payload = apiMocks.routerProviderCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.models).toBeUndefined()
    el.remove()
  })

  it('the entries block reads "Models" with a bare full-width ＋ and no hint copy', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    // revamp…3.1：区块标签改「Models」、两句提示语删除。
    expect(dialog.querySelector('.model-entries .entries-label')?.textContent?.trim()).toBe('Models')
    expect(dialog.querySelector('.entries-hint')).toBeNull()
    // 「＋ Add model」改纯 ＋ 通栏按钮。
    const add = dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement
    expect(add).toBeTruthy()
    expect(add.textContent?.trim()).toBe('＋')
    expect(add.classList.contains('add-model')).toBe(true)
    el.remove()
  })
})

describe('redesign-provider-models-settings 3.2：定制最小表单 + Advanced 折叠', () => {
  async function openCustomEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, 3)
    waButtons(el)
      .find((b) => b.textContent?.includes('New (custom)'))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  function setWaValue(host: Element, selector: string, value: string): void {
    const input = host.querySelector(selector) as unknown as HTMLInputElement
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }

  async function save(el: SebasSettingsModal): Promise<void> {
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
  }

  it('minimal custom create: name + base url + protocol; advanced collapsed by default', async () => {
    const el = await mount()
    const dialog = await openCustomEditor(el)
    apiMocks.routerProviderCreate.mockResolvedValue({ created: 'my-api' })

    const advanced = dialog.querySelector('details.advanced') as HTMLDetailsElement
    expect(advanced).toBeTruthy()
    // 默认折叠。
    expect(advanced.open).toBe(false)

    setWaValue(dialog, 'wa-input[label="Name"]', 'my-api')
    // 协议缺省 OpenAI-compatible → 单个 Base URL 落 openai_chat 槽（D4）。
    setWaValue(dialog, 'wa-input[label="Base URL (OpenAI-compatible)"]', 'https://api.example/v1')
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-custom')

    await save(el)
    expect(apiMocks.routerProviderCreate).toHaveBeenCalledTimes(1)
    const payload = apiMocks.routerProviderCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.name).toBe('my-api')
    expect(payload.protocol).toBe('openai')
    expect(payload.base_url_openai_chat).toBe('https://api.example/v1')
    // 默认提交不带 Advanced 独占字段：其余槽位 / 改名映射 / api_key_env。
    expect(payload.base_url_anthropic).toBeUndefined()
    expect(payload.base_url_openai_responses).toBeUndefined()
    expect(payload.model_map).toBeUndefined()
    expect(payload.api_key_env).toBeUndefined()
    el.remove()
  })

  it('expanding Advanced exposes the remaining slots and rename map for editing', async () => {
    const el = await mount()
    const dialog = await openCustomEditor(el)
    apiMocks.routerProviderCreate.mockResolvedValue({ created: 'my-api' })

    const advanced = dialog.querySelector('details.advanced') as HTMLDetailsElement
    advanced.open = true
    await el.updateComplete

    setWaValue(dialog, 'wa-input[label="Name"]', 'my-api')
    setWaValue(dialog, 'wa-input[label="Base URL (OpenAI-compatible)"]', 'https://api.example/v1')
    // 展开后可编辑其余槽位（anthropic 槽 + responses 槽）与改名映射。
    setWaValue(dialog, 'wa-input[label="Base URL (Anthropic)"]', 'https://api.example/anthropic')
    setWaValue(
      dialog,
      'wa-input[label="Base URL (OpenAI Responses)"]',
      'https://api.example/responses',
    )
    setWaValue(
      dialog,
      'wa-input[label^="Model rename map"]',
      'old-model -> new-model',
    )
    await save(el)

    const payload = apiMocks.routerProviderCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.base_url_anthropic).toBe('https://api.example/anthropic')
    expect(payload.base_url_openai_responses).toBe('https://api.example/responses')
    expect(payload.model_map).toEqual({ 'old-model': 'new-model' })
    el.remove()
  })
})
