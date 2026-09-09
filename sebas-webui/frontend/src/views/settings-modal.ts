/**
 * Settings modal (IA v2)：侧栏底部 Settings 入口打开的居中弹窗，对齐预览
 * 原型 preview-app.ts 的 settings-dialog 布局——暗色面板、左侧分区导航、
 * 右侧内容区、右上关闭按钮。分区由 `section` 属性驱动
 * （fix-settings-menu-and-services-semantics：缺省首项 `settings`，顺序
 * settings → services → models → appearance → env → about）：
 *
 *   - settings   → 弹窗壳/总览：工作区根目录、default agent kind、
 *                  default provider/model 三个只读项 + 「全部进程重启」
 *                  「重置 Settings」两个高危动作（wa-dialog 二次确认）
 *   - services   → watchdog 受管子进程（GET /api/admin/services：name /
 *                  desired / actual / uptime + /api/admin/events 最近错误；
 *                  enable/disable/restart 动作；无 adapter 时诚实呈现
 *                  「无 watchdog 控制面」横幅且不渲染动作按钮）
 *   - models     → provider 路由网关总览（/api/router 的 listen / debug /
 *                  auth）+ provider 管理列表
 *   - appearance → 主题三态（system / dark / light；切换与持久化在 theme.ts）
 *   - env        → 环境变量名清单（后端无 env 端点，值一律如实标注
 *                  "managed by core config"，绝不编造）
 *   - about      → /api/about 的真实数据（version / rustc / uptime /
 *                  router listen / provider count）
 *
 * 上次停留分区记忆在 localStorage `lastSettingsSection`（非法值回退
 * `settings`）。关闭交互：关闭按钮 / Esc / 点击遮罩 → `open` 置 false 并
 * 冒泡 `close` 事件，宿主（app-shell）据此同步状态。
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import {
  api,
  type About,
  type AdminEvent,
  type AdminService,
  type RouterInfo,
  type RouterProviderAdmin,
  type ProviderPreset,
  type ProviderPayload,
  type AgentDefaults,
  ApiError,
} from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import { getThemeMode, resolvesToLight, setThemeMode, type ThemeMode } from '../theme.js'

// Web Awesome 组件（provider 管理对话框用；与其它 view 同款按需注册）。
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

/**
 * 设置弹窗分区。`settings` 是弹窗壳/总览（含高危动作入口）；Services 以
 * watchdog 受管子进程为唯一数据源（/api/admin/services），/api/router 的
 * listen/debug/auth 归 Models 顶部的「Router 路由网关」总览卡——两个语义
 * 彻底解耦（fix-settings-menu-and-services-semantics D2）。
 */
export type SettingsSection =
  | 'settings'
  | 'services'
  | 'models'
  | 'appearance'
  | 'env'
  | 'about'

/** 上次停留分区的 localStorage 键（D5：单键、仅本地、无服务端同步）。 */
const LAST_SECTION_KEY = 'lastSettingsSection'

/**
 * 受管服务的对外显示名（D7：内部字符串保持 ServiceName 枚举一致的小写
 * 原名——`service_from_str("feishu")` 是 None——只在显示层做 i18n 映射）。
 */
const SERVICE_DISPLAY_NAME: Record<string, string> = {
  im: '飞书 IM',
}

/** 分区导航的静态元数据（icon 名见 components/icons.ts）。顺序即规约。 */
const SECTIONS: ReadonlyArray<{ id: SettingsSection; label: string; icon: string }> = [
  { id: 'settings', label: 'Settings', icon: 'settings' },
  { id: 'services', label: 'Services', icon: 'shield' },
  { id: 'models', label: 'Models', icon: 'zap' },
  { id: 'appearance', label: 'Appearance', icon: 'sun' },
  { id: 'env', label: 'Environment', icon: 'inbox' },
  { id: 'about', label: 'About', icon: 'about' },
]

const SECTION_DESC: Record<SettingsSection, string> = {
  settings: 'Workspace overview and maintenance actions.',
  services: 'Background services that run alongside sebas.',
  models: 'Manage model providers. Preset-derived values follow the app code; you own the API key.',
  appearance: 'How the console looks. Your choice is saved in this browser.',
  env: 'Environment variables the sebas processes read at startup. The API does not expose values.',
  about: 'Runtime build information.',
}

/** 读取上次停留分区；缺值/非法值一律回退 null（调用方保持缺省 settings）。 */
function readLastSection(): SettingsSection | null {
  try {
    const raw = localStorage.getItem(LAST_SECTION_KEY)
    return SECTIONS.some((s) => s.id === raw) ? (raw as SettingsSection) : null
  } catch {
    return null
  }
}

function writeLastSection(id: SettingsSection): void {
  try {
    localStorage.setItem(LAST_SECTION_KEY, id)
  } catch {
    // localStorage 不可用（隐私模式等）则静默跳过——记忆是锦上添花。
  }
}

/** 秒数 → 紧凑时长（watchdog 的 uptime_secs；null = 尚未拉起过）。 */
function formatUptimeSecs(secs: number | null): string {
  if (secs === null) return '—'
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (d > 0) return `${d}d ${h}h ${m}m`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}

/** Appearance 分区的主题三态（mode 语义见 theme.ts）。 */
const THEME_OPTIONS: ReadonlyArray<{ mode: ThemeMode; label: string; sub: string }> = [
  { mode: 'system', label: 'System', sub: 'Follow your OS preference' },
  { mode: 'dark', label: 'Dark', sub: 'Always dark' },
  { mode: 'light', label: 'Light', sub: 'Always light' },
]

/**
 * sebas 工作区实际读取的环境变量（grep 自 sebas-router / sebas-router /
 * sebas-acp / sebas-webui / core config）。后端没有任何 env 端点，所以
 * 这里只列名字与用途，值一列如实写 "managed by core config"。
 */
const ENV_VARS: ReadonlyArray<{ name: string; what: string }> = [
  { name: 'SEBAS_ROUTER_CONFIG', what: 'Router config file path' },
  { name: 'SEBAS_ROUTER_LISTEN', what: 'Router listen address override' },
  { name: 'SEBAS_ROUTER_PROVIDER_OVERLAY', what: 'Provider overlay file' },
  { name: 'SEBAS_STATE_FILE', what: 'Session state store path' },
  { name: 'SEBAS_WEBUI_PASSWORD', what: 'WebUI bootstrap password (used when credentials file is missing)' },
  { name: 'SEBAS_WEBUI_TOKEN', what: 'WebUI single-field login token (token or password accepted at login)' },
  { name: 'SEBAS_CONTROL_SECRET', what: 'Router control-plane secret' },
  { name: 'SEBAS_LOG_LEVEL', what: 'Core log filter' },
  { name: 'SEBAS_HANG_TIMEOUT_SECS', what: 'Agent driver hang timeout (seconds)' },
  { name: 'SEBAS_FEISHU_APP_ID', what: 'Feishu app id' },
  { name: 'SEBAS_FEISHU_APP_SECRET', what: 'Feishu app secret' },
]

@customElement('sebas-settings-modal')
export class SebasSettingsModal extends LitElement {
  /** Open state; reflected so `?open=${…}` bindings and DOM stay in sync. */
  @property({ type: Boolean, reflect: true })
  open = false

  /** 当前分区；缺省 settings（弹窗壳/总览），re-open 时按记忆恢复。 */
  @property({ type: String })
  section: SettingsSection = 'settings'

