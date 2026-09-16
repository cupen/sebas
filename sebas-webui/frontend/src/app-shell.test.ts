// @vitest-environment jsdom
/**
 * IA v2 shell audit：侧栏 = 品牌 + 项目树（sebas-project-rail）+ 底部
 * pinned 的 Settings 入口（打开居中设置弹窗）；旧的 NAV_ITEMS 链接列表
 * （Dashboard/Settings/Router/About/Admin）整体删除。路由仅保留
 * `/`、`/sessions`、`/sessions/:key`；退役路径 /settings /router /about
 * 经 router.redirectFor canonical 回 `/`；/admin/* 直接删除——当作未知
 * 路径落到 workbench fallback。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ---- WA 渲染垫片（共享）--------------------------------------------------
// outlet 会实例化 dashboard / project-rail 等视图，它们渲染 WA 表单关联组件
// （wa-button 等）；jsdom 缺 setValidity / showModal / getAnimations 会抛未处理
// rejection。共享实现见 test-support/wa-polyfills.ts。
import { installWaDomPolyfills } from './test-support/wa-polyfills.js'

installWaDomPolyfills()

// ---- hoisted mocks（须先于被测模块的静态导入生效）----------------------

const apiMocks = vi.hoisted(() => ({
  summary: vi.fn(),
  sessions: vi.fn(),
  settings: vi.fn(),
  authMe: vi.fn(),
  projectsList: vi.fn(),
  projectsBranch: vi.fn(),
  projectsAdd: vi.fn(),
  projectsReorder: vi.fn(),
  setUnauthorizedHandler: vi.fn(),
}))

/**
 * 共享 WS 客户端 mock（add-core-reachability-ws-push 2.1）：subscribe 捕获
 * handler 供用例派发 `core.reachability` 翻转通知（emit），request 可编程
 * 供用例编排 `core.reachability.get` 的应答。路径必须与被测模块同解——
 * 本文件在 src/，故用 './api/shared-ws.js'（'../api/…' 会指到不存在的
 * frontend/api/，mock 静默失效）。
 */
const wsMocks = vi.hoisted(() => {
  const handlers = new Set<(ev: unknown) => void>()
  return {
    request: vi.fn(),
    subscribe: vi.fn((h: (ev: unknown) => void) => {
      handlers.add(h)
      return () => handlers.delete(h)
    }),
    emit: (ev: unknown): void => {
      for (const h of handlers) h(ev)
    },
    clearHandlers: (): void => handlers.clear(),
  }
})

// 注意路径：本文件位于 src/，client 模块是 './api/client.js'（'../api/…'
// 会解析到不存在的 frontend/api/，mock 静默失效）。真实模块的其余导出
// （setUnauthorizedHandler 等）保持原样，只接管 api 的方法。
vi.mock('./api/client.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api/client.js')>()
  return {
    ...actual,
    setUnauthorizedHandler: apiMocks.setUnauthorizedHandler,
    api: {
      ...actual.api,
      summary: apiMocks.summary,
      sessions: apiMocks.sessions,
      settings: apiMocks.settings,
      authMe: apiMocks.authMe,
      projects: {
        ...(actual.api as { projects?: Record<string, unknown> }).projects,
        list: apiMocks.projectsList,
        branch: apiMocks.projectsBranch,
        add: apiMocks.projectsAdd,
        reorder: apiMocks.projectsReorder,
      },
    },
  }
})

vi.mock('./api/shared-ws.js', () => ({ sharedWs: wsMocks }))

// outlet 会实例化 sebas-dashboard；composer 依赖 WA 表单组件的
// ElementInternals（jsdom 不完整），与 shell 无关 —— mock 掉模块即可，
// <sebas-workbench-composer> 作为未知元素惰性渲染。dashboard 还从该模块
// 取一次性对焦请求的事件名（workbench-rail-polish 3.2），mock 里补上。
vi.mock('./views/workbench-composer.js', () => ({
  COMPOSER_FOCUS_REQUEST: 'sebas:composer-focus',
}))

// ---- 被测模块（mock 生效后导入）----------------------------------------

