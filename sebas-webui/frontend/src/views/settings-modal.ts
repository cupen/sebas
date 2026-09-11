/**
 * Settings modal (IA v3, revamp-settings-nav-and-models-editor)：侧栏底部
 * Settings 入口打开的居中弹窗——暗色面板、左侧分区导航、右侧内容区、
 * 右上关闭按钮。分区由 `section` 属性驱动，顺序即规约：
 *
 *   - generic    → 通用偏好与杂项：原 Env 分区的环境变量只读表（后端无
 *                  env 端点，值一律如实标注 "managed by core config"）；
 *                  为后续语言切换等偏好预留信息架构位置（i18n 另立变更）
 *   - appearance → 主题三态（system / dark / light；切换与持久化在 theme.ts）
 *   - ── 分隔线 ──
 *   - services   → watchdog 受管子进程（GET /api/admin/services：name /
 *                  desired / actual / uptime + /api/admin/events 最近错误；
 *                  enable/disable/restart 动作；无 adapter 时诚实呈现
 *                  「无 watchdog 控制面」横幅且不渲染动作按钮）
 *   - models     → provider 管理列表（redesign-provider-models-settings
 *                  3.4：router 运行状态归 Services，本分区不再呈现网关卡）
 *   - ── 弹性留白 + 分隔线，压底 ──
 *   - about      → INSTANCE 段在上（工作区根目录 + 复制、default agent
 *                  kind、default provider/model + 跳转 Models——原 Settings
 *                  总览的三只读项迁此），BUILD 段在下（/api/about）
 *
 * 原 `Settings` 总览分区移除：其维护动作「全部进程重启」「重置 Settings」
 * 一并删除（逐服务 restart 由 Services 承载，不做广播式入口）。
 *
 * provider 编辑器内的 fetch（revamp-settings-nav-and-models-editor，取代
 * add-fetch-models 的行内 🔍 + 结果列表挑选流）：抓取按钮在编辑器 Models
 * 区块标题旁（仅编辑既有 provider 且有可用 base URL 时渲染）；成功后整单
 * 替换编辑器草稿模型列表（按 id 去重、同 id 保留人工 capability tags），
 * 保存与否走普通编辑流；失败保留草稿并内联呈现净化原因。
 *
 * 上次停留分区记忆在 localStorage `lastSettingsSection`（缺值/非法值——含
 * 旧值 `settings`/`env`——回退缺省 `generic`）。关闭交互：关闭按钮 / Esc /
 * 点击遮罩 → `open` 置 false 并冒泡 `close` 事件，宿主（app-shell）据此
 * 同步状态。
 *
 * 弹窗误关闭修复（redesign-provider-models-settings 2.1 / design D6）：
 * Web Awesome 的子控件（如 `<wa-select>`）收起列表框时会冒泡 composed
 * `wa-hide`；所有 `<wa-dialog>` 的 `@wa-hide` 处理器都带
 * `e.target === e.currentTarget` 来源守卫——只有事件源是对话框自身才关闭。
 */