  /** /api/about 响应（About 分区）；懒加载，切到该分区时拉取。 */
  @state() private aboutData: About | null = null
  @state() private aboutError = ''
  @state() private aboutLoading = false
  /** /api/router 响应（Models 分区的「Router 路由网关」总览卡）；懒加载。 */
  @state() private router: RouterInfo | null = null
  @state() private routerError = ''
  /**
   * watchdog 受管子进程面（Services 分区，/api/admin/services）。null =
   * 尚未加载；`adapterOk` 为 false 时（无 watchdog 控制面）services 为
   * 空表且动作按钮不渲染。
   */
  @state() private services: AdminService[] | null = null
  @state() private adapterOk: boolean | null = null
  @state() private servicesError = ''
  /** /api/admin/events 的最近错误（kind 含 error/fail）；无则不渲染。 */
  @state() private serviceEvents: AdminEvent[] = []
  /** Services 行动作的内联结果（success/error 各一种呈现）。 */
  @state() private serviceAction: { ok: boolean; text: string } | null = null
  @state() private serviceBusy: string | null = null
  /** 行动作二次确认目标（disable/restart；null = 关闭）。 */
  @state() private confirmTarget: { kind: 'disable' | 'restart'; name: string } | null = null
  /** 高危动作（Settings 分区）确认与结果状态。 */
  @state() private restartAllOpen = false
  @state() private resetSettingsOpen = false
  @state() private settingsActionResult: { ok: boolean; text: string } | null = null
  @state() private settingsBusy = false
  /** Settings 总览：工作区根目录（/api/fs/browse-dirs 的服务端解析根）。 */
  @state() private overviewRoot: string | null = null
  @state() private rootCopied = false
  /** provider 管理面（/router/api/providers + /router/api/presets）。 */
  @state() private adminProviders: RouterProviderAdmin[] | null = null
  @state() private adminError = ''
  @state() private presets: ProviderPreset[] | null = null
  /** 编辑器对话框状态：mode 决定字段集；null = 关闭。 */
  @state() private editor: {
    mode: 'create-preset' | 'create-custom' | 'edit'
    name: string
    preset: string
    baseUrlAnthropic: string
    baseUrlOpenaiChat: string
    baseUrlOpenaiResponses: string
    apiKey: string
    apiKeyEnv: string
    defaultModel: string
    protocol: string
  } | null = null
  @state() private deleteTarget: string | null = null
  /** 当前默认 provider/model（add-agent-defaults-catalog；null = 未设置）。 */
  @state() private defaults: AgentDefaults | null = null
  /** 设默认对话框的草稿：目标 provider + 可选 model（null = provider 默认）。 */
  @state() private defaultDraft: { provider: string; model: string | null } | null = null
  @state() private busy = false
  @state() private actionError = ''
  @state() private probeResult: { name: string; models: string[]; note: string } | null = null
  /** 当前主题三态（Appearance 分区）；初值来自 localStorage（theme.ts）。 */
  @state() private themeMode: ThemeMode = getThemeMode()

