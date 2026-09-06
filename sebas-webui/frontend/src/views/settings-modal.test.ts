// @vitest-environment jsdom
/**
 * Settings modal (IA v2)：左侧分区导航 + 右侧内容。五个分区——
 *   - models   provider 列表（名称 + base URL，来自 /api/router）
 *   - services Router 后台服务状态（listen / debug / auth）
 *   - appearance 主题三态（system/dark/light，走真实 theme.ts）
 *   - env      环境变量名清单，值一律 "managed by core config"
 *   - about    /api/about 真实字段
 * 关闭交互（按钮 / Esc / 遮罩）一并覆盖。api client 全量 mock。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// jsdom 这里不提供 localStorage（about:blank origin），沿用仓库的内存
// polyfill 约定（见 transcript-view.test.ts）；Appearance 分区的主题
// 持久化走真实 theme.ts，所以需要它真实可读写。
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

beforeEach(() => {
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
  document.documentElement.classList.remove('wa-dark')
})

describe('sebas-settings-modal sections', () => {
  it('renders the left nav with exactly Models/Services/Appearance/Environment/About', async () => {
    const el = await mount()
    const labels = navItems(el).map((b) => b.textContent?.trim())
    expect(labels).toEqual(['Models', 'Services', 'Appearance', 'Environment', 'About'])
    el.remove()
  })

  it('defaults to the Models section rendering providers from the admin list', async () => {
    const el = await mount()
    expect(el.section).toBe('models')
    await settle(el)
    expect(apiMocks.routerProviders).toHaveBeenCalled()
    expect(apiMocks.routerPresets).toHaveBeenCalled()
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    expect(rows.length).toBe(2)
    expect(el.shadowRoot!.textContent).toContain('alpha')
    expect(el.shadowRoot!.textContent).toContain('deepseek · code')
    expect(el.shadowRoot!.textContent).toContain('custom')
    expect(el.shadowRoot!.textContent).toContain('https://a.example/anthropic')
    expect(el.shadowRoot!.textContent).toContain('beta')
    el.remove()
  })

  it('Services section renders the Router service card', async () => {
    const el = await mount()
    navItems(el)[1]!.click()
    await el.updateComplete
    await settle(el)
    expect(el.section).toBe('services')
    expect(apiMocks.router).toHaveBeenCalled()
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    expect(cards.length).toBe(2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('Router')
    expect(text).toContain('127.0.0.1:8787')
    expect(text).toContain('2 provider(s)')
    expect(text).toContain('auth configured')
    el.remove()
  })

  it('Environment section lists variable names with the honest placeholder value', async () => {
    const el = await mount()
    navItems(el)[3]!.click()
    await el.updateComplete
    await settle(el)
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
    navItems(el)[4]!.click()
    await el.updateComplete
    await settle(el)
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
    navItems(el)[2]!.click()
    await el.updateComplete
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
    navItems(el)[2]!.click()
    await el.updateComplete
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
    navItems(el)[2]!.click()
    await el.updateComplete
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
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('default: alpha / m1')
  const badge = el.shadowRoot?.querySelector('.provider-badge.default')
  expect(badge).toBeTruthy()
  const row = badge?.closest('.provider-row')
  expect(row?.textContent ?? '').toContain('alpha')
})
