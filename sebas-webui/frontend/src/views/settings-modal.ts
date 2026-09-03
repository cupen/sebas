/**
 * Settings modal (IA v2)：侧栏底部 Settings 入口打开的居中弹窗，对齐预览
 * 原型 preview-app.ts 的 settings-dialog 布局——暗色面板、左侧分区导航、
 * 右侧内容区、右上关闭按钮。分区由 `section` 属性驱动：
 *
 *   - models   → provider 列表（/api/gateway 的 providers：名称 + base URL）
 *   - services → Gateway 服务状态（listen / debug / auth）
 *   - appearance → 主题三态（system / dark / light；切换与持久化在 theme.ts）
 *   - env      → 环境变量名清单（后端无 env 端点，值一律如实标注
 *                "managed by core config"，绝不编造）
 *   - about    → /api/about 的真实数据（version / rustc / uptime /
 *                gateway listen / provider count）
 *
 * 预览原型里的 'ui' 分区换成这里的 appearance（有了真实的可切换项）；
 * 'network' 分区不迁移。
 *
 * 关闭交互：关闭按钮 / Esc / 点击遮罩 → `open` 置 false 并冒泡 `close`
 * 事件，宿主（app-shell）据此同步状态。
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type About, type GatewayInfo } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import { getThemeMode, resolvesToLight, setThemeMode, type ThemeMode } from '../theme.js'

/**
 * 设置弹窗分区。Models 与 Services 都来自 /api/gateway（是可用的唯一
 * 后端数据面），appearance 由 theme.ts 承担，Env/About 由本组件直渲染
 * （数据源见文件头注释）。预览原型中的 'network' 分区不迁移——没有
 * 真实的可切换项。
 */
export type SettingsSection = 'models' | 'services' | 'appearance' | 'env' | 'about'

/** 分区导航的静态元数据（icon 名见 components/icons.ts）。 */
const SECTIONS: ReadonlyArray<{ id: SettingsSection; label: string; icon: string }> = [
  { id: 'models', label: 'Models', icon: 'zap' },
  { id: 'services', label: 'Services', icon: 'shield' },
  { id: 'appearance', label: 'Appearance', icon: 'sun' },
  { id: 'env', label: 'Environment', icon: 'inbox' },
  { id: 'about', label: 'About', icon: 'about' },
]

const SECTION_DESC: Record<SettingsSection, string> = {
  models: 'Configured providers and their base URLs.',
  services: 'Background services that run alongside sebas.',
  appearance: 'How the console looks. Your choice is saved in this browser.',
  env: 'Environment variables the sebas processes read at startup. The API does not expose values.',
  about: 'Runtime build information.',
}

/** Appearance 分区的主题三态（mode 语义见 theme.ts）。 */
const THEME_OPTIONS: ReadonlyArray<{ mode: ThemeMode; label: string; sub: string }> = [
  { mode: 'system', label: 'System', sub: 'Follow your OS preference' },
  { mode: 'dark', label: 'Dark', sub: 'Always dark' },
  { mode: 'light', label: 'Light', sub: 'Always light' },
]

/**
 * sebas 工作区实际读取的环境变量（grep 自 sebas-router / sebas-gateway /
 * sebas-acp / sebas-webui / core config）。后端没有任何 env 端点，所以
 * 这里只列名字与用途，值一列如实写 "managed by core config"。
 */
const ENV_VARS: ReadonlyArray<{ name: string; what: string }> = [
  { name: 'SEBAS_GATEWAY_CONFIG', what: 'Gateway config file path' },
  { name: 'SEBAS_GATEWAY_LISTEN', what: 'Gateway listen address override' },
  { name: 'SEBAS_GATEWAY_PROVIDER_OVERLAY', what: 'Provider overlay file' },
  { name: 'SEBAS_STATE_FILE', what: 'Session state store path' },
  { name: 'SEBAS_WEBUI_PASSWORD', what: 'WebUI admin password (presence enables auth)' },
  { name: 'SEBAS_CONTROL_SECRET', what: 'Gateway control-plane secret' },
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

  /** 当前分区；缺省 models。 */
  @property({ type: String })
  section: SettingsSection = 'models'

  /** /api/about 响应（About 分区）；懒加载，切到该分区时拉取。 */
  @state() private aboutData: About | null = null
  @state() private aboutError = ''
  @state() private aboutLoading = false
  /** /api/gateway 响应（Models/Services 分区共享）；懒加载，切到时拉取。 */
  @state() private gateway: GatewayInfo | null = null
  @state() private gatewayError = ''
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
    // Models/Services 分区共享 gateway 数据，懒加载一次。
    if (changed.has('section') && (this.section === 'models' || this.section === 'services')) {
      this.loadGateway()
    }
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
    if (this.gateway || this.gatewayError) return // 已加载或已失败（可重试开关）
    api
      .gateway()
      .then((d) => {
        this.gateway = d.gateway
      })
      .catch((e) => {
        this.gatewayError = String(e)
      })
  }

  private renderSectionHead(section: SettingsSection) {
    return html`
      <h2>${SECTIONS.find((s) => s.id === section)?.label ?? section}</h2>
      <p class="desc">${SECTION_DESC[section]}</p>
    `
  }

  // 分区渲染：models/services 共享 /api/gateway 数据（provider 列表 /
  // Gateway 服务状态），env/about 由本组件直渲染（数据源见文件头注释）。
  private renderSection(section: SettingsSection) {
    switch (section) {
      case 'models':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderModels()}
        `
      case 'services':
        return html`
          ${this.renderSectionHead(section)}
          ${this.renderServices()}
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

  /** Models：provider 列表（名称 + base URL）。 */
  private renderModels() {
    if (this.gatewayError)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.gatewayError}</span>
        </div>
      `
    if (!this.gateway)
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
    const providers = this.gateway.providers
    return html`
      <div class="provider-list">
        ${providers.length === 0
          ? html`<div class="provider-row-empty">No providers configured.</div>`
          : providers.map(
              (p) => html`
                <div class="provider-row">
                  <span class="provider-row-name">${p.name}</span>
                  <span class="provider-row-url" title=${p.base_url_anthropic ?? p.base_url_openai ?? ''}>
                    ${p.base_url_anthropic ?? p.base_url_openai ?? 'no base url'}
                  </span>
                </div>
              `,
            )}
      </div>
    `
  }

  /** Services：Gateway 后台服务状态（listen / debug / auth）。 */
  private renderServices() {
    if (this.gatewayError)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.gatewayError}</span>
        </div>
      `
    if (!this.gateway)
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
    const g = this.gateway
    const auth = g.has_auth ? 'configured' : 'none'
    return html`
      <div class="service-card">
        <div class="service-info">
          <div class="service-name">Gateway</div>
          <div class="service-desc">ACP gateway — listens on ${g.listen ?? '—'}</div>
        </div>
        <div class="service-status">
          <span class="dot on"></span> Running
        </div>
      </div>
      <div class="service-card">
        <div class="service-info">
          <div class="service-name">Provider routing</div>
          <div class="service-desc">${g.provider_count} provider(s) · auth ${auth} · debug ${g.debug ? 'on' : 'off'}</div>
        </div>
        <div class="service-status">
          <span class="dot on"></span> ${g.provider_count > 0 ? 'Configured' : 'Idle'}
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
          <dt>Gateway listen</dt>
          <dd>${a.gateway_listen ?? '—'}</dd>
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
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-settings-modal': SebasSettingsModal
  }
}
