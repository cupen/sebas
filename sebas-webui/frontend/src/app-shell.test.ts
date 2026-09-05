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

// ---- hoisted mocks（须先于被测模块的静态导入生效）----------------------

const apiMocks = vi.hoisted(() => ({
  summary: vi.fn(),
  sessions: vi.fn(),
  settings: vi.fn(),
  projectsList: vi.fn(),
  projectsBranch: vi.fn(),
  projectsAdd: vi.fn(),
  projectsReorder: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  api: {
    summary: apiMocks.summary,
    sessions: apiMocks.sessions,
    settings: apiMocks.settings,
    projects: {
      list: apiMocks.projectsList,
      branch: apiMocks.projectsBranch,
      add: apiMocks.projectsAdd,
      reorder: apiMocks.projectsReorder,
    },
  },
}))

vi.mock('../api/shared-ws.js', () => ({
  sharedWs: { subscribe: () => () => {} },
}))

// outlet 会实例化 sebas-dashboard；composer 依赖 WA 表单组件的
// ElementInternals（jsdom 不完整），与 shell 无关 —— mock 掉模块即可，
// <sebas-workbench-composer> 作为未知元素惰性渲染。
vi.mock('./views/workbench-composer.js', () => ({}))

// ---- 被测模块（mock 生效后导入）----------------------------------------

import { matchRoute, redirectFor } from './router.js'
import { ROUTES, SebasApp } from './app-shell.js'
import './views/dashboard.js'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
// fileURLToPath: Windows 下 URL().pathname 会得到 "/D:/..."，join 后成 "D:\D:\..."
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

const projectFixture = { path: '/home/me/sebas', name: 'sebas', added_at: 1, branch: 'main' }

beforeEach(() => {
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
    // flex、侧栏 220px + min-height:0、main/outlet 满幅 flex 列。
    const styleText = [...root.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(styleText).toMatch(/:host\s*\{[^}]*display:\s*flex/)
    expect(styleText).toContain('height: 100vh')
    expect(styleText).toContain('overflow: hidden')
    expect(styleText).toContain('width: 220px')
    expect(styleText).toMatch(/nav\s*\{[^}]*min-height:\s*0/)
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
    // key 保持 RAW（%00 NUL 回归）。
    const m = matchRoute(ROUTES, '/sessions/oc_abc%00')
    expect(m?.id).toBe('session-detail')
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

describe('deep-link reachability', () => {
  it('the project rail still navigates to /sessions/:key via session rows', () => {
    const src = readFileSync(join(here, 'views/project-rail.ts'), 'utf8')
    expect(src).toContain('navigate(`/sessions/')
  })
})