  // viewStyles 提供 .panel / .callout / .skel 骨架；本组件的 :host{display:
  // contents} 等声明排在其后，同特异性下优先生效。
  static styles = [
    viewStyles,
    css`
    :host {
      display: contents;
    }
    .overlay {
      position: fixed;
      inset: 0;
      z-index: 100;
      display: grid;
      place-items: center;
      padding: var(--sebas-space-4);
      background: rgba(2, 6, 23, 0.62);
      backdrop-filter: blur(2px);
    }
    .panel {
      position: relative;
      width: min(760px, 100%);
      height: min(80vh, 640px);
      display: flex;
      flex-direction: column;
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      box-shadow: var(--sebas-shadow-2);
      color: var(--sebas-text);
      overflow: hidden;
    }
    .close {
      position: absolute;
      top: var(--sebas-space-3);
      right: var(--sebas-space-3);
      z-index: 10;
      width: 28px;
      height: 28px;
      display: grid;
      place-items: center;
      border: none;
      border-radius: var(--sebas-radius-md);
      background: none;
      color: var(--sebas-text-dim);
      font-size: 0.9rem;
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .close:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .close:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    /* 预览原型同款左右布局：左 130px 分区导航，右内容区自滚动。 */
    .layout {
      flex: 1;
      display: flex;
      min-height: 0;
    }
    .nav {
      width: 132px;
      flex: 0 0 auto;
      background: var(--sebas-surface-2);
      border-right: 1px solid var(--sebas-border);
      padding: var(--sebas-space-4) 0;
      display: flex;
      flex-direction: column;
      gap: 0;
      overflow-y: auto;
    }
    .nav .nav-item {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 6px 12px;
      font-size: 0.78rem;
      font-weight: 500;
      font-family: inherit;
      color: var(--sebas-text-dim);
      cursor: pointer;
      border: none;
      background: none;
      text-align: left;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .nav .nav-item:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    .nav .nav-item[aria-current='true'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .nav .nav-item svg {
      opacity: 0.7;
      flex: 0 0 auto;
    }
    .nav .nav-item[aria-current='true'] svg {
      opacity: 1;
    }
    .nav .nav-item:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: -2px;
    }
    .content {
      flex: 1;
      min-width: 0;
      padding: var(--sebas-space-5) var(--sebas-space-6);
      overflow-y: auto;
    }
    .content h2 {
      margin: 0 0 var(--sebas-space-1);
      font-size: 1rem;
      font-weight: 700;
      color: var(--sebas-text-bright);
    }
    .content .desc {
      font-size: 0.8rem;
      color: var(--sebas-text-dim);
      margin: 0 0 var(--sebas-space-4);
    }
    /* Models 分区：provider 列表（对齐预览原型 .provider-list）。 */
    .provider-list {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
    }
    .provider-row {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      transition: border-color var(--sebas-dur) var(--sebas-ease);
    }
    .provider-row:hover {
      border-color: var(--sebas-accent-border);
    }
    .provider-row-empty {
      padding: var(--sebas-space-4) var(--sebas-space-3);
      color: var(--sebas-text-faint);
      font-size: 0.85rem;
    }
    .provider-row-name {
      font-weight: 600;
      font-size: 0.88rem;
      color: var(--sebas-text-bright);
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .provider-row-url {
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
      font-family: var(--sebas-font-mono);
      margin-left: auto;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      max-width: 55%;
    }
    /* Models 分区管理面：工具条 / 徽标 / 行内动作 / 编辑器。 */
    .provider-toolbar {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-2);
      margin-bottom: var(--sebas-space-3);
    }
    .toolbar-error {
      font-size: 0.78rem;
      color: var(--sebas-status-failed, #f87171);
      margin-left: auto;
      overflow-wrap: anywhere;
    }
    .provider-badge {
      flex: 0 0 auto;
      padding: 1px 8px;
      border-radius: var(--sebas-radius-full);
      font-size: 0.68rem;
      font-weight: 600;
      border: 1px solid var(--sebas-border);
      color: var(--sebas-text-dim);
    }
    .provider-badge.default {
      background: color-mix(in srgb, var(--sebas-accent) 18%, transparent);
      color: var(--sebas-accent);
      border-color: var(--sebas-accent-border);
    }
    .provider-badge.preset {
      background: var(--sebas-accent-soft);
      border-color: var(--sebas-accent-border);
      color: var(--sebas-accent);
    }
    .provider-key {
      flex: 0 0 auto;
      font-size: 0.68rem;
      color: var(--sebas-text-faint);
    }
    .provider-key.on {
      color: var(--sebas-status-done);
    }
    .provider-row-actions {
      flex: 0 0 auto;
      display: flex;
      gap: 4px;
    }
    .row-action {
      width: 26px;
      height: 26px;
      display: grid;
      place-items: center;
      border: none;
      border-radius: var(--sebas-radius-md);
      background: none;
      color: var(--sebas-text-dim);
      cursor: pointer;
      font-size: 0.8rem;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .row-action:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    .row-action.danger:hover {
      color: var(--sebas-status-failed, #f87171);
    }
    .row-action:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
    }
    .probe-note {
      margin-top: 4px;
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
    }
    /* 编辑器对话框内的表单栅格。 */
    .editor-grid {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3);
      min-width: 420px;
    }
    .readonly-urls {
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      font-size: 0.75rem;
    }
    .readonly-urls .readonly-title {
      font-weight: 600;
      color: var(--sebas-text-dim);
      margin-bottom: 4px;
    }
    .readonly-urls .readonly-row {
      display: flex;
      justify-content: space-between;
      gap: var(--sebas-space-3);
      padding: 1px 0;
    }
    .readonly-urls .readonly-row code {
      font-family: var(--sebas-font-mono);
      color: var(--sebas-text-bright);
      overflow-wrap: anywhere;
    }
    .dialog-text {
      margin: 0;
      font-size: 0.85rem;
      color: var(--sebas-text);
    }
    /* Services 分区：后台服务卡（对齐预览原型 .service-card）。 */
    .service-card {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      margin-bottom: var(--sebas-space-3);
    }
    .service-card .service-info {
      flex: 1;
      min-width: 0;
    }
    .service-card .service-info .service-name {
      font-weight: 600;
      font-size: 0.9rem;
      color: var(--sebas-text-bright);
    }
    .service-card .service-info .service-desc {
      font-size: 0.78rem;
      color: var(--sebas-text-dim);
      margin-top: 2px;
    }
    .service-card .service-status {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 0.78rem;
      color: var(--sebas-text-dim);
    }
    .service-card .service-status .dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
    }
    .service-card .service-status .dot.on {
      background: var(--sebas-status-done);
    }
    .service-card .service-status .dot.off {
      background: var(--sebas-text-faint);
    }
    /* Services 行：内部名（与 /api/admin/services 对账用的稳定锚点）、
     * 动作钮与最近错误列表。 */
    .service-card .service-name .service-id {
      margin-left: 6px;
      font-family: var(--sebas-font-mono);
      font-size: 0.7rem;
      font-weight: 400;
      color: var(--sebas-text-faint);
    }
    .service-card .service-actions {
      flex: 0 0 auto;
      display: flex;
      gap: 4px;
    }
    .service-sub {
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
    }
    .service-errors {
      margin-top: var(--sebas-space-2);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      font-size: 0.78rem;
    }
    .service-errors-title {
      font-weight: 600;
      color: var(--sebas-status-failed, #f87171);
      margin-bottom: 4px;
    }
    .service-error-row {
      display: flex;
      gap: var(--sebas-space-2);
      padding: 1px 0;
      overflow-wrap: anywhere;
    }
    .service-error-kind {
      flex: 0 0 auto;
      font-family: var(--sebas-font-mono);
      color: var(--sebas-text-faint);
    }
    .service-error-msg {
      color: var(--sebas-text-dim);
    }
    /* Models 分区顶部的路由网关卡（从 Services 迁来的 listen/debug/auth）。 */
    .gateway-card {
      margin-bottom: var(--sebas-space-4);
    }
    /* Settings 总览的维护动作区。 */
    .danger-zone {
      margin-top: var(--sebas-space-5);
      padding-top: var(--sebas-space-3);
      border-top: 1px solid var(--sebas-border);
    }
    .danger-title {
      font-size: 0.8rem;
      font-weight: 600;
      color: var(--sebas-text-dim);
      margin-bottom: var(--sebas-space-2);
    }
    .danger-actions {
      display: flex;
      gap: var(--sebas-space-2);
    }
    .linkish {
      padding: 0;
      border: none;
      background: none;
      font: inherit;
      color: var(--sebas-accent);
      cursor: pointer;
      text-decoration: underline;
      text-underline-offset: 2px;
    }
    .linkish:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    /* Appearance 分区：主题三态选项。swatch 的颜色是刻意的硬编码——
     * 它展示的是两套调色板本身，必须不随当前主题变化。 */
    .theme-options {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
      max-width: 380px;
    }
    .theme-option {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      cursor: pointer;
      font-family: inherit;
      text-align: left;
      transition:
        border-color var(--sebas-dur) var(--sebas-ease),
        background var(--sebas-dur) var(--sebas-ease);
    }
    .theme-option:hover {
      border-color: var(--sebas-accent-border);
    }
    .theme-option[aria-pressed='true'] {
      background: var(--sebas-accent-soft);
      border-color: var(--sebas-accent-border);
    }
    .theme-option:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .theme-swatch {
      flex: 0 0 auto;
      width: 16px;
      height: 16px;
      border-radius: 50%;
      border: 1px solid var(--sebas-border-strong);
    }
    .theme-swatch.dark {
      background: #10141c;
    }
    .theme-swatch.light {
      background: #ffffff;
    }
    .theme-swatch.system {
      background: linear-gradient(90deg, #10141c 50%, #ffffff 50%);
    }
    .theme-option-text {
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    .theme-option-label {
      font-size: 0.88rem;
      font-weight: 600;
      color: var(--sebas-text-bright);
    }
    .theme-option[aria-pressed='true'] .theme-option-label {
      color: var(--sebas-accent);
    }
    .theme-option-sub {
      font-size: 0.75rem;
      color: var(--sebas-text-dim);
    }
    .theme-hint {
      margin: var(--sebas-space-3) 0 0;
      font-size: 0.78rem;
      color: var(--sebas-text-faint);
    }
    /* Env 清单：变量名 + 用途 + 固定的 "managed by core config" 值。 */
    .env-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 0.85rem;
    }
    .env-table th {
      text-align: left;
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      border-bottom: 1px solid var(--sebas-border);
    }
    .env-table td {
      padding: var(--sebas-space-2) var(--sebas-space-3);
      border-bottom: 1px solid var(--sebas-border);
      vertical-align: top;
    }
    .env-table tr:last-child td {
      border-bottom: none;
    }
    .env-table .var {
      font-family: var(--sebas-font-mono);
      font-size: 0.78rem;
      color: var(--sebas-text-bright);
      white-space: nowrap;
    }
    .env-table .what {
      color: var(--sebas-text-dim);
    }
    .env-table .value {
      font-family: var(--sebas-font-mono);
      font-size: 0.75rem;
      color: var(--sebas-text-faint);
      white-space: nowrap;
    }
    /* About 分区：/api/about 的真实字段。 */
    .about-list {
      margin: 0;
      padding: 0;
    }
    .about-list .kv {
      display: flex;
      align-items: baseline;
      justify-content: space-between;
      gap: var(--sebas-space-4);
      padding: var(--sebas-space-2) 0;
      border-bottom: 1px solid var(--sebas-border);
    }
    .about-list .kv:last-child {
      border-bottom: none;
    }
    .about-list dt {
      font-size: 0.85rem;
      color: var(--sebas-text-dim);
    }
    .about-list dd {
      margin: 0;
      font-family: var(--sebas-font-mono);
      font-size: 0.82rem;
      color: var(--sebas-text-bright);
      overflow-wrap: anywhere;
      text-align: right;
    }
    .version-chip {
      display: inline-block;
      padding: 1px 9px;
      border-radius: var(--sebas-radius-full);
      background: var(--sebas-accent-soft);
      border: 1px solid var(--sebas-accent-border);
      color: var(--sebas-accent);
      font-size: 0.78rem;
      font-weight: 600;
    }
    .sr-only {
      position: absolute;
      width: 1px;
      height: 1px;
      padding: 0;
      margin: -1px;
      overflow: hidden;
      clip: rect(0, 0, 0, 0);
      white-space: nowrap;
      border: 0;
    }
  `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    window.addEventListener('keydown', this.onKeydown)
    // system 模式下 OS 切换明暗时：wa-dark class 由 main.ts 的监听处理，
    // 这里只需让 Appearance 分区的提示文案保持真实。
    if (typeof window.matchMedia === 'function') {
      window
        .matchMedia('(prefers-color-scheme: light)')
        .addEventListener('change', this.onSchemeChange)
    }
  }

