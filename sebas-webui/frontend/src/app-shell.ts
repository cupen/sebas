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
import { api, setUnauthorizedHandler, type ArchiveEntry, type Role } from './api/client.js'
import { sharedWs } from './api/shared-ws.js'
import type { CoreReachabilityState } from './api/ws.js'
import { notify, setFatal, setWsDown } from './notify.js'
import { APP_TAGLINE } from './branding.js'
import { icon } from './components/icons.js'
import './components/notice-layer.js'
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
import { RAIL_FOCUS_EVENT } from './views/project-rail.js'
// （fix-webui-approval-restore-and-session-identity 4.1，design D4）聚焦会话
// 反投影项目上下文的事件名（dashboard 发、shell 收——selectedPath 单一所有权）。
import { PROJECT_FOLLOW_EVENT } from './views/dashboard.js'
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
   * 全局「核心不可达」fatal 通知（add-core-reachability-ws-push D4 →
   * add-webui-tiered-notices 2.4）：订阅权上收 shell——`/ws` 连接建立/重连
   * 即 `core.reachability.get` 初始化，此后随 `core.reachability` 翻转通知
   * 即时更新（不再轮询 /api/summary）。结构化 ok/kind/cause 与
   * `/api/summary` 的 reachability 段同形（D5）；`ok=false` 时本状态一路
   * 三用：store 的 fatal 槽位（横幅 + 锁定遮罩）、wa-split-panel 的 inert
   * （工作台整体锁定）、下传 dashboard/composer 的提交门。`null` = 未知
   * （get 未应答/连接未立）——不渲染横幅不锁定。
   */
  @state() private coreReachability: CoreReachabilityState | null = null
  /**
   * （polish-workbench-walkthrough-ux 2.1）正在查看的只读归档条目（rail
   * History 点击上报，本处接力给 dashboard 渲染）。`null` = 无归档视图。
   * 选项目 / 恢复成功 / 显式关闭都会清空。
   */
  @state() private archivedEntry: ArchiveEntry | null = null

  private params: Record<string, string> = {}
  private onNavigateBound: () => void = () => {}
  private onClick: (e: MouseEvent) => void = () => {}
  /** 窄屏媒体查询退订句柄（5.1）。 */
  private unlistenNarrow: (() => void) | null = null
  /** 共享 WS 客户端的事件订阅退订句柄（可达性翻转推送）。 */
  private unsubscribeWs: (() => void) | null = null

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

  /**
   * 布局自愈（走查发现）：`wa-split-panel` 的 `pixelsToPercentage` 是
   * `px / size * 100`，而首次渲染把 `position-in-pixels` 灌进去时容器尺寸可能
   * 还没定（`size` 为 0/NaN）——换算出的 `position` 成了 `NaN`/`Infinity`，
   * 生成的 `grid-template-columns` 里就带上非法的 `NaN%`/`Infinity%`，整条
   * 声明失效、grid 退化成单轨道：rail 铺满整宽、行内操作按钮被推到可视区外
   * 点不到，刷新也照样崩（组件自带的 ResizeObserver 修复分支只认 `Infinity`，
   * 漏了 `NaN`）。
   *
   * 每次更新后校验一次：`position` 非有限就把宽度重新灌回去触发重算。重算延到
   * 下一帧，因为首帧里容器可能仍未定尺寸、立刻重算会再得到非有限值；而下一帧
   * 布局已稳定，重算即得正确百分比。正常情形 `position` 有限，此处不做任何事。
   */
  protected updated(): void {
    const panel = this.renderRoot.querySelector<
      HTMLElement & { position: number; positionInPixels: number }
    >('wa-split-panel.frame')
    // 只在「已是数字但非有限」时介入：组件未升级（position 未定义）或换算
    // 正常（有限数字）都不打扰。
    if (!panel || typeof panel.position !== 'number' || Number.isFinite(panel.position)) return
    requestAnimationFrame(() => {
      if (typeof panel.position === 'number' && !Number.isFinite(panel.position)) {
        panel.positionInPixels = this.railWidth
      }
    })
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
      /* （workbench-live-conversation-flow 5.2）上限 520 与
         split-persist 的 RAIL_MAX_PX 一致。 */
      --max: 520px;
      /* （5.1）缝宽 6px：rail 与工作台视觉相邻，把手 hover 亮起。 */
      --divider-width: 6px;
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
      /* （3.5，design D6）nav 浮岛外边距 space-3 → space-2：rail|主区间距
         收敛为紧凑 token，拖拽分割缝可达性不变。 */
      margin: var(--sebas-space-2);
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
      /* 浮岛间距（D6）：左侧留给分割缝，其余三边自己留白。（3.5）三边
         margin space-3 → space-2——浮岛间距收敛为紧凑 token。 */
      margin: var(--sebas-space-2) var(--sebas-space-2) var(--sebas-space-2) 0;
      display: flex;
      flex-direction: column;
      position: relative; /* 子视图定位上下文 */
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
    /* fix-webui-mobile-polish：抽屉开关/遮罩/关闭钮仅窄屏出现，桌面不渲染
       布局影响（base 隐藏；窄屏规则见下方媒体块）。 */
    .rail-toggle,
    .rail-close {
      display: none;
    }
    .rail-toggle {
      align-items: center;
      justify-content: center;
      width: 38px;
      height: 38px;
      padding: 0;
      border: 1px solid var(--sebas-border, rgba(128, 138, 160, 0.35));
      border-radius: var(--sebas-radius-full, 10px);
      background: var(--sebas-surface, #fff);
      color: var(--sebas-text);
      cursor: pointer;
    }
    .nav-scrim {
      position: fixed;
      inset: 0;
      z-index: 60;
      border: 0;
      padding: 0;
      background: rgba(8, 10, 18, 0.45);
      cursor: default;
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
      /* fix-webui-mobile-polish：nav 整体成为抽屉（品牌/rail/设置/登出都在
         里面），顶栏 ☰ 悬浮常驻、主区让出顶部一条，避免覆盖工作台头部。 */
      .rail-toggle {
        display: inline-flex;
        position: fixed;
        top: 10px;
        left: 10px;
        z-index: 80;
      }
      main {
        margin: 52px var(--sebas-space-2) var(--sebas-space-2);
      }
      .brand .name small {
        display: none;
      }
      /* fix-webui-mobile-polish：项目树改为**抽屉**——顶栏 ☰ 呼出，左滑入、
         遮罩/关闭钮/选中会话收起。此前窄屏直接 display:none，手机上没有
         任何入口可选项目与会话（工作台不可用）。 */
      .rail-toggle {
        display: inline-flex;
      }
      nav.rail-drawer {
        position: fixed;
        top: 0;
        bottom: 0;
        left: 0;
        width: min(85vw, 320px);
        z-index: 70;
        margin: 0;
        border-radius: 0;
        flex-direction: column;
        padding: var(--sebas-space-3);
        overflow-y: auto;
        transform: translateX(-102%);
        transition: transform var(--sebas-dur, 0.2s) var(--sebas-ease, ease);
        background: var(--sebas-canvas, var(--sebas-bg));
      }
      nav.rail-drawer.open {
        transform: none;
        box-shadow: 0 12px 48px rgba(8, 10, 18, 0.4);
      }
      nav.rail-drawer sebas-project-rail {
        display: block;
        flex: 1;
        min-height: 0;
      }
      nav.rail-drawer .rail-close {
        display: inline-flex;
        position: absolute;
        top: 10px;
        right: 10px;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        padding: 0;
        border: 1px solid var(--sebas-border, rgba(128, 138, 160, 0.35));
        border-radius: var(--sebas-radius-full, 10px);
        background: var(--sebas-surface, #fff);
        color: var(--sebas-text);
        cursor: pointer;
      }
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
    // （4.1）聚焦会话驱动项目上下文：dashboard 在聚焦变化时投影所属项目
    // 路径，shell 更新 selectedPath（项目行点击的 rail-select 语义保持独立）。
    window.addEventListener(PROJECT_FOLLOW_EVENT, this.onProjectFollow)
    // 5.1：窄屏翻转 → 分割线禁拖（布局退化由 CSS 媒体查询承接）。
    this.unlistenNarrow = onNarrowChange((n) => (this.narrow = n))
    // add-webui-tiered-notices 3.2：WS 连接状态 → 通知层的持续 warn 驻留
    // 横幅（重连即消）；connected=true 时顺带发起可达性 get（onWsState）。
    window.addEventListener('sebas:ws-state', this.onWsState)
    // fix-webui-mobile-polish：手机抽屉里点了会话（rail-focus）即收起抽屉。
    window.addEventListener(RAIL_FOCUS_EVENT, this.onRailFocusCloseDrawer)
    // add-core-reachability-ws-push D4：订阅 core.reachability 翻转推送
    // （shell 常驻，订阅不漏帧；初始态由连接建立时的 get 补齐）。
    this.unsubscribeWs = sharedWs.subscribe(this.onCoreReachabilityEvent)
    // fix-pending-queue-liveness 2.2：停滞回合被看门狗强制收尾 → warn 分级
    // 通知就地呈现（点名会话与释放的搁浅条目数）。shell 常驻订阅，会话无
    // 论是否聚焦都可见。
    this.unsubscribeStall = sharedWs.subscribe(this.onSessionTurnStalled)
    // 会话过期 / 中途启用鉴权：任何 API 401 都把界面切回登录页。首启设置
    // 页态除外——零用户时登录门永不可过（没有凭据能试）。
    setUnauthorizedHandler(() => {
      if (this.authState !== 'setup') this.showLogin()
    })
    this.onNavigate()
    void this.checkAuth()
  }

  /**
   * 翻转推送 / get 响应的统一入账点（add-webui-tiered-notices 2.4）：
   * 状态落 @state 之外，还把 fatal 槽位同步进通知层——`ok=false` 进 fatal
   * （横幅 + 锁定）；从 fatal 恢复时清槽位并弹「核心已恢复」info。未知态
   * （调用方跳过）与重复 false 推送都幂等。
   */
  private applyCoreReachability(next: CoreReachabilityState): void {
    const hadFatal = this.coreReachability?.ok === false
    this.coreReachability = next
    if (next.ok === false) {
      setFatal({ kind: next.kind, cause: next.cause })
    } else if (hadFatal) {
      setFatal(null)
      notify({ level: 'info', message: '核心已恢复' })
    }
  }

  /** 翻转推送 → 结构化状态（D4：沿用/扩展 coreUnreachableCause 为 ok/kind/cause）。 */
  private onCoreReachabilityEvent = (ev: { type: string }): void => {
    if (ev.type !== 'core.reachability') return
    const { ok, kind, cause } = ev as { type: 'core.reachability'; ok: boolean; kind?: CoreReachabilityState['kind']; cause?: string }
    this.applyCoreReachability({ ok, ...(kind ? { kind } : {}), ...(cause ? { cause } : {}) })
  }

  private unsubscribeStall?: () => void

  /**
   * fix-pending-queue-liveness 2.2：看门狗强制收尾通知（warn 低档）。文案
   * 点名会话与释放的搁浅条目数；dedupeKey 按会话隔离，8s 去重窗内同一会话
   * 的重复帧不刷屏。
   */
  private onSessionTurnStalled = (ev: { type: string }): void => {
    if (ev.type !== 'session.turn_stalled') return
    const { session_id, released } = ev as {
      type: 'session.turn_stalled'
      session_id: string
      released: number
    }
    // encoded key 的尾段是会话引用（`channel%00reference`）——呈现用人读词，
    // 不展示编码形态。
    const label = decodeURIComponent(session_id.split('%00').pop() ?? session_id)
    const releasedText = released > 0 ? `，${released} 条待执行提交已解除卡死` : ''
    notify({
      level: 'warn',
      message: `会话「${label}」的回合长时间无任何事件，已被强制收尾${releasedText}。`,
      dedupeKey: `session.turn_stalled:${session_id}`,
    })
  }

  /**
   * add-core-reachability-ws-push D3：`/ws` 连接建立/重连即 get 当前态——
   * 初始态与断线窗口丢失翻转的收敛由同一动作覆盖（重连后 get 的响应即
   * 真实状态），不靠客户端记账。get 失败（断线竞态等）不推翻既有状态。
   */
  private async refreshCoreReachability(): Promise<void> {
    try {
      const payload = (await sharedWs.request('core.reachability.get')) as {
        ok?: boolean
        kind?: CoreReachabilityState['kind']
        cause?: string
      }
      this.applyCoreReachability({
        ok: payload?.ok !== false,
        ...(payload?.kind ? { kind: payload.kind } : {}),
        ...(payload?.cause ? { cause: payload.cause } : {}),
      })
    } catch {
      // get 本身失败：保留当前状态，重连后的下一次 get 收敛。
    }
  }

  private onWsState = (e: Event): void => {
    const connected = (e as CustomEvent<{ connected: boolean }>).detail?.connected !== false
    // fix-webui-mobile-polish：登录/首启设置态下的 /ws 认证拒绝循环（连接即
    // 被升级前 401 关闭、退避一路爬升）是未登录的**预期形状**，不是「服务
    // 器断开」——横幅只在进入工作台后才有意义；就绪瞬间 reconnectNow 兜住
    // 退避尾巴，横幅不再在工作台上驻留。
    if (this.authState !== 'ready') return
    setWsDown(!connected)
    if (connected) void this.refreshCoreReachability()
  }

  /**
   * 进入工作台的统一出口：翻转鉴权态 + 立即重连 /ws（fix-webui-mobile-polish，
   * 见 onWsState 注释）。
   */
  private markAuthReady(): void {
    this.authState = 'ready'
    sharedWs.reconnectNow()
  }

  /** 窄屏项目抽屉开关（fix-webui-mobile-polish）：≤640px 时项目树收进抽屉。 */
  @state() private railDrawerOpen = false

  /** 手机抽屉：会话聚焦（rail 点了会话）即收起，把屏幕还给工作台。 */
  private onRailFocusCloseDrawer = (): void => {
    if (this.railDrawerOpen) this.railDrawerOpen = false
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
      this.markAuthReady()
    } catch {
      if (this.authState === 'checking') {
        // /api/auth/me 本身失败（网络/服务异常）：按未启用处理，后续请求的
        // 401 会经 setUnauthorizedHandler 再切回登录页。
        this.markAuthReady()
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
    this.markAuthReady()
    void this.checkAuth()
  }

  /** 首启设置成功（root 已建、会话已立）：与登录同一放行路径。 */
  private onSetupSuccess = (e: Event): void => {
    this.authUsername = (e as CustomEvent<{ username: string | null }>).detail?.username ?? null
    this.markAuthReady()
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
    window.removeEventListener(PROJECT_FOLLOW_EVENT, this.onProjectFollow)
    this.unlistenNarrow?.()
    window.removeEventListener('sebas:ws-state', this.onWsState)
    window.removeEventListener(RAIL_FOCUS_EVENT, this.onRailFocusCloseDrawer)
    this.unsubscribeWs?.()
    this.unsubscribeWs = null
    this.unsubscribeStall?.()
    this.unsubscribeStall = undefined
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
    // 切项目 = 离开归档只读视图（归档视图只由恢复/关闭动作退出自身）。
    this.archivedEntry = null
    if (location.pathname !== '/') navigate('/')
  }

  /**
   * （4.1，design D4）聚焦会话反投影：聚焦会话所属项目写成 selectedPath
   * （未命中时 dashboard 不派发）。与 rail-select 收敛到同一状态源——主区
   * 标题/项目上下文随聚焦即时跟随，项目行点击的独立选择不受影响。
   */
  private onProjectFollow = (e: Event): void => {
    const path = (e as CustomEvent<{ path: string | null }>).detail.path
    if (!path) return
    this.selectedPath = path
  }

  /**
   * （2.1）rail History 条目点击上报的只读归档条目：接力给 dashboard。
   * 点击本身绝不触发 restore（误触陷阱拆除后的语义）。
   */
  private onArchiveView = (e: Event): void => {
    this.archivedEntry = (e as CustomEvent<ArchiveEntry>).detail
    if (location.pathname !== '/') navigate('/')
  }

  /** 归档视图关闭（恢复成功 / 显式退出）：清空只读态。 */
  private onArchiveViewClose = (): void => {
    this.archivedEntry = null
  }

  private renderOutlet() {
    switch (this.routeId) {
      case 'dashboard':
        return html`<sebas-dashboard
          .selectedPath=${this.selectedPath}
          .coreReachability=${this.coreReachability}
          .archivedEntry=${this.archivedEntry}
          @archive-view-close=${this.onArchiveViewClose}
        ></sebas-dashboard>`
      case 'sessions':
        return html`<sebas-sessions></sebas-sessions>`
      case 'session-deep-link':
        // 深链渲染同一个工作台并聚焦该会话（读 detail 即设置服务端焦点
        // 指针）——没有独立详情页。
        return html`<sebas-dashboard
          .selectedPath=${this.selectedPath}
          .coreReachability=${this.coreReachability}
          .deepLinkKey=${this.params['key'] ?? null}
          .archivedEntry=${this.archivedEntry}
          @archive-view-close=${this.onArchiveViewClose}
        ></sebas-dashboard>`
      default:
        return html`<sebas-dashboard
          .selectedPath=${this.selectedPath}
          .coreReachability=${this.coreReachability}
          .archivedEntry=${this.archivedEntry}
          @archive-view-close=${this.onArchiveViewClose}
        ></sebas-dashboard>`
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
      ${this.narrow && !this.railDrawerOpen
        ? html`<button
            class="rail-toggle"
            aria-label="打开项目树"
            title="项目树"
            @click=${() => (this.railDrawerOpen = true)}
          >
            ${icon('menu', 20)}
          </button>`
        : nothing}
      ${this.narrow && this.railDrawerOpen
        ? html`<button
            class="nav-scrim"
            aria-label="关闭项目树"
            @click=${() => (this.railDrawerOpen = false)}
          ></button>`
        : nothing}
      <wa-split-panel
        class="frame"
        orientation="horizontal"
        primary="start"
        position-in-pixels=${this.railWidth}
        ?disabled=${this.narrow}
        ?inert=${this.coreReachability?.ok === false}
        @wa-reposition=${this.onRailReposition}
      >
        <nav slot="start" class="rail-drawer ${this.railDrawerOpen && this.narrow ? 'open' : ''}" aria-label="Primary">
          <button
            class="rail-close"
            aria-label="关闭项目树"
            @click=${() => (this.railDrawerOpen = false)}
          >
            ${icon('x', 16)}
          </button>
          <a class="brand" href="/" aria-label="sebas console home">
            <span class="mark" aria-hidden="true">❯</span>
            <span class="name">sebas<small>${APP_TAGLINE}</small></span>
          </a>
          <sebas-project-rail
            .activePath=${this.selectedPath}
            @rail-select=${this.onRailSelect}
            @rail-archive-view=${this.onArchiveView}
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
          <div class="outlet${this.isWideRoute() ? '' : ' padded'}">${this.renderOutlet()}</div>
        </main>
      </wa-split-panel>
      <sebas-settings-modal
        .role=${this.authRole}
        ?open=${this.settingsOpen}
        @close=${() => (this.settingsOpen = false)}
      ></sebas-settings-modal>
      <sebas-notice-layer></sebas-notice-layer>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-app': SebasApp
  }
}
