// @vitest-environment jsdom
/**
 * Settings modal (IA v3, fix-settings-menu-and-services-semantics)：左侧分区
 * 导航 + 右侧内容。六个分区（顺序即规约）——
 *   - settings   总览壳：工作区根目录（/api/fs/browse-dirs 回显根）/
 *                default agent kind（静态 acp）/
 *                default provider-model（/api/agent-defaults + 跳转 Models
 *                链接）+「全部进程重启」「重置 Settings」高危动作（wa-dialog
 *                二次确认；无 watchdog 控制面时重启 disabled）
 *   - services   watchdog 受管子进程（/api/admin/services 经 adminServicesSafe：
 *                name / desired / actual / uptime + /api/admin/events 最近错误；
 *                im→「飞书 IM」显示映射；无 adapter 时「无 watchdog 控制面」横幅）
 *   - models     「Router 路由网关」总览卡（/api/router 的 listen / debug /
 *                auth）+ provider 管理列表（/router/api/providers）
 *   - appearance 主题三态（system/dark/light，走真实 theme.ts）
 *   - env        环境变量名清单，值一律 "managed by core config"
 *   - about      /api/about 真实字段
 * 缺省首项 settings；上次分区记忆走 localStorage `lastSettingsSection`
 * （非法值回退 settings）。关闭交互（按钮 / Esc / 遮罩）一并覆盖。
 * api client 全量 mock（9 个 admin stubs 由前序 agent 加入，本轮沿用）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
  routerProviderProbe: vi.fn(),
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
    routerProviderProbe: apiMocks.routerProviderProbe,
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
        models: ['m1'],
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
        models: ['deepseek-chat'],
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
    apiMocks.agentDefaults.mockResolvedValue({ provider: 'alpha', model: 'm1' })
    const el = await mount()
    // 总览懒加载挂在切入 settings 时：先离开再回来触发 loadOverview。
    await goto(el, 1)
    await goto(el, 0)
    const text = el.shadowRoot!.textContent ?? ''
    expect(apiMocks.fsBrowseDirs).toHaveBeenCalled()
    expect(apiMocks.agentDefaults).toHaveBeenCalled()
    expect(text).toContain('/tmp/test-work')
    expect(text).toContain('acp')
    expect(text).toContain('alpha / m1')
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

  it('Models section renders the Router gateway card plus the provider list', async () => {
    const el = await mount()
    await goto(el, 2)
    expect(el.section).toBe('models')
    expect(apiMocks.router).toHaveBeenCalled()
    expect(apiMocks.routerProviders).toHaveBeenCalled()
    expect(apiMocks.routerPresets).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('Router 路由网关')
    expect(text).toContain('127.0.0.1:8787')
    expect(text).toContain('configured')
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    expect(rows.length).toBe(2)
    expect(text).toContain('alpha')
    expect(text).toContain('deepseek · code')
    expect(text).toContain('custom')
    expect(text).toContain('https://a.example/anthropic')
    expect(text).toContain('beta')
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


it('shows the current agent default and badges the matching provider row', async () => {
  apiMocks.agentDefaults.mockResolvedValue({ provider: 'alpha', model: 'm1' })
  const el = await mount()
  await goto(el, 2)

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('default: alpha / m1')
  const badge = el.shadowRoot?.querySelector('.provider-badge.default')
  expect(badge).toBeTruthy()
  const row = badge?.closest('.provider-row')
  expect(row?.textContent ?? '').toContain('alpha')
  el.remove()
})