  disconnectedCallback(): void {
    window.removeEventListener('keydown', this.onKeydown)
    if (typeof window.matchMedia === 'function') {
      window
        .matchMedia('(prefers-color-scheme: light)')
        .removeEventListener('change', this.onSchemeChange)
    }
    super.disconnectedCallback()
  }

  private onSchemeChange = (): void => {
    this.requestUpdate()
  }

  protected willUpdate(changed: PropertyValues): void {
    // About 分区懒加载：切到 about 时拉一次（失败可重试——下次切换再取）。
    if (changed.has('section') && this.section === 'about') this.loadAbout()
    // Models 分区的 router 总览卡数据，懒加载一次。
    if (changed.has('section') && this.section === 'models') {
      this.loadGateway()
    }
    // provider 管理面：每次切到 models 都刷新（增删改后重新进入也新鲜）。
    if (changed.has('section') && this.section === 'models') this.loadProviders()
    // Services 分区：受管子进程 + 最近错误（每次切入都刷新，动作后重取）。
    if (changed.has('section') && this.section === 'services') this.loadServices()
    // Settings 总览：工作区根目录 / defaults / adapter 探测。
    if (changed.has('section') && this.section === 'settings') this.loadOverview()
  }

  /**
   * 分区记忆（D5）：打开时按 localStorage 恢复上次分区（非法值回退缺省
   * `settings`）；打开期间每次切换都写回。仅在 `open` 翻真/分区变化时
   * 触发，读写都容忍 storage 不可用。
   */
  protected updated(changed: PropertyValues): void {
    if (changed.has('open') && this.open) {
      const saved = readLastSection()
      if (saved && saved !== this.section) this.section = saved
    }
    if (changed.has('section') && this.open) writeLastSection(this.section)
  }

  private onKeydown = (e: KeyboardEvent): void => {
    if (this.open && e.key === 'Escape') this.requestClose()
  }