import { matchRoute, redirectFor } from './router.js'
import { ROUTES, SebasApp } from './app-shell.js'
import { resetNotices } from './notify.js'
import { APP_TAGLINE } from './branding.js'
import './views/dashboard.js'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
// fileURLToPath: Windows 下 URL().pathname 会得到 "/D:/..."，join 后成 "D:\D:\..."
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

const projectFixture = { path: '/home/me/sebas', name: 'sebas', added_at: 1, branch: 'main' }

beforeEach(() => {
  // 通知 store 是模块级单例：跨用例隔离（去重窗口 8s > 用例间隔）。
  resetNotices()
  apiMocks.summary.mockResolvedValue({
    active_count: 0,
    dormant_count: 0,
    spawning_count: 0,
    total_sessions: 0,
    uptime: '0s',
    recent_sessions: [],
    active_session: null,
    active_session_key: null,
    reachability: { ok: true },
  })
  apiMocks.sessions.mockResolvedValue({ recent_sessions: [], active_count: 0 })
  apiMocks.settings.mockResolvedValue({
    card_config: {
      theme_color: '#000',
      fold_long_output: false,
      thinking_display: 'auto',
      max_user_text_chars: 0,
      max_tool_output_chars: 0,
    },
    router: { listen: null, provider_count: 0, debug: false, has_auth: false, providers: [] },
  })
  apiMocks.authMe.mockResolvedValue({ enabled: false, authenticated: false })
  apiMocks.projectsList.mockResolvedValue({ projects: [projectFixture] })
  apiMocks.projectsBranch.mockRejectedValue(new Error('not fetched in this test'))
})

afterEach(() => {
  document.body.innerHTML = ''
  window.history.replaceState({}, '', '/')
})

