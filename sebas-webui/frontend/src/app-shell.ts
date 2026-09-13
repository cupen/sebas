/**
 * App shell: sidebar (brand + project tree + settings entry) + routed
 * outlet — IA v2, aligned with the preview prototype (`preview/preview-app.ts`):
 * 侧栏承载项目树与 pinned 在底部的 Settings 入口，旧的 NAV_ITEMS 链接列表
 * 已删除（settings/router/about 并入设置弹窗与工作台，admin 直接移除）。
 *
 * workbench-interaction-polish D1/D6：侧栏|主区之间是一道可拖拽的
 * `wa-split-panel` 分割线（180–480px clamp，`sebas.rail-width` 记忆），
 * 底色为 canvas token、侧栏与主区以圆角浮岛浮在其上——区域分隔靠留缝与
 * 色阶，不再靠通高硬线；分割缝 rest 透明、hover 亮起把手。窄屏（<640px）
 * 分割线禁拖、布局退化为既有纵向堆叠。
 *
 * Link interception is document-level (composedPath) so anchors rendered
 * inside any view's shadow root navigate SPA-side too — shadow retargeting
 * hides them from a shell-scoped listener.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { matchRoute, navigate, redirectFor, type RouteDef } from './router.js'
import { api, setUnauthorizedHandler, type Role } from './api/client.js'
import { icon } from './components/icons.js'
import {
  clampRailWidth,
  isNarrowViewport,
  loadRailWidth,
  onNarrowChange,
  RAIL_DEFAULT_PX,
  saveRailWidth,
} from './views/split-persist.js'

// The sidebar tree + settings modal are shell-owned; the outlet views are
// registered in main.ts.
import './views/project-rail.js'
import './views/settings-modal.js'
import './views/login-view.js'
import './views/setup-view.js'
import '@awesome.me/webawesome/dist/components/split-panel/split-panel.js'

// Exported for tests: the route resolution audit iterates these. IA v2 keeps
// only the workbench, the all-sessions table and session deep links;
// `/settings` `/router` `/about` redirect to `/` (see redirectFor) and
// `/admin/*` is deleted outright — it falls through to the dashboard
// fallback in onNavigate like any unknown path. Session deep links render
// the SAME workbench with that session focused（workbench-conversation-view
// 3.4：session-detail 视图退休，工作台是唯一对话面）。
export const ROUTES: RouteDef[] = [
  { id: 'dashboard', pattern: '/' },
  // `/sessions` stays routed (History group header link + old deep links).
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session-deep-link', pattern: '/sessions/:key' },
]

@customElement('sebas-app')
export class SebasApp extends LitElement {
  @state() private routeId: string = 'dashboard'

  /**
   * 登录鉴权门禁（webui auth）：checking = /api/auth/me 探测中；setup =
   * 服务端启用鉴权且用户库零用户（首启设置页，add-webui-multiuser-rbac
   * 5.2，由 needs_setup 驱动）；login = 已有用户但当前无有效会话（渲染登录
   * 页替代工作台）；ready = 放行。
   */
  @state() private authState: 'checking' | 'login' | 'setup' | 'ready' = 'checking'
  /** 已登录账户名（仅用于侧栏登出入口与登录页预填；null = 未登录/未启用）。 */
  @state() private authUsername: string | null = null
  /**
   * 当前登录用户的角色（add-webui-multiuser-rbac 5.4）：呈现层据此隐藏
   * 无权限入口（settings 弹窗分区等）；防线仍在服务端路由层，这里只是
   * D8 的呈现优化。null = 未认证或服务端未启用鉴权。
   */
  @state() private authRole: Role | null = null

  /**
   * Selected project path, owned here so the sidebar tree and the workbench
   * main area stay in sync across route changes (the rail is shell-mounted
   * now, the dashboard only consumes it).
   */
  @state() private selectedPath: string | null = null
  /** Whether the centered settings modal is open (sidebar entry toggles it). */
  @state() private settingsOpen = false
  /**
   * 侧栏宽度（px；workbench-interaction-polish 5.1/D1）：localStorage 记忆，
   * 拖拽 rail|main 分割线时经 clamp 后写回。
   */
  @state() private railWidth: number = loadRailWidth() ?? RAIL_DEFAULT_PX
  /** 窄屏（<640px）：分割线禁拖，布局退化既有纵向堆叠。 */
  @state() private narrow: boolean = isNarrowViewport()
  /**
   * `/ws` 断线中（add-webui-allowed-roots D6）：共享 WS 客户端经
   * `sebas:ws-state` 广播连接状态，顶部横幅提示操作者当前视图可能冻结，
   * 重连成功即消失并触发 `sebas:refetch` 刷新。
   */
  @state() private wsDown = false
  /**
   * 全局「核心不可达」横幅（harden-core-channel-deployment 4.1/D6）：自持
   * `/api/summary` 轮询（与 composer 的 reachability 同间隔），`ok=false`
   * 时以 `role=alert` 呈现 cause 原文，恢复即消失。与 ws-banner 同层（骑在
   * 出口区顶部、不阻塞浏览），判别器当前是 `ok+cause`——cover-A 的 `kind`
   * 字段落地后在此替换文案选择，cause 保持原文渲染。
   */
  @state() private coreUnreachableCause: string | null = null

  private static readonly CORE_REACHABILITY_POLL_MS = 5_000
  private corePollTimer: number | undefined = undefined

  private params: Record<string, string> = {}
  private onNavigateBound: () => void = () => {}
  private onClick: (e: MouseEvent) => void = () => {}
  /** 窄屏媒体查询退订句柄（5.1）。 */
  private unlistenNarrow: (() => void) | null = null

  /**
   * 拖拽 rail|main 分割线（5.1/D1）：换算 px、clamp、写 localStorage 并
   * 同步状态（position-in-pixels 绑定随之更新）。初始化时像素/百分比换算
   * 可能短暂产生非有限值——忽略，绝不拿垃圾值覆盖已存的宽度。
   */
  private onRailReposition = (e: Event): void => {
    const panel = e.currentTarget as HTMLElement & { positionInPixels: number }
    const raw = panel.positionInPixels
    if (!Number.isFinite(raw)) return
    const px = clampRailWidth(raw)
    this.railWidth = px
    saveRailWidth(px)
  }

  static styles = css`
    :host {
      /* 应用框架（预览原型同款）：100vh 固定高度 + overflow hidden。
         workbench-interaction-polish D6：底色换 canvas token（比 surface 深
         一档），rail / 主区以浮岛浮在其上，区域间用留缝替代通高硬线。 */
      display: flex;
      width: 100vw;
      height: 100vh;
      min-height: 0;
      overflow: hidden;
      background: var(--sebas-canvas, var(--sebas-bg));
      background-image: radial-gradient(1100px 480px at 82% -12%, rgba(91, 100, 242, 0.09), transparent 62%),
        radial-gradient(900px 420px at -8% 108%, rgba(56, 209, 221, 0.05), transparent 60%);
      background-attachment: fixed;
      color: var(--sebas-text);
    }
    /* ── 侧栏|主区 wa-split-panel（5.1/D1）────────────────────────────
       primary=start：窗口缩放时侧栏保持 px 宽度（180–480 由 --min/--max
       兜底，状态里另有 clamp）。分隔缝 rest 态透明（浮岛留缝），hover 亮
       起把手。 */
    wa-split-panel.frame {
      flex: 1;
      min-height: 0;
      min-width: 0;
      --min: 180px;
      --max: 480px;
      /* 缝宽 = 浮岛间距（rest 透明时就是纯留白）。 */
      --divider-width: 12px;
    }
    wa-split-panel.frame::part(panel) {
      min-width: 0;
      min-height: 0;
    }
    wa-split-panel.frame::part(divider) {
      background: transparent;
      border-radius: var(--sebas-radius-full);
      transition: background var(--sebas-dur) var(--sebas-ease);
    }
    wa-split-panel.frame::part(divider):hover {
      background: var(--sebas-border-strong);
    }
    nav {
      /* 浮岛（D6）：圆角 surface 卡片浮在 canvas 上，与分割缝一起构成
         区域分隔——不再有通高 1px 硬边。 */
      box-sizing: border-box; /* 高度吃进 padding，否则 100vh+padding 撑破框架 */
      min-height: 0; /* flex/grid 项默认 min-height:auto 会撑破 100vh 框架 */
      min-width: 0;
      margin: var(--sebas-space-3);
      overflow-y: auto;
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-xl);
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
      /* 浮岛间距（D6）：左侧留给分割缝，其余三边自己留白。 */
      margin: var(--sebas-space-3) var(--sebas-space-3) var(--sebas-space-3) 0;
      display: flex;
      flex-direction: column;
      position: relative; /* 断线横幅的定位上下文 */
    }
    /* 全局断线横幅（add-webui-allowed-roots D6）：骑在出口区顶部，不占
       布局流——断线期间视图本就可能冻结，横幅不应把内容顶来顶去。 */
    .ws-banner {
      position: absolute;
      top: 0;
      left: 0;
      right: 0;
      z-index: 10;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      padding: 6px 12px;
      background: var(--sebas-status-warn, #b45309);
      color: #fff;
      font-size: 0.8rem;
      font-weight: 500;
    }
    .ws-banner svg {
      flex: 0 0 auto;
    }
    .core-banner.stacked {
      top: 30px;
    }
    /* 全局「核心不可达」横幅（4.1）：与 ws-banner 同款定位；两者同时在场
       （ws 断线 + core 不可达）时纵向堆叠，互不遮挡。 */
    .core-banner {
      background: var(--sebas-status-failed, #b91c1c);
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
      /* 窄屏退化（5.1：分割线不可拖、布局回到既有纵向堆叠）：grid 面板
         改 flex 纵排、分隔缝隐藏；slot 面板恢复文档流高度。 */
      wa-split-panel.frame {
        display: flex;
        flex-direction: column;
        --divider-width: 0px;
      }
      wa-split-panel.frame::part(divider) {
        display: none;
      }
      nav {
        margin: var(--sebas-space-2);
        flex-direction: row;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--sebas-space-1);
        padding: var(--sebas-space-3) var(--sebas-space-4);
        overflow-y: visible;
      }
      main {
        margin: 0 var(--sebas-space-2) var(--sebas-space-2);
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
    // 5.1：窄屏翻转 → 分割线禁拖（布局退化由 CSS 媒体查询承接）。
    this.unlistenNarrow = onNarrowChange((n) => (this.narrow = n))
    // add-webui-allowed-roots D6：WS 连接状态 → 全局断线横幅。
    window.addEventListener('sebas:ws-state', this.onWsState)
    // harden-core-channel-deployment 4.1：全局核心可达性轮询（与 composer
    // 的 WORKBENCH_REACHABILITY_POLL_MS 同间隔）。
    this.corePollTimer = window.setInterval(
      () => void this.pollCoreReachability(),
      SebasApp.CORE_REACHABILITY_POLL_MS,
    )
    void this.pollCoreReachability()
    // 会话过期 / 中途启用鉴权：任何 API 401 都把界面切回登录页。
    setUnauthorizedHandler(() => this.showLogin())
    this.onNavigate()
    void this.checkAuth()
  }

  private async pollCoreReachability(): Promise<void> {
    try {
      const summary = await api.summary()
      const ok = summary.reachability?.ok !== false
      this.coreUnreachableCause = ok
        ? null
        : (summary.reachability?.cause ?? 'core not connected')
    } catch {
      // summary 本身失败（webui 重启窗口等）不推翻既有状态：下一轮重试。
    }
  }

  private onWsState = (e: Event): void => {
    this.wsDown = (e as CustomEvent<{ connected: boolean }>).detail?.connected === false
  }

  /**
   * 探明服务端鉴权状态，决定渲染首启设置页、登录页还是工作台
   * （add-webui-multiuser-rbac 5.2：needs_setup 驱动 setup 态）。
   */
  private async checkAuth(): Promise<void> {
    try {
      const info = await api.authMe()
      if (info.enabled && !info.authenticated) {
        this.authUsername = null
        this.authRole = null
        // 零用户首启 → 设置页建 root；否则常规登录页。
        this.authState = info.needs_setup ? 'setup' : 'login'
        return
      }
      this.authUsername = info.authenticated ? info.username : null
      this.authRole = info.authenticated ? (info.role ?? null) : null
      this.authState = 'ready'
    } catch {
      if (this.authState === 'checking') {
        // /api/auth/me 本身失败（网络/服务异常）：按未启用处理，后续请求的
        // 401 会经 setUnauthorizedHandler 再切回登录页。
        this.authState = 'ready'
      }
      // 登录/设置成功后的身份重探失败：维持当前视图，操作者可重试或登出。
    }
  }

  private showLogin(): void {
    this.authState = 'login'
  }

  /**
   * 登录成功：先放行进工作台（username 取自登录响应），再重探
   * /api/auth/me 取权威身份——登录响应不带 role，角色驱动的入口裁剪
   * （5.4）以 me 的读数为准。
   */
  private onLoginSuccess = (e: Event): void => {
    this.authUsername = (e as CustomEvent<{ username: string | null }>).detail?.username ?? null
    this.authState = 'ready'
    void this.checkAuth()
  }

  /** 首启设置成功（root 已建、会话已立）：与登录同一放行路径。 */
  private onSetupSuccess = (e: Event): void => {
    this.authUsername = (e as CustomEvent<{ username: string | null }>).detail?.username ?? null
    this.authState = 'ready'
    void this.checkAuth()
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
    this.unlistenNarrow?.()
    window.removeEventListener('sebas:ws-state', this.onWsState)
    if (this.corePollTimer !== undefined) {
      window.clearInterval(this.corePollTimer)
      this.corePollTimer = undefined
    }
    super.disconnectedCallback()
  }

  private onNavigate(): void {
    // Retired IA-v1 paths (/settings /router /about) canonicalise to `/`
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
   * Full-bleed outlet routes: the workbench (`/` and the `/sessions/:key`
   * deep link, which renders the same workbench focused) are app-frame
   * panes — the outlet carries no padding and the view flexes to fill the
   * frame, scrolling internally. Document routes (the `/sessions` table)
   * keep the readable 1080px padded column.
   */
  private isWideRoute(): boolean {
    return this.routeId === 'dashboard' || this.routeId === 'session-deep-link'
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
      case 'session-deep-link':
        // 深链渲染同一个工作台并聚焦该会话（读 detail 即设置服务端焦点
        // 指针）——没有独立详情页。
        return html`<sebas-dashboard
          .selectedPath=${this.selectedPath}
          .deepLinkKey=${this.params['key'] ?? null}
        ></sebas-dashboard>`
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
      return html`<sebas-login @login-success=${this.onLoginSuccess}></sebas-login>`
    }
    if (this.authState === 'setup') {
      // 首启（零用户）：建 root 前不渲染任何工作台骨架。
      return html`<sebas-setup @setup-success=${this.onSetupSuccess}></sebas-setup>`
    }
    return html`
      <wa-split-panel
        class="frame"
        orientation="horizontal"
        primary="start"
        position-in-pixels=${this.railWidth}
        ?disabled=${this.narrow}
        @wa-reposition=${this.onRailReposition}
      >
        <nav slot="start" aria-label="Primary">
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
        <main slot="end" @open-settings=${() => (this.settingsOpen = true)}>
          ${this.wsDown
            ? html`<div class="ws-banner" role="alert">
                ${icon('alert', 14)}<span>与服务器的连接已断开，正在重连…（当前显示可能已过期）</span>
              </div>`
            : nothing}
          ${this.coreUnreachableCause !== null
            ? html`<div class="ws-banner core-banner${this.wsDown ? ' stacked' : ''}" role="alert" data-testid="core-unreachable-banner">
                ${icon('alert', 14)}<span>核心不可达：${this.coreUnreachableCause}（会话与项目面暂不可用，页面浏览不受影响）</span>
              </div>`
            : nothing}
          <div class="outlet${this.isWideRoute() ? '' : ' padded'}">${this.renderOutlet()}</div>
        </main>
      </wa-split-panel>
      <sebas-settings-modal
        .role=${this.authRole}
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