  /** 统一出口：先翻自身 open，再通知宿主（无宿主监听时也能自行关闭）。 */
  private requestClose(): void {
    this.open = false
    this.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }))
  }

  private loadAbout(): void {
    this.aboutLoading = true
    this.aboutError = ''
    api
      .about()
      .then((d) => {
        this.aboutData = d
        this.aboutError = ''
      })
      .catch((e) => {
        this.aboutError = String(e)
      })
      .finally(() => {
        this.aboutLoading = false
      })
  }

  private loadGateway(): void {
    if (this.router || this.routerError) return // 已加载或已失败（可重试开关）
    api
      .router()
      .then((d) => {
        this.router = d.router
      })
      .catch((e) => {
        this.routerError = String(e)
      })
  }

  /**
   * Services 分区数据（fix-settings-menu-and-services-semantics 1.2）：真源
   * 是 /api/admin/services（watchdog 受管子进程），错误事件来自
   * /api/admin/events。401/403 照常上抛（登录页接管），其余失败按
   * 「无 watchdog 控制面」退化呈现。
   */
  private loadServices(): void {
    this.servicesError = ''
    api
      .adminServicesSafe()
      .then((d) => {
        this.adapterOk = d.adapter_ok
        this.services = d.services
      })
      .catch((e) => {
        this.servicesError = e instanceof ApiError ? e.message : String(e)
        this.services = []
        this.adapterOk = false
      })
    api
      .adminEventsSafe()
      .then((d) => {
        this.serviceEvents = d.events.filter((ev) => {
          const k = ev.kind.toLowerCase()
          return k.includes('error') || k.includes('fail')
        })
      })
      .catch(() => {
        this.serviceEvents = []
      })
  }

  /** Settings 总览（task 2.2）：只读项全部来自既有端点，绝不编造。 */
  private loadOverview(): void {
    api
      .adminServicesSafe()
      .then((d) => {
        this.adapterOk = d.adapter_ok
      })
      .catch(() => {
        this.adapterOk = false
      })
    api
      .agentDefaults()
      .then((d) => {
        this.defaults = d
      })
      .catch(() => {
        this.defaults = null
      })
    // 工作区根目录：browse-dirs 不带 path 时服务端回显其解析出的默认
    // 工作根（默认 agent kind 的 work_dir / cwd），这是「既有 API」里
    // 唯一诚实携带该值的端点（/api/summary 无 work-dir 字段）。
    api
      .fsBrowseDirs('')
      .then((d) => {
        this.overviewRoot = d.path
      })
      .catch(() => {
        this.overviewRoot = null
      })
  }

  // ---- Services 行动作（enable / disable / restart） ----

  /** 行动作执行：成功后重取列表；失败（含 503）内联呈现且不刷新。 */
  private async runServiceAction(kind: 'enable' | 'disable' | 'restart', name: string): Promise<void> {
    if (this.serviceBusy) return
    this.serviceBusy = name
    this.serviceAction = null
    try {
      const r =
        kind === 'enable'
          ? await api.enableService(name)
          : kind === 'disable'
            ? await api.disableService(name)
            : await api.restartService(name)
      this.serviceAction = { ok: true, text: `${name}: ${r.message || `${kind} accepted`}` }
      this.loadServices()
    } catch (err) {
      this.serviceAction = {
        ok: false,
        text: err instanceof ApiError ? err.message : String(err),
      }
    } finally {
      this.serviceBusy = null
    }
  }

  private copyRoot(): void {
    if (!this.overviewRoot) return
    // jsdom/旧浏览器可能没有 clipboard——复制失败不阻塞总览渲染。
    void navigator.clipboard?.writeText(this.overviewRoot).then(
      () => {
        this.rootCopied = true
        window.setTimeout(() => (this.rootCopied = false), 1500)
      },
      () => {},
    )
  }

  /** 高危动作一：全部进程重启（watchdog restart-core 路径）。 */
  private async restartAllProcesses(): Promise<void> {
    if (this.settingsBusy) return
    this.settingsBusy = true
    this.settingsActionResult = null
    try {
      const r = await api.adminRestart()
      this.settingsActionResult = { ok: true, text: r.message || 'restart accepted' }
    } catch (err) {
      this.settingsActionResult = {
        ok: false,
        text: err instanceof ApiError ? err.message : String(err),
      }
    } finally {
      this.settingsBusy = false
      this.restartAllOpen = false
    }
  }

  /** 高危动作二：重置 Settings（清空分区记忆并回到缺省 settings）。 */
  private resetSettings(): void {
    try {
      localStorage.removeItem(LAST_SECTION_KEY)
    } catch {
      // storage 不可用则本来就无记忆可清。
    }
    this.section = 'settings'
    this.settingsActionResult = { ok: true, text: 'Settings reset — back to defaults.' }
    this.resetSettingsOpen = false
  }

  /** provider 管理面数据：admin 列表 + 内置 preset 表（跟随代码的只读值）。 */
  private loadProviders(): void {
    this.adminError = ''
    api
      .routerProviders()
      .then((d) => {
        this.adminProviders = d.providers
      })
      .catch((e) => {
        this.adminError = e instanceof ApiError ? e.message : String(e)
        this.adminProviders = []
      })
    api
      .agentDefaults()
      .then((d) => {
        this.defaults = d
      })
      .catch(() => {
        this.defaults = null
      })
    if (this.presets === null) {
      api
        .routerPresets()
        .then((d) => {
          this.presets = d.presets
        })
        .catch(() => {
          // preset 表拉不到（router 不可达）→ 编辑器里显示空表，不阻塞列表。
          this.presets = []
        })
    }
  }

  private refreshProviders(): Promise<void> {
    return api
      .routerProviders()
      .then((d) => {
        this.adminProviders = d.providers
      })
      .catch((e) => {
        this.adminError = e instanceof ApiError ? e.message : String(e)
      })
  }

  // ---- provider 编辑器对话框 ----

  private openCreatePreset(): void {
    this.actionError = ''
    this.probeResult = null
    this.editor = {
      mode: 'create-preset',
      name: '',
      preset: this.presets?.[0]?.name ?? '',
      baseUrlAnthropic: '',
      baseUrlOpenaiChat: '',
      baseUrlOpenaiResponses: '',
      apiKey: '',
      apiKeyEnv: '',
      defaultModel: '',
      protocol: 'auto',
    }
  }

  private openCreateCustom(): void {
    this.actionError = ''
    this.probeResult = null
    this.editor = {
      mode: 'create-custom',
      name: '',
      preset: '',
      baseUrlAnthropic: '',
      baseUrlOpenaiChat: '',
      baseUrlOpenaiResponses: '',
      apiKey: '',
      apiKeyEnv: '',
      defaultModel: '',
      protocol: 'auto',
    }
  }

  private openEdit(p: RouterProviderAdmin): void {
    this.actionError = ''
    this.probeResult = null
    this.editor = {
      // preset 派生条目按 preset 语义编辑（url 只读）；自定义按 custom。
      mode: p.preset ? 'edit' : 'edit',
      name: p.name,
      preset: p.preset ?? '',
      baseUrlAnthropic: p.base_url_anthropic ?? '',
      baseUrlOpenaiChat: p.base_url_openai_chat ?? '',
      baseUrlOpenaiResponses: p.base_url_openai_responses ?? '',
      // 密钥绝不回填——空提交 = 保留旧 key（服务端语义）。
      apiKey: '',
      apiKeyEnv: p.api_key_env ?? '',
      defaultModel: '',
      protocol: 'auto',
    }
  }

  private setEditor(patch: Partial<NonNullable<SebasSettingsModal['editor']>>): void {
    if (this.editor) this.editor = { ...this.editor, ...patch }
  }

  private editorPayload(): ProviderPayload {
    const e = this.editor
    if (!e) return {}
    const payload: ProviderPayload = {
      default_model: e.defaultModel.trim() || undefined,
      protocol: e.protocol,
    }
    if (e.mode === 'create-preset' || (e.mode === 'edit' && e.preset)) {
      payload.preset = e.preset
    } else {
      payload.base_url_anthropic = e.baseUrlAnthropic.trim() || undefined
      payload.base_url_openai_chat = e.baseUrlOpenaiChat.trim() || undefined
      payload.base_url_openai_responses = e.baseUrlOpenaiResponses.trim() || undefined
      payload.api_key_env = e.apiKeyEnv.trim() || undefined
    }
    if (e.apiKey.trim()) payload.api_key = e.apiKey.trim()
    if (e.mode === 'create-preset' || e.mode === 'create-custom') {
      payload.name = e.name.trim()
    }
    return payload
  }

  private async submitEditor(): Promise<void> {
    if (!this.editor || this.busy) return
    const e = this.editor
    if ((e.mode === 'create-preset' || e.mode === 'create-custom') && !e.name.trim()) {
      this.actionError = '名称不能为空'
      return
    }
    if (
      e.mode === 'create-custom' &&
      !e.baseUrlAnthropic.trim() &&
      !e.baseUrlOpenaiChat.trim() &&
      !e.baseUrlOpenaiResponses.trim()
    ) {
      this.actionError = '自定义 provider 至少需要填写一个 Base URL'
      return
    }
    this.busy = true
    this.actionError = ''
    const payload = this.editorPayload()
    try {
      if (e.mode === 'create-preset' || e.mode === 'create-custom') {
        await api.routerProviderCreate(payload)
      } else {
        await api.routerProviderUpdate(e.name, payload)
      }
      this.editor = null
      await this.refreshProviders()
    } catch (err) {
      this.actionError = err instanceof ApiError ? err.message : String(err)
    } finally {
      this.busy = false
    }
  }

  private async confirmDelete(): Promise<void> {
    if (!this.deleteTarget || this.busy) return
    this.busy = true
    this.actionError = ''
    try {
      await api.routerProviderDelete(this.deleteTarget)
      this.deleteTarget = null
      await this.refreshProviders()
    } catch (err) {
      this.actionError = err instanceof ApiError ? err.message : String(err)
    } finally {
      this.busy = false
    }
  }

  private async probeProvider(name: string): Promise<void> {
    if (this.busy) return
    this.busy = true
    this.actionError = ''
    this.probeResult = null
    try {
      const r = await api.routerProviderProbe(name)
      this.probeResult = {
        name,
        models: r.models,
        note: r.applied
          ? 'model list saved to the provider catalog.'
          : 'preset-derived provider: the code table owns the catalog; only the display above is updated.',
      }
    } catch (err) {
      this.actionError = err instanceof ApiError ? err.message : String(err)
    } finally {
      this.busy = false
    }
  }

  private renderSectionHead(section: SettingsSection) {
    return html`
      <h2>${SECTIONS.find((s) => s.id === section)?.label ?? section}</h2>
      <p class="desc">${SECTION_DESC[section]}</p>
    `
  }

  // 分区渲染：settings 为总览壳；services 读 watchdog 受管子进程面；
  // models 顶部承载 /api/router 的路由网关总览；env/about 直渲染
  // （数据源见文件头注释）。
  private renderSection(section: SettingsSection) {
    switch (section) {
      case 'settings':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderSettings()}
        `
      case 'services':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderServices()}
        `
      case 'models':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderModels()}
        `
      case 'appearance':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderAppearance()}
        `
      case 'env':
        return html`
          ${this.renderSectionHead(section)}
          <div class="panel">
            <table class="env-table">
              <thead>
                <tr>
                  <th>Variable</th>
                  <th>Used for</th>
                  <th>Value</th>
                </tr>
              </thead>
              <tbody>
                ${ENV_VARS.map(
                  (v) => html`
                    <tr>
                      <td class="var">${v.name}</td>
                      <td class="what">${v.what}</td>
                      <td class="value">managed by core config</td>
                    </tr>
                  `,
                )}
              </tbody>
            </table>
          </div>
        `
      case 'about':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderAbout()}
        `
    }
  }

  /** Models：路由网关总览卡（/api/router 的 listen / debug / auth，task 1.3）
   *  + provider 管理页（列表 + 新增/编辑/删除/探测）。 */
  private renderModels() {
    if (this.routerError)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.routerError}</span>
        </div>
      `
    // 「Router 路由网关」总览卡（fix-settings-menu-and-services-semantics
    // D2：listen / debug / auth 从 Services 分区迁来，provider 路由事实
    // 归 Models 语义）。
    const gateway =
      this.router === null
        ? html`<div class="panel panel-pad gateway-card">
            <div class="skel-row"><div class="skel skel-line" style="width:40%"></div></div>
            <div class="skel-row"><div class="skel skel-line" style="width:60%"></div></div>
          </div>`
        : html`<div class="readonly-urls gateway-card">
            <div class="readonly-title">Router 路由网关</div>
            <div class="readonly-row"><span>Listen</span><code>${this.router.listen ?? '—'}</code></div>
            <div class="readonly-row">
              <span>Debug</span><code>${this.router.debug ? 'on' : 'off'}</code>
            </div>
            <div class="readonly-row">
              <span>Auth</span><code>${this.router.has_auth ? 'configured' : 'none'}</code>
            </div>
          </div>`
    const providers = this.adminProviders
    return html`
      ${gateway}
      <div class="provider-toolbar">
        <wa-button variant="brand" appearance="filled" @click=${() => this.openCreatePreset()}>
          ＋ New (preset)
        </wa-button>
        <wa-button appearance="outlined" @click=${() => this.openCreateCustom()}>
          ＋ New (custom)
        </wa-button>
        <span class="label" role="status">
          ${this.defaults?.provider
            ? `default: ${this.defaults.provider}${this.defaults.model ? ` / ${this.defaults.model}` : ''}`
            : 'no default set'}
        </span>
        ${this.defaults?.provider
          ? html`<button
              class="row-action"
              title="Clear the default for new sessions"
              ?disabled=${this.busy}
              @click=${() => void this.clearDefault()}
            >
              ✕
            </button>`
          : nothing}
        ${this.adminError
          ? html`<span class="toolbar-error" role="alert">${this.adminError}</span>`
          : nothing}
      </div>

      ${this.actionError
        ? html`<div class="callout callout-error" role="alert">${this.actionError}</div>`
        : nothing}
      ${this.probeResult
        ? html`
            <div class="callout" role="status">
              <strong>${this.probeResult.name}</strong>: ${this.probeResult.models.join(', ')}
              <div class="probe-note">${this.probeResult.note}</div>
            </div>
          `
        : nothing}

      ${providers === null
        ? html`
            <div class="panel panel-pad">
              <div class="skel-row"><div class="skel skel-line" style="width:60%"></div></div>
              <div class="skel-row"><div class="skel skel-line" style="width:40%"></div></div>
            </div>
          `
        : html`
            <div class="provider-list">
              ${providers.length === 0
                ? html`<div class="provider-row-empty">No providers configured.</div>`
                : providers.map((p) => this.renderProviderRow(p))}
            </div>
          `}
    `
  }

  private renderProviderRow(p: RouterProviderAdmin) {
    const url = p.base_url_anthropic ?? p.base_url_openai_chat ?? p.base_url_openai_responses
    return html`
      <div class="provider-row">
        <span class="provider-row-name">${p.name}</span>
        <span class="provider-badge ${p.preset ? 'preset' : 'custom'}">
          ${p.preset ? `${p.preset} · code` : 'custom'}
        </span>
        <span class="provider-key ${p.api_key_configured ? 'on' : 'off'}">
          ${p.api_key_configured ? 'key configured' : 'no key'}
        </span>
        ${this.defaults?.provider === p.name
          ? html`<span class="provider-badge default" role="status">default</span>`
          : nothing}
        <span class="provider-row-url" title=${url ?? ''}>${url ?? 'no base url'}</span>
        <span class="provider-row-actions">
          <button
            class="row-action"
            title="Set as default for new sessions"
            ?disabled=${this.busy}
            @click=${() => this.openSetDefault(p)}
          >
            ★
          </button>
          ${p.base_url_openai_chat || p.base_url_openai_responses || p.preset
            ? html`
                <button
                  class="row-action"
                  title="Probe model list"
                  ?disabled=${this.busy}
                  @click=${() => void this.probeProvider(p.name)}
                >
                  🔍
                </button>
              `
            : nothing}
          <button class="row-action" title="Edit" @click=${() => this.openEdit(p)}>✎</button>
          <button
            class="row-action danger"
            title="Delete"
            ?disabled=${this.busy}
            @click=${() => (this.deleteTarget = p.name)}
          >
            🗑
          </button>
        </span>
      </div>
    `
  }

  /**
   * Settings 总览壳（task 2.2/2.3）：三个只读项 + 两个高危动作。只读项的
   * 数据全部来自既有端点，缺值如实显示 '—'；高危动作在无 watchdog 控制
   * 面时 disabled + tooltip（spec「高危动作二次确认」）。
   */
  private renderSettings() {
    const adapterOk = this.adapterOk === true
    const adapterKnown = this.adapterOk !== null
    return html`
      <dl class="about-list">
        <div class="kv">
          <dt>Workspace root</dt>
          <dd>
            ${this.overviewRoot ?? '—'}
            ${this.overviewRoot
              ? html`<button
                  class="row-action"
                  title="Copy workspace root"
                  @click=${() => this.copyRoot()}
                >
                  ${this.rootCopied ? '✓' : '⧉'}
                </button>`
              : nothing}
          </dd>
        </div>
        <div class="kv">
          <dt>Default agent kind</dt>
          <dd>acp <span class="service-sub">(default kind for new sessions)</span></dd>
        </div>
        <div class="kv">
          <dt>Default provider / model</dt>
          <dd>
            ${this.defaults?.provider
              ? html`<button
                  class="linkish"
                  title="Open the Models section"
                  @click=${() => (this.section = 'models')}
                >
                  ${this.defaults.provider}${this.defaults.model ? ` / ${this.defaults.model}` : ''}
                </button>`
              : '— (set one in Models)'}
          </dd>
        </div>
      </dl>

      ${this.settingsActionResult
        ? html`<div
            class="callout ${this.settingsActionResult.ok ? '' : 'callout-error'}"
            role=${this.settingsActionResult.ok ? 'status' : 'alert'}
          >
            ${this.settingsActionResult.text}
          </div>`
        : nothing}

      <div class="danger-zone">
        <div class="danger-title">Maintenance</div>
        <div class="danger-actions">
          <wa-button
            variant="danger"
            appearance="outlined"
            ?disabled=${this.settingsBusy || (adapterKnown && !adapterOk)}
            title=${adapterKnown && !adapterOk ? '无 watchdog 控制面' : 'Restart every managed service'}
            @click=${() => {
              this.settingsActionResult = null
              this.restartAllOpen = true
            }}
          >
            全部进程重启
          </wa-button>
          <wa-button
            appearance="outlined"
            ?disabled=${this.settingsBusy}
            title="Clear the remembered settings section in this browser"
            @click=${() => {
              this.settingsActionResult = null
              this.resetSettingsOpen = true
            }}
          >
            重置 Settings
          </wa-button>
        </div>
      </div>
    `
  }

  /**
   * Services：watchdog 受管子进程面（task 1.2）。唯一数据源是
   * /api/admin/services；无 adapter（裸 core）时显示「无 watchdog 控制面」
   * 横幅、列表为空、enable/disable/restart 按钮不渲染（spec 退化 scenario）。
   */
  private renderServices() {
    if (this.servicesError)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.servicesError}</span>
        </div>
      `
    if (this.services === null)
      return html`
        <div class="panel panel-pad">
          ${[0, 1].map(
            () => html`
              <div class="skel-row">
                <div class="skel skel-line" style="width:30%"></div>
                <div class="skel skel-line" style="width:50%"></div>
              </div>
            `,
          )}
        </div>
      `
    if (this.adapterOk !== true)
      return html`
        <div class="callout services-banner" role="status">
          无 watchdog 控制面 — 受管服务列表不可用。启用请运行 <code>sebas run</code>（watchdog
          形态）后再打开此页。
        </div>
      `
    return html`
      ${this.serviceAction
        ? html`<div
            class="callout ${this.serviceAction.ok ? '' : 'callout-error'}"
            role=${this.serviceAction.ok ? 'status' : 'alert'}
          >
            ${this.serviceAction.text}
          </div>`
        : nothing}
      ${this.services.map((s) => this.renderServiceRow(s))}
      ${this.serviceEvents.length > 0
        ? html`
            <div class="service-errors">
              <div class="service-errors-title">Recent errors</div>
              ${this.serviceEvents
                .slice(-3)
                .reverse()
                .map(
                  (ev) => html`
                    <div class="service-error-row">
                      <span class="service-error-kind">${ev.kind}</span>
                      <span class="service-error-msg">${ev.message}</span>
                    </div>
                  `,
                )}
            </div>
          `
        : nothing}
    `
  }

  /** 单个受管服务行：name（含 im→飞书 IM 映射）/ desired / actual / uptime。 */
  private renderServiceRow(s: AdminService) {
    const running = s.status === 'running'
    return html`
      <div class="service-card">
        <div class="service-info">
          <div class="service-name">
            ${SERVICE_DISPLAY_NAME[s.name] ?? s.name}
            <span class="service-id">${s.name}</span>
          </div>
          <div class="service-desc">
            desired ${s.desired} · status ${s.status} · up ${formatUptimeSecs(s.uptime_secs)}
          </div>
        </div>
        <div class="service-status">
          <span class="dot ${running ? 'on' : 'off'}"></span>${s.status}
        </div>
        <div class="service-actions">
          <button
            class="row-action"
            title="Enable service"
            ?disabled=${this.serviceBusy !== null}
            @click=${() => void this.runServiceAction('enable', s.name)}
          >
            ▶
          </button>
          <button
            class="row-action"
            title="Disable service"
            ?disabled=${this.serviceBusy !== null}
            @click=${() => (this.confirmTarget = { kind: 'disable', name: s.name })}
          >
            ■
          </button>
          <button
            class="row-action"
            title="Restart service"
            ?disabled=${this.serviceBusy !== null}
            @click=${() => (this.confirmTarget = { kind: 'restart', name: s.name })}
          >
            ⟳
          </button>
        </div>
      </div>
    `
  }

  /** Appearance：主题三态；选择立即生效（翻 <html> 的 wa-dark）并持久化。 */
  private renderAppearance() {
    return html`
      <div class="theme-options" role="group" aria-label="Theme">
        ${THEME_OPTIONS.map(
          (o) => html`
            <button
              class="theme-option"
              aria-pressed=${this.themeMode === o.mode ? 'true' : 'false'}
              @click=${() => {
                setThemeMode(o.mode)
                this.themeMode = o.mode
              }}
            >
              <span class="theme-swatch ${o.mode}"></span>
              <span class="theme-option-text">
                <span class="theme-option-label">${o.label}</span>
                <span class="theme-option-sub">${o.sub}</span>
              </span>
            </button>
          `,
        )}
      </div>
      <p class="theme-hint">
        ${this.themeMode === 'system'
          ? `Your OS currently asks for ${resolvesToLight('system') ? 'light' : 'dark'}; the console follows it.`
          : 'Applied immediately, saved for this browser.'}
      </p>
    `
  }

  private renderAbout() {
    if (this.aboutError)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.aboutError}</span>
        </div>
      `
    if (this.aboutLoading || !this.aboutData)
      return html`
        <div class="panel panel-pad">
          ${[0, 1, 2].map(
            () => html`
              <div class="skel-row">
                <div class="skel skel-line" style="width:26%"></div>
                <div class="skel skel-line" style="width:42%"></div>
              </div>
            `,
          )}
        </div>
      `
    const a = this.aboutData
    return html`
      <dl class="about-list">
        <div class="kv">
          <dt>Version</dt>
          <dd><span class="version-chip">${a.version}</span></dd>
        </div>
        <div class="kv">
          <dt>Uptime</dt>
          <dd>${a.uptime}</dd>
        </div>
        <div class="kv">
          <dt>Rust toolchain</dt>
          <dd>${a.rustc_version}</dd>
        </div>
        <div class="kv">
          <dt>Router listen</dt>
          <dd>${a.router_listen ?? '—'}</dd>
        </div>
        <div class="kv">
          <dt>Providers</dt>
          <dd>${a.provider_count}</dd>
        </div>
      </dl>
    `
  }

  render() {
    if (!this.open) return nothing
    return html`
      <div
        class="overlay"
        @click=${(e: MouseEvent) => {
          // 点在遮罩（而非面板）上才关闭。
          if (e.target === e.currentTarget) this.requestClose()
        }}
      >
        <div class="panel" role="dialog" aria-modal="true" aria-label="Settings">
          <h2 class="sr-only">Settings</h2>
          <button class="close" aria-label="Close settings" @click=${this.requestClose}>✕</button>
          <div class="layout">
            <nav class="nav" aria-label="Settings sections">
              ${SECTIONS.map(
                (s) => html`
                  <button
                    class="nav-item"
                    aria-current=${this.section === s.id ? 'true' : 'false'}
                    @click=${() => (this.section = s.id)}
                  >
                    ${icon(s.icon, 14)}${s.label}
                  </button>
                `,
              )}
            </nav>
            <div class="content">${this.renderSection(this.section)}</div>
          </div>
        </div>
      </div>
      ${this.renderProviderDialogs()}
    `
  }

  /** 对话框群：provider 编辑器/删除/设默认 + Services 行动作确认 +
   *  Settings 高危动作二次确认（全部挂在 settings 面板外层）。 */
  private renderProviderDialogs() {
    return html`
      ${this.renderActionConfirmDialogs()}
      <wa-dialog
        label=${this.editorLabel()}
        ?open=${this.editor !== null}
        @wa-hide=${() => (this.editor = null)}
        class="provider-editor"
      >
        ${this.editor === null ? nothing : this.renderEditorBody()}
        <wa-button slot="footer" appearance="plain" @click=${() => (this.editor = null)}>
          Cancel
        </wa-button>
        <wa-button
          slot="footer"
          variant="brand"
          ?disabled=${this.busy}
          @click=${() => void this.submitEditor()}
        >
          ${this.busy ? 'Saving…' : 'Save'}
        </wa-button>
      </wa-dialog>

      <wa-dialog
        label="Delete provider"
        ?open=${this.deleteTarget !== null}
        @wa-hide=${() => (this.deleteTarget = null)}
      >
        <p class="dialog-text">
          Delete provider
          <strong>${this.deleteTarget ?? ''}</strong>? This removes it from the router
          configuration.
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.deleteTarget = null)}>
          Cancel
        </wa-button>
        <wa-button
          slot="footer"
          variant="danger"
          ?disabled=${this.busy}
          @click=${() => void this.confirmDelete()}
        >
          Delete
        </wa-button>
      </wa-dialog>

      <wa-dialog
        label="Set default for new sessions"
        ?open=${this.defaultDraft !== null}
        @wa-hide=${() => (this.defaultDraft = null)}
      >
        ${this.defaultDraft === null
          ? nothing
          : html`
              <p class="dialog-text">
                New sessions will start with provider
                <strong>${this.defaultDraft.provider}</strong>
                ${this.defaultDraft.model ? ` and model <strong>${this.defaultDraft.model}</strong>` : ''}.
              </p>
              ${this.modelChoicesFor(this.defaultDraft.provider).length > 0
                ? html`
                    <wa-select
                      label="Default model"
                      value=${this.defaultDraft.model ?? ''}
                      @change=${(e: Event) =>
                        (this.defaultDraft = {
                          ...this.defaultDraft!,
                          model: (e.target as HTMLSelectElement).value || null,
                        })}
                    >
                      <wa-option value="">(provider default)</wa-option>
                      ${this.modelChoicesFor(this.defaultDraft.provider).map(
                        (m) => html`<wa-option value=${m}>${m}</wa-option>`,
                      )}
                    </wa-select>
                  `
                : html`<p class="dialog-text">This provider has no model catalog yet.</p>`}
            `}
        <wa-button slot="footer" appearance="plain" @click=${() => (this.defaultDraft = null)}>
          Cancel
        </wa-button>
        <wa-button
          slot="footer"
          variant="brand"
          ?disabled=${this.busy || this.defaultDraft === null}
          @click=${() => void this.confirmSetDefault()}
        >
          ${this.busy ? 'Saving…' : 'Set default'}
        </wa-button>
      </wa-dialog>
    `
  }

  /**
   * 二次确认对话框群（D4）：
   *  - Services 行动作 disable/restart：文案含受影响进程名与「不可撤销」；
   *  - Settings 高危动作「全部进程重启」「重置 Settings」。
   * 全部复用既有 wa-dialog 模式，不引入新 modal 框架。
   */
  private renderActionConfirmDialogs() {
    return html`
      <wa-dialog
        label=${this.confirmTarget
          ? `${this.confirmTarget.kind === 'disable' ? 'Disable' : 'Restart'} service`
          : 'Service action'}
        ?open=${this.confirmTarget !== null}
        @wa-hide=${() => (this.confirmTarget = null)}
      >
        <p class="dialog-text">
          ${this.confirmTarget?.kind === 'disable' ? 'Disable' : 'Restart'} managed service
          <strong>${this.confirmTarget?.name ?? ''}</strong>? This takes effect immediately and
          cannot be undone（不可撤销）; in-flight work on that process is interrupted.
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.confirmTarget = null)}>
          Cancel
        </wa-button>
        <wa-button
          slot="footer"
          variant="danger"
          ?disabled=${this.serviceBusy !== null}
          @click=${() => {
            const t = this.confirmTarget
            if (!t) return
            this.confirmTarget = null
            void this.runServiceAction(t.kind, t.name)
          }}
        >
          ${this.confirmTarget?.kind === 'disable' ? 'Disable' : 'Restart'}
        </wa-button>
      </wa-dialog>

      <wa-dialog
        label="全部进程重启"
        ?open=${this.restartAllOpen}
        @wa-hide=${() => (this.restartAllOpen = false)}
      >
        <p class="dialog-text">
          Restart every managed service via the watchdog? 进行中的会话会被中断，此操作不可撤销
          （不可撤销）。The WebUI itself stays up.
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.restartAllOpen = false)}>
          Cancel
        </wa-button>
        <wa-button
          slot="footer"
          variant="danger"
          ?disabled=${this.settingsBusy}
          @click=${() => void this.restartAllProcesses()}
        >
          ${this.settingsBusy ? 'Restarting…' : 'Restart all'}
        </wa-button>
      </wa-dialog>

      <wa-dialog
        label="重置 Settings"
        ?open=${this.resetSettingsOpen}
        @wa-hide=${() => (this.resetSettingsOpen = false)}
      >
        <p class="dialog-text">
          Clear the remembered settings section（lastSettingsSection）in this browser and return
          to the default Settings tab? This cannot be undone.
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.resetSettingsOpen = false)}>
          Cancel
        </wa-button>
        <wa-button slot="footer" variant="danger" @click=${() => this.resetSettings()}>
          Reset
        </wa-button>
      </wa-dialog>
    `
  }

  /** 设默认对话框的模型选项：目标 provider 的 catalog（admin 列表）。 */
  private modelChoicesFor(provider: string): string[] {
    return this.adminProviders.find((p) => p.name === provider)?.models ?? []
  }

  private openSetDefault(p: RouterProviderAdmin): void {
    this.actionError = ''
    this.defaultDraft = { provider: p.name, model: null }
  }

  private async clearDefault(): Promise<void> {
    this.busy = true
    this.actionError = ''
    try {
      this.defaults = await api.setAgentDefaults({ provider: null, model: null })
      window.dispatchEvent(new CustomEvent('sebas:refetch', { bubbles: true, composed: true }))
    } catch (err) {
      this.actionError = err instanceof ApiError ? err.message : String(err)
    } finally {
      this.busy = false
    }
  }

  private async confirmSetDefault(): Promise<void> {
    if (!this.defaultDraft || this.busy) return
    this.busy = true
    this.actionError = ''
    try {
      this.defaults = await api.setAgentDefaults({
        provider: this.defaultDraft.provider,
        model: this.defaultDraft.model,
      })
      this.defaultDraft = null
      // composer 等消费方即时重取模型数据源（与 app-shell 的 refetch 约定一致）。
      window.dispatchEvent(new CustomEvent('sebas:refetch', { bubbles: true, composed: true }))
    } catch (err) {
      this.actionError = err instanceof ApiError ? err.message : String(err)
    } finally {
      this.busy = false
    }
  }

  private editorLabel(): string {
    const e = this.editor
    if (!e) return ''
    if (e.mode === 'create-preset') return 'New provider (preset)'
    if (e.mode === 'create-custom') return 'New provider (custom)'
    return `Edit provider: ${e.name}`
  }

  /** 编辑器表单体：mode 决定哪些字段可编辑。 */
  private renderEditorBody() {
    const e = this.editor!
    const isPresetMode = e.mode === 'create-preset' || (e.mode === 'edit' && !!e.preset)
    const presetDef = this.presets?.find((p) => p.name === e.preset) ?? null
    return html`
      ${this.actionError
        ? html`<div class="callout callout-error" role="alert">${this.actionError}</div>`
        : nothing}
      <div class="editor-grid">
        ${e.mode === 'create-preset' || e.mode === 'create-custom'
          ? html`
              <wa-input
                label="Name"
                required
                .value=${e.name}
                @input=${(ev: any) => this.setEditor({ name: ev.target.value })}
              ></wa-input>
            `
          : html`
              <wa-input label="Name" .value=${e.name} disabled></wa-input>
            `}
        ${e.mode === 'create-preset'
          ? html`
              <wa-select
                label="Preset"
                value=${e.preset}
                @change=${(ev: any) => this.setEditor({ preset: ev.target.value })}
              >
                ${(this.presets ?? []).map(
                  (p) => html`<wa-option value=${p.name}>${p.name}</wa-option>`,
                )}
              </wa-select>
            `
          : nothing}
        ${isPresetMode && presetDef
          ? html`
              <div class="readonly-urls" part="preset-details">
                <div class="readonly-title">Preset values (owned by the code, follow updates)</div>
                <div class="readonly-row">
                  <span>Anthropic</span><code>${presetDef.base_url_anthropic ?? '—'}</code>
                </div>
                <div class="readonly-row">
                  <span>OpenAI Chat</span><code>${presetDef.base_url_openai_chat ?? '—'}</code>
                </div>
                <div class="readonly-row">
                  <span>OpenAI Responses</span>
                  <code>${presetDef.base_url_openai_responses ?? '—'}</code>
                </div>
                <div class="readonly-row"><span>Default env</span><code>${presetDef.api_key_env}</code></div>
              </div>
            `
          : nothing}
        ${!isPresetMode
          ? html`
              <wa-input
                label="Base URL (Anthropic)"
                placeholder="empty = no Anthropic protocol"
                .value=${e.baseUrlAnthropic}
                @input=${(ev: any) => this.setEditor({ baseUrlAnthropic: ev.target.value })}
              ></wa-input>
              <wa-input
                label="Base URL (OpenAI Chat)"
                placeholder="chat-completions endpoint"
                .value=${e.baseUrlOpenaiChat}
                @input=${(ev: any) => this.setEditor({ baseUrlOpenaiChat: ev.target.value })}
              ></wa-input>
              <wa-input
                label="Base URL (OpenAI Responses)"
                placeholder="Responses API endpoint"
                .value=${e.baseUrlOpenaiResponses}
                @input=${(ev: any) => this.setEditor({ baseUrlOpenaiResponses: ev.target.value })}
              ></wa-input>
              <wa-input
                label="API key env var"
                placeholder="e.g. MY_OPENAI_API_KEY"
                .value=${e.apiKeyEnv}
                @input=${(ev: any) => this.setEditor({ apiKeyEnv: ev.target.value })}
              ></wa-input>
            `
          : nothing}
        <wa-input
          label="API key"
          type="password"
          placeholder=${e.mode === 'edit' ? 'leave empty to keep the stored key' : 'paste API key'}
          .value=${e.apiKey}
          @input=${(ev: any) => this.setEditor({ apiKey: ev.target.value })}
        ></wa-input>
        <wa-input
          label="Default model"
          placeholder="model id passed to the agent (optional)"
          .value=${e.defaultModel}
          @input=${(ev: any) => this.setEditor({ defaultModel: ev.target.value })}
        ></wa-input>
        <wa-select
          label="Protocol"
          value=${e.protocol}
          @change=${(ev: any) => this.setEditor({ protocol: ev.target.value })}
        >
          <wa-option value="auto">Auto (Anthropic first)</wa-option>
          <wa-option value="anthropic">Anthropic</wa-option>
          <wa-option value="openai">OpenAI</wa-option>
        </wa-select>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-settings-modal': SebasSettingsModal
  }
}