async function mountShell(): Promise<SebasApp> {
  const el = document.createElement('sebas-app') as SebasApp
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

describe('sidebar IA v2', () => {
  it('renders the shared brand tagline in the rail brand (workbench-rail-polish 1.1)', async () => {
    const el = await mountShell()
    // 三处副标题（rail 顶 / 登录页 / 首启页）同一常量：rail 顶先验。
    expect(el.shadowRoot!.querySelector('.brand .name small')?.textContent).toBe(APP_TAGLINE)
    // 旧「agent router」副标题不再出现。
    expect(el.shadowRoot!.textContent).not.toContain('agent router')
    el.remove()
  })

  it('mounts the project tree + bottom settings entry, and ships no legacy nav links', async () => {
    const el = await mountShell()
    const nav = el.shadowRoot!.querySelector('nav')
    expect(nav).toBeTruthy()
    // 项目树挂在侧栏里，Settings 入口钉在底部。
    expect(el.shadowRoot!.querySelector('sebas-project-rail')).toBeTruthy()
    const settingsBtn = el.shadowRoot!.querySelector<HTMLButtonElement>('button.settings-btn')
    expect(settingsBtn?.textContent ?? '').toContain('Settings')
    // NAV_ITEMS 链接列表整体删除：无导航链接，退役路径一个都不出现。
    const hrefs = [...el.shadowRoot!.querySelectorAll('nav a')].map((a) => a.getAttribute('href'))
    expect(hrefs).toEqual(['/']) // 仅剩品牌回链
    expect(hrefs).not.toContain('/settings')
    expect(hrefs).not.toContain('/gateway')
    expect(hrefs).not.toContain('/about')
    expect(hrefs).not.toContain('/admin/status')
    expect(el.shadowRoot!.querySelectorAll('a.item')).toHaveLength(0)
    el.remove()
  })

  it('renders the full-bleed workbench frame on /: shell flex + outlet flex', async () => {
    const el = await mountShell()
    const root = el.shadowRoot!
    // jsdom 不解析 shadow 计算样式（:host 显示 inline、宽度 auto），改断言
    // 样式表规则与 DOM 契约（实机几何由浏览器验收覆盖）：框架 100vh 全屏
    // flex；侧栏|主区是 wa-split-panel 分割（180–520px clamp，5.2）+ 圆角浮岛
    // nav；main/outlet 满幅 flex 列。
    const styleText = [...root.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(styleText).toMatch(/:host\s*\{[^}]*display:\s*flex/)
    expect(styleText).toContain('height: 100vh')
    expect(styleText).toContain('overflow: hidden')
    // 侧栏|主区分割（5.1/D1）：wa-split-panel 承载，clamp 边界在样式表里。
    expect(root.querySelector('wa-split-panel.frame')).toBeTruthy()
    expect(styleText).toMatch(/wa-split-panel\.frame\s*\{[^}]*--min:\s*180px/)
    expect(styleText).toMatch(/wa-split-panel\.frame\s*\{[^}]*--max:\s*520px/)
    // nav 变浮岛：圆角 + surface 底，不再有通高 border-right 硬线。
    expect(styleText).toMatch(/nav\s*\{[^}]*border-radius:/)
    expect(styleText).toMatch(/nav\s*\{[^}]*min-height:\s*0/)
    expect(styleText).not.toMatch(/nav\s*\{[^}]*border-right:/)
    expect(styleText).toMatch(/main\s*\{[^}]*display:\s*flex/)
    // main 不带 padding（通栏面板）；只有文档型路由的 .outlet.padded 有。
    expect(styleText).not.toMatch(/main\s*\{[^}]*padding:/)
    const outlet = root.querySelector('.outlet')!
    expect(outlet.classList.contains('padded')).toBe(false)
    const outletRule = styleText.match(/\.outlet\s*\{[^}]*\}/)?.[0] ?? ''
    expect(outletRule).toContain('display: flex')
    expect(outletRule).not.toContain('max-width')
    el.remove()
  })

  it('remembers and restores the rail width from storage (5.1/D1)', async () => {
    // 组件初始化读 localStorage（此环境不可用 → 退默认 280px，5.2），拖拽回调
    // 持久化走 split-persist（其单测覆盖假 storage 路径）；这里断言绑定
    // 与监听在位。
    const el = await mountShell()
    const root = el.shadowRoot!
    const frame = root.querySelector('wa-split-panel.frame') as HTMLElement & {
      positionInPixels: number
    }
    expect(frame).toBeTruthy()
    expect(frame.getAttribute('position-in-pixels')).toBe('280')
    expect(frame.hasAttribute('disabled')).toBe(false)
    expect((el as unknown as { railWidth: number }).railWidth).toBe(280)

    // 拖拽回调：clamp 后写状态（此环境 storage 不可用，写入静默降级）。
    const target = el as unknown as { onRailReposition: (e: Event) => void; railWidth: number }
    target.onRailReposition({
      currentTarget: { positionInPixels: 9999 },
    } as unknown as Event)
    expect(target.railWidth).toBe(520)
    el.remove()
  })

  it('settings entry opens the centered modal stub; the close event shuts it', async () => {
    const el = await mountShell()
    const modal = el.shadowRoot!.querySelector('sebas-settings-modal')! as HTMLElement & {
      updateComplete: Promise<boolean>
    }
    // 关闭态不渲染任何内容。
    expect(modal.hasAttribute('open')).toBe(false)
    expect(modal.shadowRoot!.querySelector('.panel')).toBeNull()

    const btn = el.shadowRoot!.querySelector<HTMLButtonElement>('button.settings-btn')!
    btn.click()
    await el.updateComplete
    await modal.updateComplete
    expect(modal.hasAttribute('open')).toBe(true)
    // 居中弹窗骨架：dialog 语义 + 占位正文 + 可命名关闭按钮。
    const panel = modal.shadowRoot!.querySelector('[role="dialog"]')
    expect(panel?.getAttribute('aria-label')).toBe('Settings')
    expect(modal.shadowRoot!.querySelector('.close')).toBeTruthy()
    expect(modal.shadowRoot!.textContent).toContain('Settings')

    modal.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }))
    await el.updateComplete
    await modal.updateComplete
    expect(modal.hasAttribute('open')).toBe(false)
    el.remove()
  })

  it('the composer\'s open-settings event (bubbling from inside the workbench) opens the modal', async () => {
    const el = await mountShell()
    const modal = el.shadowRoot!.querySelector('sebas-settings-modal')! as HTMLElement & {
      updateComplete: Promise<boolean>
    }
    expect(modal.hasAttribute('open')).toBe(false)

    // The composer dispatches this composed event from its shadow root;
    // simulating it from the mounted dashboard (an ancestor-path element)
    // proves the listener catches events that cross shadow boundaries.
    const dashboard = el.shadowRoot!.querySelector('sebas-dashboard')!
    dashboard.dispatchEvent(new CustomEvent('open-settings', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(modal.hasAttribute('open')).toBe(true)
    el.remove()
  })
})