import { LitElement, css, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import {
  api,
  type About,
  type AdminEvent,
  type AdminService,
  type RouterProviderAdmin,
  type ProviderPreset,
  type ProviderPayload,
  type ProviderModelEntry,
  type ModelCapability,
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
 * 设置弹窗分区（revamp-settings-nav-and-models-editor 1.1）。顺序即规约：
 * generic → appearance →〔分隔线〕services → models →〔弹性留白 + 分隔线，
 * 压底〕about。`settings` 总览分区移除（只读项并入 About、维护动作删除）；
 * `env` 并入 `generic`（分区 id 直接改名，旧记忆值按非法值回退）。
 */
export type SettingsSection =
  | 'generic'
  | 'appearance'
  | 'services'
  | 'models'
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
  { id: 'generic', label: 'Generic', icon: 'settings' },
  { id: 'appearance', label: 'Appearance', icon: 'sun' },
  { id: 'services', label: 'Services', icon: 'shield' },
  { id: 'models', label: 'Models', icon: 'zap' },
  { id: 'about', label: 'About', icon: 'about' },
]

/** 在该分区项之前渲染一条组间分隔线（appearance|services 之间、about 上方）。 */
const NAV_BREAKS: ReadonlySet<SettingsSection> = new Set(['services', 'about'])

const SECTION_DESC: Record<SettingsSection, string> = {
  generic: 'General preferences and misc reference values. Language switching will live here later.',
  appearance: 'How the console looks. Your choice is saved in this browser.',
  services: 'Background services that run alongside sebas.',
  models: 'Manage model providers. Preset-derived values follow the app code; you own the API key.',
  about: 'What this instance is — its workspace, defaults, and the build it runs on.',
}

/** 读取上次停留分区；缺值/非法值（含旧值 `settings`/`env`）一律回退 null
 * （调用方保持缺省 `generic`）。 */
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

  /** 当前分区；缺省 generic（revamp…1.1），re-open 时按记忆恢复。 */
  @property({ type: String })
  section: SettingsSection = 'generic'

  /** /api/about 响应（About 分区）；懒加载，切到该分区时拉取。 */
  @state() private aboutData: About | null = null
  @state() private aboutError = ''
  @state() private aboutLoading = false
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
  /** About INSTANCE 段：工作区根目录（/api/fs/browse-dirs 的服务端解析根）。 */
  @state() private overviewRoot: string | null = null
  @state() private rootCopied = false
  /** provider 管理面（/router/api/providers + /router/api/presets）。 */
  @state() private adminProviders: RouterProviderAdmin[] | null = null
  @state() private adminError = ''
  @state() private presets: ProviderPreset[] | null = null
  /**
   * 编辑器对话框状态：mode 决定字段集；null = 关闭。
   *
   * 表单最小输入（redesign-provider-models-settings 3.1/3.2 / design
   * D3–D5）：预制 = 选 preset + API key + 模型条目（实例名默认取 preset
   * 名）；定制 = 再加实例名 / 单个 base url / 协议；其余 URL 槽、模型
   * 改名映射、default model 收进默认折叠的 Advanced；`api_key_env` 不是
   * 输入项（preset 的 env 名只是隐式回退，编辑时静默回填存量值防丢）。
   */
  @state() private editor: {
    mode: 'create-preset' | 'create-custom' | 'edit'
    name: string
    preset: string
    /** 定制最小输入的单个 base url：按 protocol 落到对应槽位（视图绑定）。 */
    baseUrlAnthropic: string
    baseUrlOpenaiChat: string
    baseUrlOpenaiResponses: string
    apiKey: string
    defaultModel: string
    protocol: string
    /** Advanced：模型改名映射，每行 `旧id -> 新id`。 */
    modelMapText: string
    /** 模型条目（id + 显式能力标记；text 隐含）。 */
    models: ProviderModelEntry[]
    /** 操作者动过条目列表（未动 → preset 派生不落 models，保持跟随代码）。 */
    modelsTouched: boolean
  } | null = null
  @state() private deleteTarget: string | null = null
  /** 默认 provider/model（router admin defaults 透传；workbench-agent-wire-fix
   * 3.3 起 /api/agent-defaults 退役，改读 /router/api/providers 代理旁的
   * admin defaults 端点——经既有 routerProviders 数据推导，无独立端点）。 */
  @state() private defaults: { provider: string | null; model: string | null } | null = null
  /** 设默认对话框的草稿：目标 provider + 可选 model（null = provider 默认）。 */
  @state() private defaultDraft: { provider: string; model: string | null } | null = null
  @state() private busy = false
  @state() private actionError = ''
  /**
   * （revamp…4.3）fetch 进程序（design D3）：pending = 请求在飞；error =
   * 净化后的失败原因。成功不留独立痕迹——结果直接整单替换 `editor.models`
   * （语义=与官方同步），保存与否走普通编辑流。失败绝不冒充空成功列表。
   */
  @state() private fetchState:
    | { state: 'pending' }
    | { state: 'error'; reason: string }
    | null = null
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
    /* 预览原型同款左右布局：左 160px 分区导航（revamp…1.2：加宽加高），
     * 右内容区自滚动。 */
    .layout {
      flex: 1;
      display: flex;
      min-height: 0;
    }
    .nav {
      width: 160px;
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
      gap: 8px;
      flex: 0 0 auto;
      height: 36px;
      padding: 0 12px;
      font-size: 0.875rem;
      font-weight: 500;
      font-family: inherit;
      color: var(--sebas-text-dim);
      cursor: pointer;
      border: none;
      background: none;
      text-align: left;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease),
        box-shadow var(--sebas-dur) var(--sebas-ease);
    }
    .nav .nav-item:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    /* 当前项左侧 accent 竖条（inset box-shadow 不挤占布局）。 */
    .nav .nav-item[aria-current='true'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
      box-shadow: inset 2px 0 0 var(--sebas-accent);
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
    /* 组间分隔线（1px 低对比线，左右留白 12px）；.tail 另加 margin-top:
     * auto 把 About 连同分隔线一起压到导航底部（design D5）。 */
    .nav .nav-sep {
      flex: 0 0 auto;
      height: 1px;
      margin: 6px 12px;
      background: var(--sebas-border);
    }
    .nav .nav-sep.tail {
      margin-top: auto;
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
      flex-direction: column;
      gap: 6px;
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      transition: border-color var(--sebas-dur) var(--sebas-ease);
    }
    .provider-row:hover {
      border-color: var(--sebas-accent-border);
    }
    .provider-row-main {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      min-width: 0;
    }
    /* 模型条目行（redesign-provider-models-settings：列表呈现条目与能力标记）。 */
    .provider-row-models {
      display: flex;
      flex-wrap: wrap;
      gap: 2px 12px;
      font-size: 0.72rem;
    }
    .model-chip {
      display: inline-flex;
      align-items: baseline;
      gap: 4px;
      min-width: 0;
    }
    .model-chip code {
      font-family: var(--sebas-font-mono);
      color: var(--sebas-text-dim);
      overflow-wrap: anywhere;
    }
    .model-chip-tags {
      color: var(--sebas-text-faint);
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
    /* 编辑器 Advanced 折叠区（redesign-provider-models-settings 3.2）与
     * 模型条目编辑器（3.1）。 */
    details.advanced {
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      padding: var(--sebas-space-2) var(--sebas-space-3);
    }
    details.advanced > summary {
      cursor: pointer;
      font-size: 0.78rem;
      font-weight: 600;
      color: var(--sebas-text-dim);
      user-select: none;
    }
    details.advanced > summary:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    details.advanced .advanced-body {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3);
      padding-top: var(--sebas-space-3);
    }
    .advanced-note {
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
    }
    /* 编辑器「Models」区块头（revamp…3.1/4.2）：标题 + 同排 fetch 按钮，
     * 失败原因内联在该区块内（保留草稿、绝不冒充空成功列表）。 */
    .model-entries {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
    }
    .model-entries .entries-head {
      display: flex;
      align-items: center;
      gap: 6px;
    }
    .model-entries .entries-label {
      font-size: 0.78rem;
      font-weight: 600;
      color: var(--sebas-text-dim);
    }
    .model-entries .fetch-error {
      font-size: 0.72rem;
      color: var(--sebas-status-failed, #f87171);
      overflow-wrap: anywhere;
    }
    .model-entry-row {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-2);
      flex-wrap: wrap;
    }
    .model-entry-row wa-input {
      flex: 1 1 200px;
      min-width: 0;
    }
    .model-entry-row .tag-check {
      display: inline-flex;
      align-items: center;
      gap: 3px;
      font-size: 0.72rem;
      color: var(--sebas-text-dim);
      cursor: pointer;
      user-select: none;
    }
    .model-entry-row .tag-check input {
      accent-color: var(--sebas-accent, currentColor);
      cursor: pointer;
    }
    /* 「＋」通栏按钮（revamp…3.1）：整行宽度的幽灵按钮，虚线边暗示可加行。 */
    .model-entries .add-model {
      width: 100%;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 5px 0;
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      background: none;
      color: var(--sebas-text-dim);
      font-size: 0.9rem;
      line-height: 1.2;
      cursor: pointer;
      transition:
        border-color var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .model-entries .add-model:hover {
      border-color: var(--sebas-accent-border);
      color: var(--sebas-accent);
    }
    .model-entries .add-model:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
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
    /* Env 清单（Generic 分区承载）：变量名 + 用途 + 固定的
     * "managed by core config" 值。 */
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
    /* About 分区：INSTANCE 段在上（原 Settings 总览三只读项）、BUILD 段
     * （/api/about）在下的分段小标题。 */
    .about-seg-title {
      margin: var(--sebas-space-4) 0 var(--sebas-space-1);
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
    }
    .about-seg-title:first-child {
      margin-top: 0;
    }
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
    // About 分区懒加载（revamp…2.2：INSTANCE + BUILD 两段）：切到 about 时
    // 拉 /api/about（BUILD）与工作区根目录（INSTANCE；失败可重试——下次切
    // 换再取）。
    if (changed.has('section') && this.section === 'about') {
      this.loadAbout()
      this.loadOverview()
    }
    // provider 管理面：每次切到 models 都刷新（增删改后重新进入也新鲜）。
    // redesign-provider-models-settings 3.4：Models 分区不再读 /api/router。
    if (changed.has('section') && this.section === 'models') this.loadProviders()
    // Services 分区：受管子进程 + 最近错误（每次切入都刷新，动作后重取）。
    if (changed.has('section') && this.section === 'services') this.loadServices()
  }

  /**
   * 分区记忆（D5）：打开时按 localStorage 恢复上次分区（缺值/非法值——含
   * 旧值 `settings`/`env`——回退缺省 `generic`）；打开期间每次切换都写回。
   * 仅在 `open` 翻真/分区变化时触发，读写都容忍 storage 不可用。
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

  /**
   * About INSTANCE 段加载（revamp…2.2，原 Settings 总览路径并入 About）：
   * 只读项全部来自既有端点，绝不编造。adapter 探测随「全部进程重启」的
   * 删除一并移除——Services 分区自己拉取并呈现同一事实。
   */
  private loadOverview(): void {
    // workbench-agent-wire-fix 3.3：/api/agent-defaults 退役——default
    // provider/model 行如实呈现「未设置」（★ 设置的本地默认不跨弹窗存活）。
    this.defaults = null
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
    this.fetchState = null
    this.editor = {
      mode: 'create-preset',
      name: '',
      preset: this.presets?.[0]?.name ?? '',
      baseUrlAnthropic: '',
      baseUrlOpenaiChat: '',
      baseUrlOpenaiResponses: '',
      apiKey: '',
      defaultModel: '',
      protocol: 'auto',
      modelMapText: '',
      models: [],
      modelsTouched: false,
    }
  }

  private openCreateCustom(): void {
    this.actionError = ''
    this.fetchState = null
    this.editor = {
      mode: 'create-custom',
      name: '',
      preset: '',
      baseUrlAnthropic: '',
      baseUrlOpenaiChat: '',
      baseUrlOpenaiResponses: '',
      apiKey: '',
      defaultModel: '',
      protocol: 'openai',
      modelMapText: '',
      models: [],
      modelsTouched: false,
    }
  }

  private openEdit(p: RouterProviderAdmin): void {
    this.actionError = ''
    this.fetchState = null
    const map = p.model_map ?? {}
    this.editor = {
      mode: 'edit',
      name: p.name,
      preset: p.preset ?? '',
      baseUrlAnthropic: p.base_url_anthropic ?? '',
      baseUrlOpenaiChat: p.base_url_openai_chat ?? '',
      baseUrlOpenaiResponses: p.base_url_openai_responses ?? '',
      // 密钥绝不回填——空提交 = 保留旧 key（服务端语义）。
      apiKey: '',
      // default_model 在 Advanced（编辑回填；put 整体替换，回填防丢）。
      defaultModel: p.default_model ?? '',
      protocol: p.protocol ?? 'auto',
      modelMapText: Object.entries(map)
        .map(([from, to]) => `${from} -> ${to}`)
        .join('\n'),
      models: p.models.map((m) => ({ id: m.id, tags: [...m.tags] })),
      modelsTouched: false,
    }
  }

  private setEditor(patch: Partial<NonNullable<SebasSettingsModal['editor']>>): void {
    if (this.editor) this.editor = { ...this.editor, ...patch }
  }

  // ---- 模型条目编辑（3.1：可增删、可勾选能力；text 隐含）----

  private setModelId(index: number, id: string): void {
    if (!this.editor) return
    const models = this.editor.models.map((m, i) => (i === index ? { ...m, id } : m))
    this.setEditor({ models, modelsTouched: true })
  }

  private toggleModelTag(index: number, tag: ModelCapability, on: boolean): void {
    if (!this.editor) return
    const models = this.editor.models.map((m, i) => {
      if (i !== index) return m
      const tags = m.tags.filter((t) => t !== tag)
      if (on) tags.push(tag)
      return { ...m, tags }
    })
    this.setEditor({ models, modelsTouched: true })
  }

  private addModelEntry(): void {
    if (!this.editor) return
    this.setEditor({ models: [...this.editor.models, { id: '', tags: [] }], modelsTouched: true })
  }

  private removeModelEntry(index: number): void {
    if (!this.editor) return
    this.setEditor({
      models: this.editor.models.filter((_, i) => i !== index),
      modelsTouched: true,
    })
  }

  /** Advanced 的模型改名映射文本 → 对象（跳过残行；`旧id -> 新id` 每行）。 */
  private parseModelMap(text: string): Record<string, string> {
    const out: Record<string, string> = {}
    for (const line of text.split('\n')) {
      const [from, to] = line.split('->').map((s) => s.trim())
      if (from && to) out[from] = to
    }
    return out
  }

  /**
   * 提交载荷（3.1/3.2 验收锚点）：
   * - 预制派生：绝不含 `base_url_*` / `api_key_env`（连接数据跟随代码）；
   * - 定制：Advanced 展开与否不影响「最小提交」形态——其余 URL 槽 / 改名
   *   映射只在有值时出现（默认折叠 = 空值 = 不提交）；
   * - `models` 只在操作者动过条目列表时提交（预制派生未动 = 保持跟随
   *   代码表；定制编辑恒提交，防整体替换丢目录）；
   * - `api_key_env` 不是输入项；定制编辑静默回填存量值（与空 key 保留
   *   同一姿态）。
   */
  private editorPayload(stored: RouterProviderAdmin | null): ProviderPayload {
    const e = this.editor
    if (!e) return {}
    const payload: ProviderPayload = { protocol: e.protocol }
    const isPreset = e.mode === 'create-preset' || (e.mode === 'edit' && !!e.preset)
    if (isPreset) {
      payload.preset = e.preset
      if (e.defaultModel.trim()) payload.default_model = e.defaultModel.trim()
    } else {
      payload.base_url_anthropic = e.baseUrlAnthropic.trim() || undefined
      payload.base_url_openai_chat = e.baseUrlOpenaiChat.trim() || undefined
      payload.base_url_openai_responses = e.baseUrlOpenaiResponses.trim() || undefined
      if (e.defaultModel.trim()) payload.default_model = e.defaultModel.trim()
      const mm = this.parseModelMap(e.modelMapText)
      if (Object.keys(mm).length > 0) payload.model_map = mm
      if (e.mode === 'edit') payload.api_key_env = stored?.api_key_env ?? undefined
    }
    if (e.apiKey.trim()) payload.api_key = e.apiKey.trim()
    const entries = e.models
      .filter((m) => m.id.trim())
      .map((m) => ({ id: m.id.trim(), tags: [...m.tags] }))
    if (e.modelsTouched || (e.mode === 'edit' && !isPreset)) payload.models = entries
    if (e.mode === 'create-preset' || e.mode === 'create-custom') {
      // D5：预制实例名缺省取 preset 名；name 输入只在 Advanced。
      payload.name = e.name.trim() || (e.mode === 'create-preset' ? e.preset : '')
    }
    return payload
  }

  private async submitEditor(): Promise<void> {
    if (!this.editor || this.busy) return
    const e = this.editor
    if (e.mode === 'create-custom' && !e.name.trim()) {
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
    const stored =
      e.mode === 'edit'
        ? (this.adminProviders?.find((p) => p.name === e.name) ?? null)
        : null
    const payload = this.editorPayload(stored)
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

  /**
   * （revamp…4.2，design D3/D4）编辑器内抓取：调 core 的 providers 域抓取
   * op（只读，**不提交任何写请求**）。成功 → 整单替换编辑器草稿的模型列表
   * （按 id 去重；同 id 的既有条目保留人工 capability tags；Map 合并以抓取
   * 顺序为骨架）；失败 → 保留草稿、内联呈现净化原因。保存与否由用户走普通
   * 编辑流（fetch 置 modelsTouched，preset 派生的替换才会随保存提交）。
   */
  private async fetchModelsIntoEditor(): Promise<void> {
    const e = this.editor
    if (!e || e.mode !== 'edit' || this.busy || this.fetchState?.state === 'pending') return
    this.fetchState = { state: 'pending' }
    this.actionError = ''
    try {
      const r = await api.fetchProviderModels(e.name)
      const tagsById = new Map(e.models.map((m) => [m.id, m.tags]))
      const seen = new Set<string>()
      const models: ProviderModelEntry[] = []
      for (const id of r.models) {
        if (seen.has(id)) continue
        seen.add(id)
        models.push({ id, tags: [...(tagsById.get(id) ?? [])] })
      }
      this.setEditor({ models, modelsTouched: true })
      this.fetchState = null
    } catch (err) {
      this.fetchState = {
        state: 'error',
        reason: err instanceof ApiError ? err.message : String(err),
      }
    }
  }

  /**
   * （revamp…4.2 / design D4）编辑器 fetch 按钮的渲染条件：仅在编辑既有
   * provider 且「有可用 base URL」时渲染——preset 派生用 presetDef 的
   * code-table URL 判断；custom 用草稿任一槽位非空判断。新建模式（create-*）
   * 没有已存储的 provider 可探测（probe op 按 name 寻址），不渲染。
   */
  private canFetchInEditor(): boolean {
    const e = this.editor
    if (!e || e.mode !== 'edit') return false
    const presetDef = this.presets?.find((p) => p.name === e.preset) ?? null
    const hasUsableUrl =
      e.preset && presetDef
        ? !!(presetDef.base_url_anthropic ||
            presetDef.base_url_openai_chat ||
            presetDef.base_url_openai_responses)
        : !!(e.baseUrlAnthropic.trim() ||
            e.baseUrlOpenaiChat.trim() ||
            e.baseUrlOpenaiResponses.trim())
    return hasUsableUrl
  }

  private renderSectionHead(section: SettingsSection) {
    return html`
      <h2>${SECTIONS.find((s) => s.id === section)?.label ?? section}</h2>
      <p class="desc">${SECTION_DESC[section]}</p>
    `
  }

  // 分区渲染：generic 承载原 Env 只读表；services 读 watchdog 受管子进程
  // 面；models 承载 provider 管理（router 运行状态归 Services，3.4）；
  // about = INSTANCE（原 Settings 总览只读项）+ BUILD（/api/about）。
  private renderSection(section: SettingsSection) {
    switch (section) {
      case 'generic':
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
      case 'about':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderAbout()}
        `
    }
  }

  /**
   * Models：provider 管理页（列表 + 新增/编辑/删除）。抓取入口在编辑器
   * 「Models」区块标题旁（revamp…4.2），provider 行不再带 🔍。router 运行
   * 状态（listen / debug / auth / desired / actual）不在此呈现——归
   * Services 分区（redesign-provider-models-settings 3.4 / spec「router
   * 状态只在 Services 呈现」）。
   */
  private renderModels() {
    const providers = this.adminProviders
    return html`
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
        <div class="provider-row-main">
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
        ${p.models.length > 0
          ? html`<div class="provider-row-models">
              ${p.models.map(
                (m) => html`
                  <span class="model-chip">
                    <code>${m.id}</code>${m.tags.length
                      ? html`<span class="model-chip-tags">${m.tags.join(' ')}</span>`
                      : nothing}
                  </span>
                `,
              )}
            </div>`
          : nothing}
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
    // core 恒启动（enable-core-by-default）：不渲染 enable/disable，只留
    // restart（走 restart-core 确认路径）。
    const alwaysOn = s.name === 'core'
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
          ${alwaysOn
            ? ''
            : html`
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
              `}
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

  /**
   * About（revamp…2.2）：INSTANCE 段在上——原 Settings 总览的三只读项
   * （工作区根目录 + 复制、default agent kind、default provider/model +
   * 跳转 Models）；BUILD 段在下——/api/about 的真实字段。缺值如实显示
   * '—'，绝不编造。
   */
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
      <h3 class="about-seg-title">Instance</h3>
      <dl class="about-list about-instance">
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

      <h3 class="about-seg-title">Build</h3>
      <dl class="about-list about-build">
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
                (s, i) => html`
                  ${i > 0 && NAV_BREAKS.has(s.id)
                    ? html`<div
                        class="nav-sep${s.id === 'about' ? ' tail' : ''}"
                        role="separator"
                      ></div>`
                    : nothing}
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

  /**
   * wa-hide 来源守卫（redesign-provider-models-settings 2.1 / design D6）：
   * Web Awesome 子控件（如 `<wa-select>`）收起自身列表框时会冒泡 composed
   * `wa-hide`；`<wa-dialog>` 只在事件源是对话框自身时才关闭，子控件冒泡
   * 上来的 hide 一律忽略——编辑器内任何选择交互都不会连带关闭整个弹窗。
   */
  private guardedHide(close: () => void): (e: Event) => void {
    return (e: Event) => {
      if (e.target === e.currentTarget) close()
    }
  }

  /** 对话框群：provider 编辑器/删除/设默认 + Services 行动作确认（全部
   *  挂在 settings 面板外层；全部带 wa-hide 来源守卫——编辑器 / 设默认 /
   *  删除 / 服务确认，共 4 处）。原 Settings 高危动作对话框（全部进程重启 /
   *  重置 Settings）随分区删除一并移除（revamp…2.1）。 */
  private renderProviderDialogs() {
    return html`
      ${this.renderActionConfirmDialogs()}
      <wa-dialog
        label=${this.editorLabel()}
        ?open=${this.editor !== null}
        @wa-hide=${this.guardedHide(() => (this.editor = null))}
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
        @wa-hide=${this.guardedHide(() => (this.deleteTarget = null))}
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
        @wa-hide=${this.guardedHide(() => (this.defaultDraft = null))}
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
   * 二次确认对话框（D4）：Services 行动作 disable/restart——文案含受影响
   * 进程名与「不可撤销」。复用既有 wa-dialog 模式，不引入新 modal 框架。
   */
  private renderActionConfirmDialogs() {
    return html`
      <wa-dialog
        label=${this.confirmTarget
          ? `${this.confirmTarget.kind === 'disable' ? 'Disable' : 'Restart'} service`
          : 'Service action'}
        ?open=${this.confirmTarget !== null}
        @wa-hide=${this.guardedHide(() => (this.confirmTarget = null))}
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
    `
  }

  /** 设默认对话框的模型选项：目标 provider 目录条目的 id（admin 列表）。 */
  private modelChoicesFor(provider: string): string[] {
    return (
      this.adminProviders
        ?.find((p) => p.name === provider)
        ?.models.map((m) => m.id) ?? []
    )
  }

  private openSetDefault(p: RouterProviderAdmin): void {
    this.actionError = ''
    this.defaultDraft = { provider: p.name, model: null }
  }

  private async clearDefault(): Promise<void> {
    this.busy = true
    this.actionError = ''
    try {
      this.defaults = null
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
      this.defaults = {
        provider: this.defaultDraft.provider,
        model: this.defaultDraft.model,
      }
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

  /** 定制最小输入的「单个 Base URL」读写的是 protocol 选中的槽位
   *  （design D4：一个 URL 落到协议命名的槽位，绝不写满三槽）。 */
  private primaryBaseUrl(): string {
    const e = this.editor!
    return e.protocol === 'anthropic' ? e.baseUrlAnthropic : e.baseUrlOpenaiChat
  }

  private setPrimaryBaseUrl(v: string): void {
    const e = this.editor
    if (!e) return
    if (e.protocol === 'anthropic') this.setEditor({ baseUrlAnthropic: v })
    else this.setEditor({ baseUrlOpenaiChat: v })
  }

  /**
   * 模型条目编辑器（revamp…3.1/4.2）：id + 能力勾选（text 隐含，vision/
   * audio/video 显式；标记是展示用元数据，不影响路由）。区块标题「Models」
   * 旁是 fetch 按钮（仅编辑既有 provider 且有可用 base URL 时渲染，D4）；
   * 抓取成功整单替换下方草稿列表；失败在区块内联呈现净化原因、草稿不动。
   */
  private renderModelEntries(): TemplateResult {
    const e = this.editor!
    const fetching = this.fetchState?.state === 'pending'
    return html`
      <div class="model-entries">
        <div class="entries-head">
          <span class="entries-label">Models</span>
          ${this.canFetchInEditor()
            ? html`
                <button
                  class="row-action"
                  title="Fetch model list from the provider's official base URL"
                  data-testid="fetch-models"
                  ?disabled=${this.busy || fetching}
                  @click=${() => void this.fetchModelsIntoEditor()}
                >
                  🔍
                </button>
                ${fetching
                  ? html`<span class="entries-label" role="status">fetching…</span>`
                  : nothing}
              `
            : nothing}
          ${this.fetchState?.state === 'error'
            ? html`<span class="fetch-error" role="alert">
                fetch failed — ${this.fetchState.reason}
              </span>`
            : nothing}
        </div>
        ${e.models.map(
          (m, i) => html`
            <div class="model-entry-row" data-testid="model-entry">
              <wa-input
                placeholder="model id"
                .value=${m.id}
                @input=${(ev: Event) => this.setModelId(i, (ev.target as HTMLInputElement).value)}
              ></wa-input>
              ${(['vision', 'audio', 'video'] as const).map(
                (tag) => html`
                  <label class="tag-check">
                    <input
                      type="checkbox"
                      ?checked=${m.tags.includes(tag)}
                      data-testid=${`tag-${tag}`}
                      @change=${(ev: Event) =>
                        this.toggleModelTag(
                          i,
                          tag,
                          (ev.target as HTMLInputElement).checked,
                        )}
                    />
                    ${tag}
                  </label>
                `,
              )}
              <button
                class="row-action danger"
                title="Remove model entry"
                ?disabled=${this.busy}
                @click=${() => this.removeModelEntry(i)}
              >
                ✕
              </button>
            </div>
          `,
        )}
        <button
          class="add-model"
          title="Add a model entry"
          data-testid="add-model-entry"
          ?disabled=${this.busy}
          @click=${() => this.addModelEntry()}
        >
          ＋
        </button>
      </div>
    `
  }

  /** 编辑器表单体（3.1/3.2）：mode 决定最小字段集；其余输入收进默认折叠
   *  的 Advanced（`<details class="advanced">`）。`api_key_env` 不是输入
   *  项——只在 Advanced 如实展示继承/存量的 env 回退名。 */
  private renderEditorBody() {
    const e = this.editor!
    const isPreset = e.mode === 'create-preset' || (e.mode === 'edit' && !!e.preset)
    const presetDef = this.presets?.find((p) => p.name === e.preset) ?? null
    const stored =
      e.mode === 'edit' ? (this.adminProviders?.find((p) => p.name === e.name) ?? null) : null
    const advancedEnvName = isPreset
      ? (presetDef?.api_key_env ?? stored?.api_key_env ?? '')
      : (stored?.api_key_env ?? '')
    const primaryIsAnthropic = e.protocol === 'anthropic'
    return html`
      ${this.actionError
        ? html`<div class="callout callout-error" role="alert">${this.actionError}</div>`
        : nothing}
      <div class="editor-grid">
        ${e.mode === 'create-custom'
          ? html`
              <wa-input
                label="Name"
                required
                .value=${e.name}
                @input=${(ev: Event) => this.setEditor({ name: (ev.target as HTMLInputElement).value })}
              ></wa-input>
            `
          : e.mode === 'edit'
            ? html`<wa-input label="Name" .value=${e.name} disabled></wa-input>`
            : nothing}
        ${e.mode === 'create-preset'
          ? html`
              <wa-select
                label="Preset"
                value=${e.preset}
                @change=${(ev: Event) =>
                  this.setEditor({ preset: (ev.target as HTMLSelectElement).value })}
              >
                ${(this.presets ?? []).map(
                  (p) => html`<wa-option value=${p.name}>${p.name}</wa-option>`,
                )}
              </wa-select>
            `
          : nothing}
        ${isPreset && presetDef
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
              </div>
            `
          : nothing}
        ${!isPreset
          ? html`
              <wa-input
                label=${primaryIsAnthropic ? 'Base URL (Anthropic)' : 'Base URL (OpenAI-compatible)'}
                placeholder=${primaryIsAnthropic
                  ? 'anthropic-messages endpoint'
                  : 'chat-completions endpoint'}
                .value=${this.primaryBaseUrl()}
                @input=${(ev: Event) =>
                  this.setPrimaryBaseUrl((ev.target as HTMLInputElement).value)}
              ></wa-input>
              <wa-select
                label="Protocol"
                value=${e.protocol}
                @change=${(ev: Event) =>
                  this.setEditor({ protocol: (ev.target as HTMLSelectElement).value })}
              >
                <wa-option value="openai">OpenAI-compatible</wa-option>
                <wa-option value="anthropic">Anthropic</wa-option>
                ${e.mode === 'edit'
                  ? html`<wa-option value="auto">Auto (stored preference)</wa-option>`
                  : nothing}
              </wa-select>
            `
          : nothing}
        <wa-input
          label="API key"
          type="password"
          placeholder=${e.mode === 'edit' ? 'leave empty to keep the stored key' : 'paste API key'}
          .value=${e.apiKey}
          @input=${(ev: Event) => this.setEditor({ apiKey: (ev.target as HTMLInputElement).value })}
        ></wa-input>
        ${this.renderModelEntries()}

        <details class="advanced">
          <summary>Advanced</summary>
          <div class="advanced-body">
            ${e.mode === 'create-preset'
              ? html`
                  <wa-input
                    label="Name (defaults to the preset name)"
                    placeholder=${e.preset}
                    .value=${e.name}
                    @input=${(ev: Event) =>
                      this.setEditor({ name: (ev.target as HTMLInputElement).value })}
                  ></wa-input>
                `
              : nothing}
            ${!isPreset
              ? html`
                  <wa-input
                    label=${primaryIsAnthropic
                      ? 'Base URL (OpenAI-compatible)'
                      : 'Base URL (Anthropic)'}
                    placeholder="empty = protocol not served"
                    .value=${primaryIsAnthropic ? e.baseUrlOpenaiChat : e.baseUrlAnthropic}
                    @input=${(ev: Event) =>
                      primaryIsAnthropic
                        ? this.setEditor({ baseUrlOpenaiChat: (ev.target as HTMLInputElement).value })
                        : this.setEditor({ baseUrlAnthropic: (ev.target as HTMLInputElement).value })}
                  ></wa-input>
                  <wa-input
                    label="Base URL (OpenAI Responses)"
                    placeholder="Responses API endpoint"
                    .value=${e.baseUrlOpenaiResponses}
                    @input=${(ev: Event) =>
                      this.setEditor({
                        baseUrlOpenaiResponses: (ev.target as HTMLInputElement).value,
                      })}
                  ></wa-input>
                `
              : nothing}
            ${advancedEnvName
              ? html`<div class="advanced-note">
                  API key env fallback: <code>${advancedEnvName}</code> (inherited; used only when
                  no plaintext key is stored)
                </div>`
              : nothing}
            <wa-input
              label="Default model"
              placeholder="model id passed to the agent (optional)"
              .value=${e.defaultModel}
              @input=${(ev: Event) =>
                this.setEditor({ defaultModel: (ev.target as HTMLInputElement).value })}
            ></wa-input>
            ${!isPreset
              ? html`
                  <wa-input
                    label="Model rename map (one per line: old-id -> new-id)"
                    placeholder="old-id -> new-id"
                    .value=${e.modelMapText}
                    @input=${(ev: Event) =>
                      this.setEditor({ modelMapText: (ev.target as HTMLInputElement).value })}
                  ></wa-input>
                `
              : nothing}
            ${isPreset
              ? html`
                  <wa-select
                    label="Protocol"
                    value=${e.protocol}
                    @change=${(ev: Event) =>
                      this.setEditor({ protocol: (ev.target as HTMLSelectElement).value })}
                  >
                    <wa-option value="auto">Auto (Anthropic first)</wa-option>
                    <wa-option value="anthropic">Anthropic</wa-option>
                    <wa-option value="openai">OpenAI</wa-option>
                  </wa-select>
                `
              : nothing}
          </div>
        </details>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-settings-modal': SebasSettingsModal
  }
}
