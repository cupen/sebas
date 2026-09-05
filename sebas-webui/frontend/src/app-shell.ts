/**
 * App shell: sidebar (brand + project tree + settings entry) + routed
 * outlet — IA v2, aligned with the preview prototype (`preview/preview-app.ts`):
 * 侧栏承载项目树与 pinned 在底部的 Settings 入口，旧的 NAV_ITEMS 链接列表
 * 已删除（settings/gateway/about 并入设置弹窗与工作台，admin 直接移除）。
 *
 * Link interception is document-level (composedPath) so anchors rendered
 * inside any view's shadow root navigate SPA-side too — shadow retargeting
 * hides them from a shell-scoped listener. The workbench composer's
 * "settings →" control likewise crosses shadow boundaries: it dispatches a
 * bubbling composed `open-settings` event that <main> catches here to open
 * the centered settings modal (the old `/settings` route redirects to `/`).
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { matchRoute, navigate, redirectFor, type RouteDef } from './router.js'
import { api, setUnauthorizedHandler } from './api/client.js'
import { icon } from './components/icons.js'

// The sidebar tree + settings modal are shell-owned; the outlet views are
// registered in main.ts.
import './views/project-rail.js'
import './views/settings-modal.js'
import './views/login-view.js'

// Exported for tests: the route resolution audit iterates these. IA v2 keeps
// only the workbench, the all-sessions table and session deep links;
// `/settings` `/gateway` `/about` redirect to `/` (see redirectFor) and
// `/admin/*` is deleted outright — it falls through to the dashboard
// fallback in onNavigate like any unknown path.
export const ROUTES: RouteDef[] = [
  { id: 'dashboard', pattern: '/' },
  // `/sessions` stays routed (History group header link + old deep links).
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session-detail', pattern: '/sessions/:key' },
]

@customElement('sebas-app')
export class SebasApp extends LitElement {
  @state() private routeId: string = 'dashboard'

  /**
   * 登录鉴权门禁（webui auth）：checking = /api/auth/me 探测中；login =
   * 服务端启用鉴权且当前无有效会话（渲染登录页替代工作台）；ready = 放行。
   */
  @state() private authState: 'checking' | 'login' | 'ready' = 'checking'
  /** 已登录账户名（仅用于侧栏登出入口与登录页预填；null = 未登录/未启用）。 */
  @state() private authUsername: string | null = null

  /**
   * Selected project path, owned here so the sidebar tree and the workbench
   * main area stay in sync across route changes (the rail is shell-mounted
   * now, the dashboard only consumes it).
   */
  @state() private selectedPath: string | null = null
  /** Whether the centered settings modal is open (sidebar entry toggles it). */
  @state() private settingsOpen = false

  private params: Record<string, string> = {}
  private onNavigateBound: () => void = () => {}
  private onClick: (e: MouseEvent) => void = () => {}

  static styles = css`
    :host {
      /* 应用框架（预览原型同款）：100vh 固定高度 + overflow hidden，
         侧栏与出口区各自内部滚动，页面本身不滚。环境渐变背景照抄
         preview-app.ts。 */
      display: flex;
      width: 100vw;
      height: 100vh;
      min-height: 0;
      overflow: hidden;
      background: var(--sebas-bg);
      background-image: radial-gradient(1100px 480px at 82% -12%, rgba(91, 100, 242, 0.09), transparent 62%),
        radial-gradient(900px 420px at -8% 108%, rgba(56, 209, 221, 0.05), transparent 60%);
      background-attachment: fixed;
      color: var(--sebas-text);
    }
    nav {
      width: 220px;
      flex: 0 0 auto;
      position: sticky;
      top: 0;
      height: 100vh;
      box-sizing: border-box; /* 高度吃进 padding，否则 100vh+padding 撑破框架 */
      min-height: 0; /* flex 项默认 min-height:auto 会撑破 100vh 框架 */
      overflow-y: auto;
      background: var(--sebas-surface);
      border-right: 1px solid var(--sebas-border);
      padding: var(--sebas-space-4) var(--sebas-space-3);
      display: flex;
      flex-direction: column;
      gap: 2px;
    }
    .brand {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-2) var(--sebas-space-2) var(--sebas-space-4);
      text-decoration: none;
      color: var(--sebas-text-bright);
    }
    .brand .mark {
      display: grid;
      place-items: center;
      width: 28px;
      height: 28px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-md);
      background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
      color: var(--sebas-accent-ink);
      font-family: var(--sebas-font-mono);
      font-size: 0.9rem;
      font-weight: 700;
      box-shadow:
        var(--sebas-shadow-1),
        inset 0 1px 0 rgba(255, 255, 255, 0.18);
    }
    .brand .name {
      font-weight: 700;
      font-size: 1rem;
      letter-spacing: 0.01em;
    }
    .brand .name small {
      display: block;
      font-weight: 500;
      font-size: 0.66rem;
      letter-spacing: 0.09em;
      text-transform: uppercase;
      color: var(--sebas-text-faint);
    }
    /* Pinned settings entry (预览原型同款 sticky footer)：树滚动时按钮
     * 始终钉在侧栏可见底部。 */
    .sidebar-footer {
      position: sticky;
      bottom: calc(-1 * var(--sebas-space-4)); /* 抵消 nav 的底部 padding */
      margin-top: auto;
      background: var(--sebas-surface);
      padding-top: var(--sebas-space-2);
      z-index: 1;
    }
    .settings-btn {
      display: flex;
      align-items: center;
      gap: 10px;
      width: 100%;
      padding: 7px 10px;
      border: none;
      border-radius: var(--sebas-radius-md);
      background: none;
      color: var(--sebas-text-dim);
      font: inherit;
      font-size: 0.85rem;
      font-weight: 500;
      text-align: left;
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .settings-btn:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .settings-btn svg {
      opacity: 0.8;
      flex: 0 0 auto;
    }
    .settings-btn:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .spacer {
      flex: 0 0 8px;
    }
    main {
      flex: 1;
      min-width: 0;
      min-height: 0;
      display: flex;
      flex-direction: column;
    }
    .outlet {
      /* 满幅工作台：workbench 类路由（/ 与 /sessions/:key）直接铺满
         出口区（去掉居中窄栏），滚动交给视图内部（turn-stream）。 */
      flex: 1;
      min-height: 0;
      min-width: 0;
      display: flex;
      flex-direction: column;
      position: relative; /* 子视图定位上下文 */
    }
    /* 文档型路由（/sessions 表格页）维持 1080px 可读列宽并自行滚动：
       与预览原型“全屏应用”的差异在 IA 上是刻意的（表格页是次级页）。 */
    .outlet.padded {
      width: 100%;
      max-width: 1080px;
      margin: 0 auto;
      padding: var(--sebas-space-6) var(--sebas-space-8);
      overflow-y: auto;
    }
    /* Route change mounts a fresh view — replay a soft rise-in. */
    .outlet > * {
      animation: sebas-view-in 0.28s var(--sebas-ease) both;
      min-height: 0;
    }
    @keyframes sebas-view-in {
      from {
        opacity: 0;
        transform: translateY(6px);
      }
      to {
        opacity: 1;
        transform: none;
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .outlet > * {
        animation: none;
      }
    }
    @media (max-width: 900px) {
      /* 无 main padding——窄屏只收窄文档型路由的内边距。 */
      .outlet.padded {
        padding: var(--sebas-space-5) var(--sebas-space-4);
      }
    }
    @media (max-width: 640px) {
      :host {
        flex-direction: column;
      }
      nav {
        position: static;
        height: auto;
        width: auto;
        flex-direction: row;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--sebas-space-1);
        border-right: none;
        border-bottom: 1px solid var(--sebas-border);
        padding: var(--sebas-space-3) var(--sebas-space-4);
      }
      .brand {
        padding: 0 var(--sebas-space-4) 0 0;
      }
      .brand .name small {
        display: none;
      }
      /* 窄屏时项目树收进顶栏之外（预览原型的 Projects/Chat 顶页签明确
       * 不在本期范围内）——只留品牌 + Settings 图标，保证 375px 无横向
       * 滚动；/sessions 仍可经会话详情页的 "All sessions" 回链到达。 */
      sebas-project-rail,
      .spacer {
        display: none;
      }
      .sidebar-footer {
        position: static;
        margin-top: 0;
        padding-top: 0;
        background: none;
      }
      .settings-btn {
        width: auto;
        margin-left: auto;
        padding: 6px;
        gap: 0;
      }
      .settings-btn .settings-label {
        display: none;
      }
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    this.onNavigateBound = this.onNavigate.bind(this)
    this.onClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey)
        return
      for (const node of e.composedPath()) {
        if (!(node instanceof HTMLAnchorElement)) continue
        const href = node.getAttribute('href')
        if (href && href.startsWith('/')) {
          e.preventDefault()
          navigate(href)
        }
        break
      }
    }
    window.addEventListener('popstate', this.onNavigateBound)
    document.addEventListener('click', this.onClick)
    // 会话过期 / 中途启用鉴权：任何 API 401 都把界面切回登录页。
    setUnauthorizedHandler(() => this.showLogin())
    this.onNavigate()
    void this.checkAuth()
  }

  /** 探明服务端鉴权状态，决定渲染登录页还是工作台。 */
  private async checkAuth(): Promise<void> {
    try {
      const info = await api.authMe()
      if (info.enabled && !info.authenticated) {
        this.authUsername = null
        this.authState = 'login'
        return
      }
      this.authUsername = info.authenticated ? info.username : null
      this.authState = 'ready'
    } catch {
      // /api/auth/me 本身失败（网络/服务异常）：按未启用处理，后续请求的
      // 401 会经 setUnauthorizedHandler 再切回登录页。
      this.authState = 'ready'
    }
  }

  private showLogin(): void {
    this.authState = 'login'
  }

  private onLoginSuccess = (e: Event): void => {
    this.authUsername = (e as CustomEvent<{ username: string }>).detail.username
    this.authState = 'ready'
  }

  private async onLogout(): Promise<void> {
    try {
      await api.authLogout()
    } catch {
      // 注销失败（会话已过期等）也无妨：照样回登录页。
    }
    this.showLogin()
  }

  disconnectedCallback(): void {
    window.removeEventListener('popstate', this.onNavigateBound)
    document.removeEventListener('click', this.onClick)
    super.disconnectedCallback()
  }

  private onNavigate(): void {
    // Retired IA-v1 paths (/settings /gateway /about) canonicalise to `/`
    // before matching, so address bar and rendered view agree.
    const retired = redirectFor(location.pathname)
    if (retired) history.replaceState({}, '', retired)
    const match = matchRoute(ROUTES, location.pathname)
    if (!match) {
      // Unknown path (incl. the deleted /admin/*): render the workbench
      // rather than a dead screen.
      this.routeId = 'dashboard'
      this.params = {}
    } else {
      this.routeId = match.id
      this.params = match.params
    }
  }

  /**
   * Full-bleed outlet routes: the workbench (`/`) and the session detail
   * (`/sessions/:key`) are app-frame panes — the outlet carries no padding
   * and the view flexes to fill the frame, scrolling internally. Document
   * routes (the `/sessions` table) keep the readable 1080px padded column.
   */
  private isWideRoute(): boolean {
    return this.routeId === 'dashboard' || this.routeId === 'session-detail'
  }

  /** 侧栏项目树选中项目 → 记录并回到 workbench（其它路由上点树也要生效）。 */
  private onRailSelect = (e: Event): void => {
    this.selectedPath = (e as CustomEvent<{ path: string | null }>).detail.path
    if (location.pathname !== '/') navigate('/')
  }

  private renderOutlet() {
    switch (this.routeId) {
      case 'dashboard':
        return html`<sebas-dashboard .selectedPath=${this.selectedPath}></sebas-dashboard>`
      case 'sessions':
        return html`<sebas-sessions></sebas-sessions>`
      case 'session-detail':
        return html`<sebas-session-detail key=${this.params['key'] ?? ''}></sebas-session-detail>`
      default:
        return html`<sebas-dashboard .selectedPath=${this.selectedPath}></sebas-dashboard>`
    }
  }

  render() {
    if (this.authState === 'checking') {
      // 鉴权探测期间先不渲染任何内容，避免登录页/工作台闪现。
      return html``
    }
    if (this.authState === 'login') {
      return html`<sebas-login
        .hintUsername=${this.authUsername}
        @login-success=${this.onLoginSuccess}
      ></sebas-login>`
    }
    return html`
      <nav aria-label="Primary">
        <a class="brand" href="/" aria-label="sebas console home">
          <span class="mark" aria-hidden="true">❯</span>
          <span class="name">sebas<small>agent router</small></span>
        </a>
        <sebas-project-rail
          .activePath=${this.selectedPath}
          @rail-select=${this.onRailSelect}
        ></sebas-project-rail>
        <div class="spacer" aria-hidden="true"></div>
        <div class="sidebar-footer">
          ${this.authUsername
            ? html`<button
                class="settings-btn"
                aria-label="Sign out"
                title="退出登录"
                @click=${() => void this.onLogout()}
              >
                ${icon('logout', 16)}<span class="settings-label">退出 (${this.authUsername})</span>
              </button>`
            : nothing}
          <button
            class="settings-btn"
            aria-haspopup="dialog"
            aria-label="Open settings"
            @click=${() => (this.settingsOpen = true)}
          >
            ${icon('settings', 16)}<span class="settings-label">Settings</span>
          </button>
        </div>
      </nav>
      <main
        @open-settings=${() => (this.settingsOpen = true)}
      ><div class="outlet${this.isWideRoute() ? '' : ' padded'}">${this.renderOutlet()}</div></main>
      <sebas-settings-modal
        ?open=${this.settingsOpen}
        @close=${() => (this.settingsOpen = false)}
      ></sebas-settings-modal>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-app': SebasApp
  }
}