describe('routes after IA v2', () => {
  it('keeps the workbench, the sessions table and session deep links', () => {
    expect(matchRoute(ROUTES, '/')?.id).toBe('dashboard')
    expect(matchRoute(ROUTES, '/sessions')?.id).toBe('sessions')
    // key 保持 RAW（%00 NUL 回归）。深链渲染同一个工作台（3.4：无独立详情页）。
    const m = matchRoute(ROUTES, '/sessions/oc_abc%00')
    expect(m?.id).toBe('session-deep-link')
    expect(m?.params['key']).toBe('oc_abc%00')
  })

  it('redirects retired paths (/settings /gateway /about) to /', () => {
    for (const path of ['/settings', '/gateway', '/about']) {
      expect(redirectFor(path)).toBe('/')
      // 退役路径不再有路由定义。
      expect(matchRoute(ROUTES, path)).toBeNull()
    }
  })

  it('admin is deleted outright: no route, no redirect — falls back to the workbench', async () => {
    expect(matchRoute(ROUTES, '/admin/status')).toBeNull()
    expect(redirectFor('/admin/status')).toBeNull()
    window.history.pushState({}, '', '/admin/status')
    window.dispatchEvent(new PopStateEvent('popstate'))
    const el = await mountShell()
    // 未知路径 → workbench fallback，地址栏不动。
    expect(el.shadowRoot!.querySelector('.outlet sebas-dashboard')).toBeTruthy()
    expect(window.location.pathname).toBe('/admin/status')
    el.remove()
  })

  it('navigating to /settings canonicalises the address bar to / and renders the workbench', async () => {
    window.history.pushState({}, '', '/settings')
    window.dispatchEvent(new PopStateEvent('popstate'))
    const el = await mountShell()
    expect(window.location.pathname).toBe('/')
    expect(el.shadowRoot!.querySelector('.outlet sebas-dashboard')).toBeTruthy()
    el.remove()
  })
})

describe('deep-link reachability (workbench-conversation-view 3.1)', () => {
  it('the project rail switches focus IN PLACE — it no longer navigates to deep links', () => {
    const src = readFileSync(join(here, 'views/project-rail.ts'), 'utf8')
    // 点会话 = POST switch + 停在工作台；深链导航已退役。
    expect(src).toContain('api.switchSession')
    expect(src).not.toContain('navigate(`/sessions/')
  })

  it('the shell routes /sessions/:key to the workbench with the deep-link key', () => {
    const src = readFileSync(join(here, 'app-shell.ts'), 'utf8')
    expect(src).toContain('session-deep-link')
    expect(src).toContain('deepLinkKey')
    expect(src).not.toContain('sebas-session-detail')
  })
})

/**
 * add-webui-tiered-notices 3.2：旧 app-shell `ws-banner` 已删除，`/ws` 断线
 * 收编为通知层的持续 warn 驻留横幅（重连即消，`sebas:refetch` 钩子不动）。
 */
