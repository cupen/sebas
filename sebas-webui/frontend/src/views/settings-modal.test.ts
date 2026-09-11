// @vitest-environment jsdom
/**
 * Settings modal (IA v3, fix-settings-menu-and-services-semantics +
 * redesign-provider-models-settings)：左侧分区导航 + 右侧内容。六个分区
 * （顺序即规约）——
 *   - settings   总览壳：工作区根目录（/api/fs/browse-dirs 回显根）/
 *                default agent kind（静态 acp）/
 *                default provider-model（跳转 Models 链接）+「全部进程重启」
 *                「重置 Settings」高危动作（wa-dialog 二次确认；无 watchdog
 *                控制面时重启 disabled）
 *   - services   watchdog 受管子进程（/api/admin/services 经 adminServicesSafe：
 *                name / desired / actual / uptime + /api/admin/events 最近错误；
 *                im→「飞书 IM」显示映射；无 adapter 时「无 watchdog 控制面」
 *                横幅而非空列表冒充——router 行的 desired/actual/uptime 只在
 *                此分区呈现）
 *   - models     provider 管理列表（/router/api/providers，条目携带能力
 *                标记）。redesign-provider-models-settings 3.4：不再渲染
 *                Router 网关卡、不再请求 /api/router
 *   - appearance 主题三态（system/dark/light，走真实 theme.ts）
 *   - env        环境变量名清单，值一律 "managed by core config"
 *   - about      /api/about 真实字段
 * 缺省首项 settings；上次分区记忆走 localStorage `lastSettingsSection`
 * （非法值回退 settings）。关闭交互（按钮 / Esc / 遮罩）一并覆盖；子
 * `<wa-select>` 冒泡的 `wa-hide` 不得连带关闭对话框（2.1 来源守卫）。
 * api client 全量 mock（9 个 admin stubs 由前序 agent 加入，本轮沿用）。
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
  agentDefaults: vi.fn(),
  setAgentDefaults: vi.fn(),
  adminServices: vi.fn(),
  adminEvents: vi.fn(),
  adminServicesSafe: vi.fn(),
  adminEventsSafe: vi.fn(),
  enableService: vi.fn(),
  disableService: vi.fn(),
  restartService: vi.fn(),
  adminRestart: vi.fn(),
  fsBrowseDirs: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  ApiError: class ApiError extends Error {
    readonly status: number
    constructor(status: number, message: string) {
      super(message)
      this.status = status
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
    agentDefaults: apiMocks.agentDefaults,
    setAgentDefaults: apiMocks.setAgentDefaults,
    adminServices: apiMocks.adminServices,
    adminEvents: apiMocks.adminEvents,
    adminServicesSafe: apiMocks.adminServicesSafe,
    adminEventsSafe: apiMocks.adminEventsSafe,
    enableService: apiMocks.enableService,
    disableService: apiMocks.disableService,
    restartService: apiMocks.restartService,
    adminRestart: apiMocks.adminRestart,
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
      providers: [
        {
          name: 'alpha',
          preset: 'deepseek',
          base_url_anthropic: 'https://a.example/anthropic',
          base_url_openai_chat: 'https://a.example/v1',
          base_url_openai_responses: null,
        },
        {
          name: 'beta',
          base_url_anthropic: null,
          base_url_openai_chat: 'https://b.example/v1',
          base_url_openai_responses: null,
        },
      ],
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
  apiMocks.agentDefaults.mockResolvedValue({ provider: null, model: null })
  apiMocks.adminServices.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEvents.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.enableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.disableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.restartService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.adminRestart.mockResolvedValue({ operation_id: 'op-test', message: 'restart accepted' })
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
  it('renders the left nav with exactly Settings/Services/Models/Appearance/Environment/About', async () => {
    const el = await mount()
    const labels = navItems(el).map((b) => b.textContent?.trim())
    expect(labels).toEqual(['Settings', 'Services', 'Models', 'Appearance', 'Environment', 'About'])
    el.remove()
  })

  it('defaults to the Settings overview shell with workspace/defaults/danger actions', async () => {
    const el = await mount()
    expect(el.section).toBe('settings')
    await settle(el)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('Workspace root')
    expect(text).toContain('Default agent kind')
    expect(text).toContain('Default provider / model')
    const buttons = waButtons(el).map((b) => b.textContent?.trim())
    expect(buttons).toContain('全部进程重启')
    expect(buttons).toContain('重置 Settings')
    el.remove()
  })

  it('Settings overview renders workspace root, acp kind and the provider/model link', async () => {
    const el = await mount()
    // 总览懒加载挂在切入 settings 时：先离开再回来触发 loadOverview。
    await goto(el, 1)
    await goto(el, 0)
    const text = el.shadowRoot!.textContent ?? ''
    expect(apiMocks.fsBrowseDirs).toHaveBeenCalled()
    expect(text).toContain('/tmp/test-work')
    expect(text).toContain('acp')
    // 工作区根目录带复制按钮。
    const copy = el.shadowRoot!.querySelector('button[title="Copy workspace root"]')
    expect(copy).toBeTruthy()
    el.remove()
  })

  it('Settings danger actions confirm via wa-dialog; restart disabled without adapter', async () => {
    const el = await mount()
    await goto(el, 1)
    await goto(el, 0)
    const restart = waButtons(el).find((b) => b.textContent?.includes('全部进程重启'))!
    expect(restart).toBeTruthy()
    expect(restart.hasAttribute('disabled')).toBe(false)
    restart.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="全部进程重启"]') as HTMLElement & {
      open?: boolean
    }
    expect(dialog).toBeTruthy()
    expect(dialog.hasAttribute('open')).toBe(true)
    el.remove()
  })

  it('Settings restart button is disabled with tooltip when no watchdog adapter', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, 1)
    await goto(el, 0)
    const restart = waButtons(el).find((b) => b.textContent?.includes('全部进程重启'))!
    expect(restart.hasAttribute('disabled')).toBe(true)
    expect(restart.getAttribute('title')).toContain('无 watchdog 控制面')
    // 重置 Settings 不依赖 adapter，始终可用。
    const reset = waButtons(el).find((b) => b.textContent?.includes('重置 Settings'))!
    expect(reset.hasAttribute('disabled')).toBe(false)
    el.remove()
  })

  it('remembers lastSettingsSection across opens; illegal values fall back to settings', async () => {
    localStorage.setItem('lastSettingsSection', 'services')
    const el = await mount()
    await settle(el)
    expect(el.section).toBe('services')
    // 打开期间切换分区会写回记忆。
    await goto(el, 2)
    expect(localStorage.getItem('lastSettingsSection')).toBe('models')
    el.remove()

    localStorage.setItem('lastSettingsSection', 'nope')
    const el2 = await mount()
    await settle(el2)
    expect(el2.section).toBe('settings')
    el2.remove()
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
    await goto(el, 1)
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
    await goto(el, 1)
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
    await goto(el, 1)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('无 watchdog 控制面')
    expect(el.shadowRoot!.querySelectorAll('.service-card').length).toBe(0)
    expect(el.shadowRoot!.querySelectorAll('.service-actions button').length).toBe(0)
    el.remove()
  })

  it('Models section renders the provider list only — no gateway card, no /api/router', async () => {
    const el = await mount()
    apiMocks.router.mockClear()
    await goto(el, 2)
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
    await goto(el, 1)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('router')
    expect(text).toContain('desired running · status running · up 1m')
    el.remove()
  })

  it('Environment section lists variable names with the honest placeholder value', async () => {
    const el = await mount()
    await goto(el, 4)
    expect(el.section).toBe('env')
    const rows = [...el.shadowRoot!.querySelectorAll('.env-table tbody tr')]
    expect(rows.length).toBeGreaterThan(0)
    expect(el.shadowRoot!.textContent).toContain('SEBAS_ROUTER_LISTEN')
    // Every row's value is the "not exposed" marker — no fabricated data.
    for (const row of rows) {
      expect(row.querySelector('.value')?.textContent).toBe('managed by core config')
    }
    el.remove()
  })

  it('About section renders the real /api/about payload', async () => {
    const el = await mount()
    await goto(el, 5)
    expect(el.section).toBe('about')
    expect(apiMocks.about).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('0.4.2')
    expect(text).toContain('3h 12m')
    expect(text).toContain('1.88')
    expect(text).toContain('127.0.0.1:8787')
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
    await goto(el, 3)
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
    await goto(el, 3)
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
    await goto(el, 3)
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
  // workbench-agent-wire-fix 3.3：/api/agent-defaults 端点退役——总览与
  // provider 列表不再渲染全局 default 徽章（默认 agent 改为项目级记忆）。

  const el = await mount()
  await goto(el, 2)

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('no default set')
  expect(el.shadowRoot?.querySelector('.provider-badge.default')).toBeNull()
  el.remove()
})

describe('add-fetch-models：provider 抓取入口与挑选语义', () => {
  /** 找指定 provider 行上的抓取按钮（🔍，data-testid=fetch-models）。 */
  function fetchButtonFor(el: SebasSettingsModal, name: string): HTMLButtonElement | null {
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    const row = rows.find(
      (r) => r.querySelector('.provider-row-name')?.textContent?.trim() === name,
    )
    return row?.querySelector<HTMLButtonElement>('button[data-testid="fetch-models"]') ?? null
  }

  /** 抓取结果卡里的挑选按钮（＋，data-testid=pick-fetched-model）。 */
  function pickButtons(el: SebasSettingsModal): HTMLButtonElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLButtonElement>('button[data-testid="pick-fetched-model"]')]
  }

  beforeEach(() => {
    apiMocks.fetchProviderModels.mockResolvedValue({ provider: 'beta', models: ['m-pro', 'm-flash'] })
  })

  // 3.1 验收「两种模式都能触发」：custom（有 URL）与 preset 派生（行内
  // 物化代码表 URL）都渲染抓取按钮且能发起抓取。
  it('renders the fetch action for custom and preset-derived providers and both trigger', async () => {
    const el = await mount()
    await goto(el, 2)
    expect(apiMocks.fetchProviderModels).not.toHaveBeenCalled()

    const betaFetch = fetchButtonFor(el, 'beta')
    expect(betaFetch).toBeTruthy()
    betaFetch!.click()
    await settle(el)
    expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith('beta')
    // 抓取结果列表呈现（ok 态）。
    expect(pickButtons(el).length).toBe(2)
    el.remove()
  })

  // 3.1 验收「无可用 base url 不渲染入口」：三槽位全空 → 无 🔍 按钮。
  it('hides the fetch action for a provider with no usable base URL', async () => {
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
    await goto(el, 2)
    expect(fetchButtonFor(el, 'urlless')).toBeNull()
    el.remove()
  })

  // 3.2 验收「抓取本身不提交任何写请求，挑选后才提交」：抓取只调
  // fetchProviderModels；挑选某个 id 才发一次 routerProviderUpdate（普通
  // 编辑 PUT），payload 是完整的 custom 字段集 + models 追加该 id。
  it('fetch submits no write; picking a fetched id issues one ordinary edit', async () => {
    const el = await mount()
    await goto(el, 2)
    apiMocks.routerProviderUpdate.mockClear()
    apiMocks.routerProviderUpdate.mockResolvedValue({ updated: 'beta' })

    fetchButtonFor(el, 'beta')!.click()
    await settle(el)
    // 抓取本身零写请求。
    expect(apiMocks.routerProviderUpdate).not.toHaveBeenCalled()
    expect(apiMocks.routerProviderCreate).not.toHaveBeenCalled()

    // 挑选 m-pro → 普通编辑 PUT：models 追加该 id（条目形态，text 隐含），
    // 其余字段无损回填。
    pickButtons(el)[0]!.click()
    await settle(el)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledTimes(1)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledWith('beta', {
      name: 'beta',
      base_url_anthropic: undefined,
      base_url_openai_chat: 'https://b.example/v1',
      base_url_openai_responses: undefined,
      api_key_env: undefined,
      default_model: undefined,
      protocol: undefined,
      model_map: undefined,
      models: [{ id: 'm-pro', tags: [] }],
    })
    el.remove()
  })

  // 3.2 验收（preset 语义）：preset 派生 provider 的目录跟随代码表，挑选
  // 走 default_model 编辑（与 /provider 卡片「使用 <model>」同一契约）。
  it('picking on a preset-derived provider edits default_model, not the code-table catalog', async () => {
    const el = await mount()
    await goto(el, 2)
    apiMocks.routerProviderUpdate.mockClear()
    apiMocks.routerProviderUpdate.mockResolvedValue({ updated: 'alpha' })

    fetchButtonFor(el, 'alpha')!.click()
    await settle(el)
    pickButtons(el)[0]!.click()
    await settle(el)
    expect(apiMocks.routerProviderUpdate).toHaveBeenCalledWith('alpha', {
      preset: 'deepseek',
      default_model: 'm-pro',
    })
    el.remove()
  })

  // 3.3 验收：失败如实呈现净化原因，绝不渲染空结果列表冒充「没有模型」。
  it('renders the sanitized failure instead of an empty list', async () => {
    apiMocks.fetchProviderModels.mockRejectedValue(
      new ApiError(502, "fetch_models: provider 'beta' 上游抓取失败: HTTP 401 Unauthorized"),
    )
    const el = await mount()
    await goto(el, 2)
    fetchButtonFor(el, 'beta')!.click()
    await settle(el)

    const callout = el.shadowRoot!.querySelector('.callout-error')
    expect(callout).toBeTruthy()
    expect(callout!.textContent).toContain('HTTP 401')
    // 失败 ≠ 空列表：不渲染结果列表、不渲染挑选按钮。
    expect(el.shadowRoot!.querySelector('.fetch-result-list')).toBeNull()
    expect(pickButtons(el).length).toBe(0)
    el.remove()
  })

  // 3.3 验收（redesign-provider-models-settings）：表单重做后抓取入口仍
  // 可达——预制派生与定制的行内 🔍 都能触发，机制与语义归 add-fetch-models，
  // 本 change 未新增抓取语义（上方两条用例原样保留即为此证）。
  it('3.3 keeps the fetch entry reachable for preset and custom providers after the form rework', async () => {
    const el = await mount()
    await goto(el, 2)
    expect(fetchButtonFor(el, 'alpha')).toBeTruthy()
    expect(fetchButtonFor(el, 'beta')).toBeTruthy()
    fetchButtonFor(el, 'alpha')!.click()
    await settle(el)
    expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith('alpha')
    expect(pickButtons(el).length).toBe(2)
    el.remove()
  })
})

describe('redesign-provider-models-settings 2.1：wa-hide 来源守卫', () => {
  async function openEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, 2)
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
    await goto(el, 2)
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
})

describe('redesign-provider-models-settings 3.2：定制最小表单 + Advanced 折叠', () => {
  async function openCustomEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, 2)
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