describe('ws 断线 → 持续 warn 驻留横幅（add-webui-tiered-notices 3.2）', () => {
  /** 让子组件（通知层/横幅）的 Lit 更新落定。 */
  async function flush(el: SebasApp): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  function layerOf(el: SebasApp): HTMLElement & { shadowRoot: ShadowRoot } {
    const layer = el.shadowRoot!.querySelector('sebas-notice-layer') as unknown as
      | (HTMLElement & { shadowRoot: ShadowRoot })
      | null
    expect(layer).toBeTruthy()
    return layer!
  }

  it('shows the persistent warn banner while /ws is down and clears it on reconnect', async () => {
    const el = await mountShell()
    const layer = layerOf(el)
    // 起点干净：既无断线横幅也无 fatal 横幅。
    expect(layer.shadowRoot.querySelector('[data-testid="ws-down-banner"]')).toBeNull()
    expect(layer.shadowRoot.querySelector('[data-testid="core-fatal-banner"]')).toBeNull()

    window.dispatchEvent(new CustomEvent('sebas:ws-state', { detail: { connected: false } }))
    await flush(el)
    const banner = layer.shadowRoot.querySelector(
      '[data-testid="ws-down-banner"]',
    ) as unknown as { shadowRoot: ShadowRoot }
    expect(banner).toBeTruthy()
    expect(banner.shadowRoot.textContent).toContain('与服务器的连接已断开')
    expect(banner.shadowRoot.querySelector('[role="alert"]')).toBeTruthy()

    // 重连成功：横幅消失（sebas:refetch 刷新由既有钩子负责）。
    window.dispatchEvent(new CustomEvent('sebas:ws-state', { detail: { connected: true } }))
    await flush(el)
    expect(layer.shadowRoot.querySelector('[data-testid="ws-down-banner"]')).toBeNull()
    el.remove()
  })
})


/**
 * add-webui-tiered-notices：「全局核心可达性横幅」MODIFIED 为 fatal 锁定语义。
 * 横幅由 WS 推送驱动（get 初始化 + 翻转通知），`ok=false` 进 fatal——横幅 +
 * 工作台整体 inert 锁定遮罩；恢复即解锁并弹「核心已恢复」info。断线窗口丢
 * 帧由重连后的 get 收敛。旧 `core-unreachable-banner` testid 迁移至
 * `core-fatal-banner`（5.1）。
 */
describe('global core fatal notice (add-webui-tiered-notices)', () => {
  function connectWs(): void {
    window.dispatchEvent(new CustomEvent('sebas:ws-state', { detail: { connected: true } }))
  }

  /** 让 get 往返 / 通知派发 / Lit 重渲染都落定。 */
  async function flush(el: SebasApp): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  afterEach(() => {
    wsMocks.request.mockReset()
    wsMocks.clearHandlers()
  })

  function layerOf(el: SebasApp): HTMLElement & { shadowRoot: ShadowRoot } {
    const layer = el.shadowRoot!.querySelector('sebas-notice-layer') as unknown as
      | (HTMLElement & { shadowRoot: ShadowRoot })
      | null
    expect(layer).toBeTruthy()
    return layer!
  }

  function bannerOf(el: SebasApp): (HTMLElement & { shadowRoot: ShadowRoot }) | null {
    return layerOf(el).shadowRoot.querySelector(
      '[data-testid="core-fatal-banner"]',
    ) as unknown as (HTMLElement & { shadowRoot: ShadowRoot }) | null
  }

  function frameOf(el: SebasApp): HTMLElement {
    return el.shadowRoot!.querySelector('wa-split-panel.frame')!
  }

  it('initializes from core.reachability.get when /ws connects; unknown state shows nothing', async () => {
    wsMocks.request.mockResolvedValue({ ok: false, kind: 'startup_failed', cause: 'socket absent' })
    const el = await mountShell()
    // 未知态（get 未应答）：不渲染横幅也不锁定。
    expect(bannerOf(el)).toBeNull()
    expect(frameOf(el).hasAttribute('inert')).toBe(false)

    connectWs()
    await flush(el)
    // 初始态 = get 响应：fatal 横幅按 kind 分档 + cause 原文，role=alert。
    expect(wsMocks.request).toHaveBeenCalledWith('core.reachability.get')
    const banner = bannerOf(el)
    expect(banner).toBeTruthy()
    expect(banner!.shadowRoot.querySelector('[role="alert"]')).toBeTruthy()
    expect(banner!.shadowRoot.textContent).toContain('核心启动失败')
    expect(banner!.shadowRoot.textContent).toContain('socket absent')
    // 锁定：工作台整体 inert（浏览一并锁）+ 遮罩与原因卡在场。
    expect(frameOf(el).hasAttribute('inert')).toBe(true)
    expect(layerOf(el).shadowRoot.querySelector('[data-testid="core-lock-overlay"]')).toBeTruthy()
    el.remove()
  })

  it('updates on core.reachability flips; recovery clears, unlocks and toasts 核心已恢复', async () => {
    wsMocks.request.mockResolvedValue({ ok: true })
    const el = await mountShell()
    connectWs()
    await flush(el)
    expect(bannerOf(el)).toBeNull()
    expect(frameOf(el).hasAttribute('inert')).toBe(false)

    // 翻转通知（kind + cause）：横幅即时出现，工作台被锁，不等待任何轮询。
    wsMocks.emit({ type: 'core.reachability', ok: false, kind: 'disconnected', cause: 'connection dropped' })
    await flush(el)
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('与核心的连接已断开')
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('connection dropped')
    expect(frameOf(el).hasAttribute('inert')).toBe(true)

    // 恢复通知：无需刷新页面——横幅即时消失、锁定解除，「核心已恢复」info 弹出。
    wsMocks.emit({ type: 'core.reachability', ok: true })
    await flush(el)
    expect(bannerOf(el)).toBeNull()
    expect(frameOf(el).hasAttribute('inert')).toBe(false)
    expect(layerOf(el).shadowRoot.querySelector('[data-testid="core-lock-overlay"]')).toBeNull()
    const items = [...layerOf(el).shadowRoot.querySelectorAll('wa-toast-item')]
    expect(items.some((it) => it.textContent?.includes('核心已恢复'))).toBe(true)
    el.remove()
  })

  it('a kind-less payload degrades to the generic headline (D5)', async () => {
    wsMocks.request.mockResolvedValue({ ok: false, cause: 'mystery' })
    const el = await mountShell()
    connectWs()
    await flush(el)
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('核心不可达')
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('mystery')
    el.remove()
  })

  it('a missed flip during a ws outage converges via the fresh get on reconnect', async () => {
    wsMocks.request.mockResolvedValueOnce({ ok: true })
    const el = await mountShell()
    connectWs()
    await flush(el)
    expect(bannerOf(el)).toBeNull()

    // 断线（持续 warn 横幅接手）；断线窗口 core 翻转——帧已丢失，无人记账。
    window.dispatchEvent(new CustomEvent('sebas:ws-state', { detail: { connected: false } }))
    await el.updateComplete
    wsMocks.request.mockResolvedValueOnce({
      ok: false,
      kind: 'auth_rejected',
      cause: 'core rejected channel handshake',
    })

    // 重连：同一 get 动作带回当前真实状态，横幅与锁定据此收敛。
    connectWs()
    await flush(el)
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('核心拒绝接入')
    expect(bannerOf(el)!.shadowRoot.textContent).toContain('core rejected channel handshake')
    expect(frameOf(el).hasAttribute('inert')).toBe(true)
    el.remove()
  })

  it('auth 门禁页不受锁定影响：login 态无工作台骨架、无通知层实例', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false })
    wsMocks.request.mockResolvedValue({ ok: false, kind: 'disconnected' })
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('sebas-login')).toBeTruthy()
    connectWs()
    await flush(el)
    // 登录/设置流程照常可交互：不渲染通知层（锁定遮罩无处存在），也无骨架可锁。
    expect(el.shadowRoot!.querySelector('sebas-notice-layer')).toBeNull()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeNull()
    el.remove()
  })

  it('retires the poller and the legacy banner markup: no interval, no old testid', () => {
    const src = readFileSync(join(here, 'app-shell.ts'), 'utf8')
    expect(src).not.toContain('CORE_REACHABILITY_POLL_MS')
    expect(src).not.toContain('setInterval')
    expect(src).not.toContain('pollCoreReachability')
    // 5.1：旧横幅实现（ws-banner / core-banner / stacked）整体删除。
    expect(src).not.toContain('core-unreachable-banner')
    expect(src).not.toContain('ws-banner')
    expect(src).not.toContain('core-banner')
    expect(src).not.toContain('stacked')
  })
})

describe('multiuser auth gate (add-webui-multiuser-rbac 5.2/5.4)', () => {
  /** 等 login/setup 成功后的身份重探（checkAuth 的第二次 authMe）落定。 */
  async function afterRecheck(el: SebasApp): Promise<void> {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  function modalRole(el: SebasApp): string | null {
    const modal = el.shadowRoot!.querySelector('sebas-settings-modal') as unknown as {
      role: string | null
    }
    return modal?.role ?? null
  }

  it('renders the first-run setup view while needs_setup is set (zero users)', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false, needs_setup: true })
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('sebas-setup')).toBeTruthy()
    // 设置页与登录页/工作台互斥：建 root 前不渲染任何工作台骨架。
    expect(el.shadowRoot!.querySelector('sebas-login')).toBeNull()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeNull()
    el.remove()
  })

  it('renders the login view when users exist but there is no session (no needs_setup)', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false })
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('sebas-login')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('sebas-setup')).toBeNull()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeNull()
    el.remove()
  })

  /** shell 在 connectedCallback 注册的全局 401 处理器（client 经 mock 捕获）。 */
  function unauthorizedHandler(): () => void {
    const calls = apiMocks.setUnauthorizedHandler.mock.calls
    expect(calls.length).toBeGreaterThan(0)
    return calls[calls.length - 1][0] as () => void
  }

  it('a 401 while on the first-run setup view does NOT flip to the login gate', async () => {
    // 零用户首启姿态：可达性轮询恒 401，登录门无凭据可试——setup 态必须
    // 纹丝不动（webui auth e2e：曾把设置页 5 秒踢翻成登录页，root 引导
    // 旅程断头）。
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false, needs_setup: true })
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('sebas-setup')).toBeTruthy()
    unauthorizedHandler()()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('sebas-setup')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('sebas-login')).toBeNull()
    el.remove()
  })

  it('a 401 outside the setup view (expired session) still flips to the login gate', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: false, authenticated: false })
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeTruthy()
    unauthorizedHandler()()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('sebas-login')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeNull()
    el.remove()
  })

  // （add-core-reachability-ws-push D6）登录/首启设置态的「跳过轮询」特判
  // 随轮询器一起消亡：可达性初始态来自 /ws 连接建立后的 get——auth 开启时
  // 升级前拒绝已天然挡住未认证端，无需任何等价物。

  it('a setup-success enters the workbench and re-fetches the identity (role drives the modal)', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false, needs_setup: true })
    const el = await mountShell()
    apiMocks.authMe.mockResolvedValue({
      enabled: true,
      authenticated: true,
      username: 'cupen',
      role: 'root',
    })
    el.shadowRoot!.querySelector('sebas-setup')!.dispatchEvent(
      new CustomEvent('setup-success', {
        detail: { username: 'cupen' },
        bubbles: true,
        composed: true,
      }),
    )
    await afterRecheck(el)
    expect(el.shadowRoot!.querySelector('.outlet')).toBeTruthy()
    expect(el.shadowRoot!.querySelector('sebas-setup')).toBeNull()
    // 身份重探带回 root：设置弹窗收到角色（5.4 分区裁剪的数据源）。
    expect(modalRole(el)).toBe('root')
    expect(el.shadowRoot!.textContent).toContain('退出 (cupen)')
    el.remove()
  })

  it('a login-success re-fetches the identity and plumbs the member role to the modal', async () => {
    apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: false })
    const el = await mountShell()
    apiMocks.authMe.mockResolvedValue({
      enabled: true,
      authenticated: true,
      username: 'amy',
      role: 'member',
    })
    el.shadowRoot!.querySelector('sebas-login')!.dispatchEvent(
      new CustomEvent('login-success', {
        detail: { username: 'amy' },
        bubbles: true,
        composed: true,
      }),
    )
    await afterRecheck(el)
    expect(el.shadowRoot!.querySelector('.outlet')).toBeTruthy()
    expect(modalRole(el)).toBe('member')
    el.remove()
  })

  it('auth disabled keeps the ready workbench with no role (null passes every section)', async () => {
    const el = await mountShell()
    expect(el.shadowRoot!.querySelector('.outlet')).toBeTruthy()
    expect(modalRole(el)).toBeNull()
    el.remove()
  })
})
