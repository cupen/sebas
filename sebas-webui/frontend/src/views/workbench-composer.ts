/**
 * Workbench composer (workbench-interaction-polish D2–D4：纯跟随模式)。
 *
 * The conversation input is SESSION-SCOPED and always in follow-up mode:
 * submit sends the message to the focused session
 * (`POST /api/sessions/{key}/message`); it never spawns a new one — creation
 * lives in the rail's creation dialog (`sebas-new-session-dialog`). The
 * bottom toolbar places the locked agent identity (immutable since creation)
 * at the LEFT and the model chip + submit control at the RIGHT. No settings
 * entry (the app shell owns settings), no permission-mode control (creation
 * mode choice lives in the dialog; mid-session switching stays in the
 * session header), no creation controls of any kind.
 *
 * Submit control state machine (design D4, priority order):
 *   POST 在途 (spinner) > turnInFlight && 无字 (红色停止方块 → cancel 链路)
 *   > turnInFlight && 有字 (排队形态，提交复用既有 turn-queue)
 *   > 有字 (send) > 禁用。
 *
 * Model chip (design D3): single chip at the bottom-right listing the
 * focused session's `available_models` in a two-level menu grouped by
 * provider — the grouping cross-references the shared Settings catalog
 * (`loadModelCatalog`); ids the catalog cannot place fall into an explicit
 * "会话提供" group at the bottom; a wholly unavailable catalog degrades to a
 * flat list. Switching goes through `session/set_config_option`
 * (`POST /api/sessions/{key}/model`). No models = explicit honest note.
 *
 * Reaches the agent-core reachability report from /api/summary to gate
 * submit when the core is offline (a submit would only bounce), re-polled
 * on a 5s interval so the banner and disabled state follow reality. A
 * transient submit error is surfaced inline via the shared `.callout-error`
 * style and the message text is preserved so the operator can retry.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type AgentKindInfo } from '../api/client.js'
import {
  loadModelCatalog,
  groupSessionModels,
  SESSION_PROVIDED_GROUP_LABEL,
  type ModelCatalog,
} from '../api/model-catalog.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'

/** Reachability 轮询周期：断连横幅与 composer 禁用态的翻转延迟上限。 */
const WORKBENCH_REACHABILITY_POLL_MS = 5_000

/** 提交控件的五态（design D4 优先级渲染的判别值，测试按 data-state 断言）。 */
type SubmitState = 'disabled' | 'send' | 'sending' | 'stop' | 'queued'

@customElement('sebas-workbench-composer')
export class SebasWorkbenchComposer extends LitElement {
  /**
   * Focused session's encoded key. `null` = nothing focused: the composer
   * renders no creation controls and an explicit hint pointing at the rail's
   * creation entry (spec「no focus means the rail dialog」).
   */
  @property({ attribute: false }) sessionKey: string | null = null
  /**
   * Focused session's bound agent kind from the wire (`null` = the
   * configured default kind). Rendered read-only (🔒) at the bottom-left.
   */
  @property({ attribute: false }) agentKind: string | null = null
  /** Focused session's selectable models (agent configOptions). */
  @property({ attribute: false }) sessionModels: string[] = []
  /** Focused session's current model id. */
  @property({ attribute: false }) currentModel: string | null = null
  /**
   * （workbench-interaction-polish 4.3，design D4）聚焦会话是否有 turn 在飞
   * （status == Working）。dashboard 供数；turn 结束（WS 推送）自动复位。
   */
  @property({ type: Boolean }) turnInFlight = false

  @state() private text = ''
  @state() private sending = false
  @state() private error: string | null = null
  /** Agent catalog（/api/agents）：跟随模式 🔒 标签的 display 名解析。 */
  @state() private agents: AgentKindInfo[] = []
  /**
   * Settings 目录（design D3 分组交叉引用的数据源）：只用于把会话平铺模型
   * id 归回 provider 组；目录不可得时芯片退化为平铺，不伪造分组。
   */
  @state() private catalog: ModelCatalog | null = null
  /** Set when the agent core is unreachable; gates submit. */
  @state() private unreachable: { ok: false; cause: string } | null = null
  /** 中程切换聚焦会话模型时的在途标记（add-acp-model-selection 语义）。 */
  @state() private modelSwitching = false
  /** 模型芯片两级菜单的开合（design D3）。 */
  @state() private modelMenuOpen = false
  /** Reachability 轮询定时器（connectedCallback 启动，disconnectedCallback 清理）。 */
  private reachabilityTimer: number | undefined = undefined

  private reloadCatalogBound = (): void => {
    void this.loadCatalog()
  }

  connectedCallback(): void {
    super.connectedCallback()
    void this.loadReachability()
    void this.loadAgents()
    void this.loadCatalog()
    // defaults/catalog 变更（管理页 set/clear）即时反映到芯片分组。
    window.addEventListener('sebas:refetch', this.reloadCatalogBound)
    // Reachability 只在挂载时求值一次会让断连横幅永不恢复（core 回来后
    // composer 仍被禁用）——周期性重查，横幅与禁用态随真实状态翻转。
    this.reachabilityTimer = window.setInterval(() => {
      void this.loadReachability()
    }, WORKBENCH_REACHABILITY_POLL_MS)
  }

  disconnectedCallback(): void {
    window.removeEventListener('sebas:refetch', this.reloadCatalogBound)
    this.removeMenuDismissListeners()
    super.disconnectedCallback()
    if (this.reachabilityTimer !== undefined) {
      window.clearInterval(this.reachabilityTimer)
      this.reachabilityTimer = undefined
    }
  }

  protected updated(changed: PropertyValues): void {
    // 聚焦会话变了（切换/关闭/新建跳转）：收起模型菜单、清掉上一个会话的
    // 输入残留交给调用方……文本保留是既有语义（失败重试），这里只在会话
    // 真正更换时清空，避免把 A 会话的草稿发进 B 会话。
    if (changed.has('sessionKey')) {
      this.modelMenuOpen = false
      const prev = changed.get('sessionKey')
      if (prev !== undefined && this.sessionKey !== prev) this.text = ''
    }
  }

  /** 目录加载（design D2：与创建对话框共用 loadModelCatalog，防漂移）。 */
  private async loadCatalog(): Promise<void> {
    const { catalog } = await loadModelCatalog()
    this.catalog = catalog
  }

  private async loadAgents(): Promise<void> {
    try {
      const d = await api.agents()
      this.agents = d.agents
    } catch {
      // catalog 不可达：🔒 标签退回 raw slug（display 名是锦上添花）。
      this.agents = []
    }
  }

  private async loadReachability(): Promise<void> {
    try {
      const data = await api.summary()
      if (data.reachability && data.reachability.ok === false) {
        this.unreachable = { ok: false, cause: data.reachability.cause ?? 'core not connected' }
      } else {
        this.unreachable = null
      }
      // 逐 agent 可用性归 /api/agents（workbench-agent-wire-fix 3.2），
      // summary 只承担 core 可达性门禁。
      void this.loadAgents()
    } catch {
      /* add-webui-allowed-roots D6：summary 请求本身失败（服务进程死亡 /
       * 网络故障）与 reachability.ok = false 同款对待——进入不可达态禁用
       * 提交门，如实呈现而不是放行一次注定失败的提交。轮询恢复后自动
       * 解除。 */
      this.unreachable = { ok: false, cause: '无法获取服务状态（服务可能未运行）' }
    }
  }

  /** 跟随模式输入门禁：无聚焦会话或 core 不可达 = 禁用。 */
  private inputDisabled(): boolean {
    return this.sending || this.sessionKey === null || this.unreachable !== null
  }

  /**
   * 提交控件状态机（design D4，优先级从高到低）：POST 在途（转圈）>
   * turn 在飞且无字（停止方块）> turn 在飞且有字（排队形态）> 有字
   * （send）> 禁用。
   */
  private submitState(): SubmitState {
    if (this.sending) return 'sending'
    if (this.sessionKey === null || this.unreachable !== null) return 'disabled'
    const hasText = this.text.trim().length > 0
    if (this.turnInFlight) return hasText ? 'queued' : 'stop'
    return hasText ? 'send' : 'disabled'
  }

  private async submit(): Promise<void> {
    const key = this.sessionKey
    if (!key) return
    const prompt = this.text.trim()
    if (!prompt) return
    if (this.sending || this.unreachable !== null) return
    this.sending = true
    this.error = null
    try {
      // turn 在飞时服务端自动排队（workbench-turn-queue）——composer 不区分
      // 开轮与排队，提交语义一条路径。
      await api.sendMessage(key, prompt)
      this.text = ''
      // 舞台就地刷新：dashboard 监听后立刻重取聚焦 detail（WS 推送之外的
      // 乐观刷新，避免等下一个 summary 周期）。
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.sending = false
    }
  }

  /**
   * 停止（design D5）：点击红色方块 → 新 cancel 链路
   * （POST /api/sessions/{key}/cancel）。错误走既有 callout；turn 结束
   * （WS 推送 turnInFlight=false）自动回到 send 态。
   */
  private async cancelTurn(): Promise<void> {
    const key = this.sessionKey
    if (!key) return
    this.error = null
    try {
      await api.cancelSession(key)
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    }
  }

  /** 跟随模式的模型切换：`session/set_config_option`（add-acp-model-selection）。 */
  private async switchModel(modelId: string): Promise<void> {
    const key = this.sessionKey
    if (!key || this.modelSwitching) return
    this.modelSwitching = true
    this.error = null
    try {
      await api.setSessionModel(key, modelId)
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.modelSwitching = false
    }
  }

  /**
   * Follow-up mode's read-only agent label: the bound kind resolved to its
   * display name via /api/agents; unknown/unreachable kinds fall back
   * to the raw slug, and `null` (the wire's "no kind recorded") means the
   * configured default.
   */
  private agentLabel(): string {
    if (this.agentKind) {
      const a = this.agents.find((x) => x.id === this.agentKind)
      return a?.display ?? this.agentKind
    }
    return this.agents.find((x) => x.id === 'native' && false)?.display ?? 'default agent'
  }

  // ─── 模型芯片（design D3）────────────────────────────────────────────

  private openModelMenu(): void {
    if (this.sessionModels.length === 0) return
    this.modelMenuOpen = true
    // 捕获阶段监听 document 点击：点菜单外任意处收起（打开菜单的那次点击
    // 已过捕获阶段，不会立刻自吞）。
    document.addEventListener('click', this.dismissMenuOnDocClick, true)
  }

  private closeModelMenu(): void {
    this.modelMenuOpen = false
    this.removeMenuDismissListeners()
  }

  private dismissMenuOnDocClick = (e: Event): void => {
    const path = e.composedPath()
    if (!path.includes(this)) this.closeModelMenu()
  }

  private menuKeydown = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.stopPropagation()
      this.closeModelMenu()
    }
  }

  private removeMenuDismissListeners(): void {
    document.removeEventListener('click', this.dismissMenuOnDocClick, true)
  }

  /**
   * 分组视图（design D3）：目录交叉引用 → provider 组；查不到 → 「会话提供」
   * 置底；目录不可得 → 单组平铺（消费方据 groups.length===1 &&
   * provider===null 识别平铺态，不渲染组头）。
   */
  private modelGroups() {
    return groupSessionModels(this.sessionModels, this.catalog)
  }

  private renderModelChip() {
    if (this.sessionModels.length === 0) {
      // 显式诚实态：该会话无可选模型，绝不渲染空菜单（agent-workbench
      // delta「chip without session models is stated honestly」）。
      return html`<span
        class="label placeholder model-chip-empty"
        data-testid="model-chip-unavailable"
        role="status"
        title="该会话未提供可选模型"
        >无可用模型</span
      >`
    }
    const groups = this.modelGroups()
    const flat = groups.length === 1 && groups[0]!.provider === null
    const label = this.currentModel ?? this.sessionModels[0]!
    return html`
      <div class="model-wrap">
        <button
          class="chip"
          type="button"
          data-testid="model-chip"
          aria-haspopup="listbox"
          aria-expanded=${this.modelMenuOpen ? 'true' : 'false'}
          title="会话模型（切换走 session/set_config_option）"
          @click=${() =>
            this.modelMenuOpen ? this.closeModelMenu() : this.openModelMenu()}
        >
          ${icon('zap', 12)}<span class="chip-label">${label}</span>
        </button>
        ${this.modelMenuOpen
          ? html`
              <div
                class="model-menu"
                role="listbox"
                aria-label="Session model"
                data-testid="model-menu"
                @keydown=${this.menuKeydown}
              >
                ${groups.map((g) =>
                  flat
                    ? this.renderModelItems(g.models)
                    : html`
                        <div class="menu-group">
                          <div class="menu-group-label" data-testid="model-group">
                            ${g.provider ?? SESSION_PROVIDED_GROUP_LABEL}
                          </div>
                          ${this.renderModelItems(g.models)}
                        </div>
                      `,
                )}
              </div>
            `
          : nothing}
      </div>
    `
  }

  private renderModelItems(models: string[]) {
    return models.map(
      (m) => html`
        <button
          class="menu-item"
          type="button"
          role="option"
          aria-selected=${this.currentModel === m ? 'true' : 'false'}
          data-model=${m}
          ?disabled=${this.modelSwitching}
          @click=${() => {
            this.closeModelMenu()
            if (m !== this.currentModel) void this.switchModel(m)
          }}
        >
          <span class="menu-item-label">${m}</span>
          ${this.currentModel === m ? html`<span class="check" aria-hidden="true">✓</span>` : nothing}
        </button>
      `,
    )
  }

  /** 提交控件：按 submitState() 渲染五态（design D4）。 */
  private renderSubmitButton() {
    const state = this.submitState()
    const meta: Record<
      SubmitState,
      { label: string; icon: ReturnType<typeof icon>; disabled: boolean }
    > = {
      disabled: { label: 'Send', icon: icon('forward', 14), disabled: true },
      send: { label: 'Send', icon: icon('forward', 14), disabled: false },
      sending: {
        label: '发送中',
        icon: html`<span class="spinner" aria-hidden="true"></span>`,
        disabled: true,
      },
      stop: { label: '停止回复', icon: icon('stop', 14), disabled: false },
      queued: { label: '排队提交', icon: icon('clock', 14), disabled: false },
    }
    const m = meta[state]
    const onClick =
      state === 'stop'
        ? () => void this.cancelTurn()
        : state === 'send' || state === 'queued'
          ? () => void this.submit()
          : () => {}
    return html`
      <button
        class="send-button ${state}"
        type="button"
        data-state=${state}
        data-testid="submit-control"
        aria-label=${m.label}
        title=${m.label}
        ?disabled=${m.disabled}
        @click=${onClick}
      >
        ${m.icon}
      </button>
    `
  }

  render() {
    // 无聚焦会话：空态提示（dashboard 的 empty-stream 承接主提示，composer
    // 就地给一条指向 rail 创建入口的显式说明），不渲染任何创建控件。
    if (this.sessionKey === null) {
      return html`
        ${this.renderBanners()}
        <div class="composer no-focus" data-testid="composer-no-focus">
          <span class="no-focus-hint">
            在左侧项目栏的 <b>+</b> 新建会话后，这里开始对话。
          </span>
        </div>
      `
    }
    return html`
      ${this.renderBanners()}
      <div class="composer">
        <wa-textarea
          placeholder="Ask for follow-up changes…"
          aria-label="Message"
          resize="none"
          ?disabled=${this.inputDisabled()}
          .value=${this.text}
          @input=${(e: Event) => (this.text = (e.target as HTMLTextAreaElement).value)}
          @keydown=${(e: KeyboardEvent) => {
            // 回车直接发送；Shift+Enter 换行；IME 组词中的回车不触发发送。
            // turn 在飞且有字 = 排队提交（同一发送路径）；无字不触发。
            if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
              e.preventDefault()
              void this.submit()
            }
          }}
        ></wa-textarea>
        <div class="composer-bottom">
          <div class="left-tools">
            <span
              class="label"
              data-testid="agent-lock"
              title="Agent is immutable — chosen when the session was created"
              >🔒 ${this.agentLabel()}</span
            >
          </div>
          <div class="right-tools">
            ${this.renderModelChip()}
            ${this.renderSubmitButton()}
          </div>
        </div>
      </div>
    `
  }

  private renderBanners() {
    return html`
      ${this.unreachable
        ? html`
            <div class="callout callout-warning" role="status">
              ${icon('alert')}<span>core not connected: ${this.unreachable.cause}</span>
            </div>
          `
        : nothing}
      ${this.error
        ? html`
            <div class="callout callout-error" role="alert" data-testid="composer-error">
              ${icon('alert')}<span>${this.error}</span>
            </div>
          `
        : nothing}
    `
  }

  static styles = [
    viewStyles,
    css`
      :host {
        /* 宿主吃满 dashboard 分配的输入框分割面（5.2：拖出的高度给输入
           区），无聚焦提示态同理。 */
        display: flex;
        flex-direction: column;
        min-height: 0;
      }
      /* Composer: ONE rounded shell (浮岛视觉 D6 与预览原型同款), the
       * wa-textarea inside is chrome-stripped so the shell is the only
       * visible card. */
      .composer {
        flex: 1;
        min-height: 0;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: 18px;
        padding: var(--sebas-space-3);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
      }
      .composer wa-textarea {
        width: 100%;
        flex: 1;
        min-height: 36px;
      }
      /* 剥掉 wa-textarea 自带的底色/边框/阴影，只留纯文本输入区；高度随
         分割面拉伸（resize=none 关掉组件自带的拖角，5.2 的分割线是唯一的
         高度控制面），内容多时输入区内部滚动。 */
      .composer wa-textarea::part(base) {
        background: transparent;
        border: none;
        box-shadow: none;
        height: 100%;
        min-height: 36px;
        padding: 4px 8px;
        overflow-y: auto;
      }
      /* 8.1: the native textarea lives in wa-textarea's shadow root, so the
         shared focus-visible rule can't reach it — ring the host instead.
         5.5 / design D5: composer focus is "your move", so the ring is the
         --signal accent, not the shared indigo focus ring. */
      .composer wa-textarea:focus-within {
        outline: 2px solid var(--sebas-signal);
        outline-offset: 2px;
        border-radius: var(--sebas-radius-sm);
      }
      /* 无聚焦会话的显式提示（spec：指向 rail 创建入口，绝不就地给创建控件）。 */
      .composer.no-focus {
        align-items: center;
        justify-content: center;
        padding: var(--sebas-space-4);
      }
      .no-focus-hint {
        font-size: 0.82rem;
        color: var(--sebas-text-dim);
      }
      .no-focus-hint b {
        color: var(--sebas-text-bright);
      }
      /* Bottom toolbar: locked agent identity LEFT, model chip + submit
         RIGHT（agent-workbench delta「toolbar composition」）。 */
      .composer-bottom {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
      }
      .composer-bottom .left-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        min-width: 0;
      }
      .composer-bottom .right-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        margin-left: auto;
      }
      .composer-bottom .label {
        font-family: var(--sebas-font-mono);
      }
      .composer-bottom .label.placeholder {
        color: var(--sebas-text-faint);
        letter-spacing: 0.15em;
      }
      /* ── 模型芯片（design D3）────────────────────────────────────────── */
      .model-wrap {
        position: relative;
        display: inline-flex;
        min-width: 0;
      }
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        max-width: 260px;
        border: 1px solid var(--sebas-border);
        background: var(--sebas-surface-2);
        border-radius: 999px;
        padding: 2px 10px;
        font: inherit;
        font-size: 0.72rem;
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-dim);
        cursor: pointer;
        transition:
          color var(--sebas-dur) var(--sebas-ease),
          border-color var(--sebas-dur) var(--sebas-ease);
      }
      .chip:hover {
        color: var(--sebas-text-bright);
        border-color: var(--sebas-border-strong);
      }
      .chip:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      .chip .chip-label {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .model-chip-empty {
        font-size: 0.72rem;
        letter-spacing: 0.05em;
        font-style: italic;
      }
      /* 两级分组菜单：贴芯片上方弹出（composer 在页底）。 */
      .model-menu {
        position: absolute;
        right: 0;
        bottom: calc(100% + 6px);
        min-width: 240px;
        max-height: 280px;
        overflow-y: auto;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border-strong);
        border-radius: var(--sebas-radius-md);
        box-shadow: var(--sebas-shadow-2);
        padding: 4px;
        z-index: 20;
      }
      .menu-group + .menu-group {
        margin-top: 4px;
      }
      .menu-group-label {
        font-size: 0.66rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--sebas-text-faint);
        padding: 4px 8px 2px;
      }
      .menu-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        border: none;
        background: none;
        border-radius: var(--sebas-radius-sm);
        padding: 5px 8px;
        font: inherit;
        font-size: 0.76rem;
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-dim);
        cursor: pointer;
        text-align: left;
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .menu-item:hover:enabled {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .menu-item:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: -1px;
      }
      .menu-item[aria-selected='true'] {
        color: var(--sebas-accent);
      }
      .menu-item .menu-item-label {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .menu-item .check {
        flex: 0 0 auto;
        color: var(--sebas-accent);
        font-weight: 700;
      }
      /* ── 提交控件状态机（design D4）──────────────────────────────────── */
      /* 28px accent icon send button; disabled dims instead of vanishing. */
      .send-button {
        width: 28px;
        height: 28px;
        display: grid;
        place-items: center;
        background: var(--sebas-accent);
        color: var(--sebas-accent-ink);
        border: none;
        border-radius: var(--sebas-radius-md);
        cursor: pointer;
        padding: 0;
        transition:
          opacity var(--sebas-dur) var(--sebas-ease),
          background var(--sebas-dur) var(--sebas-ease);
      }
      .send-button:disabled {
        opacity: 0.35;
        cursor: not-allowed;
      }
      .send-button:hover:enabled {
        filter: brightness(1.05);
      }
      .send-button:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      /* 流式且输入为空：红色停止方块（点击走 cancel 链路）。 */
      .send-button.stop {
        background: var(--sebas-status-failed);
        opacity: 1;
      }
      /* 流式且有字：排队形态（queued 色，提交进既有 turn-queue）。 */
      .send-button.queued {
        background: var(--sebas-status-queued);
        color: #062a2e;
        opacity: 1;
      }
      /* POST 在途：转圈。 */
      .send-button .spinner {
        width: 13px;
        height: 13px;
        border-radius: 50%;
        border: 2px solid var(--sebas-accent-ink);
        border-top-color: transparent;
        animation: sebas-spin 0.8s linear infinite;
      }
      @keyframes sebas-spin {
        to {
          transform: rotate(360deg);
        }
      }
      @media (prefers-reduced-motion: reduce) {
        /* reduced-motion：转圈降为静态指示（全局 tokens 的 reduced-motion
           规则不进 shadow DOM，组件内自行遵守既有约定）。 */
        .send-button .spinner {
          animation: none;
        }
      }
    `,
  ]
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-workbench-composer': SebasWorkbenchComposer
  }
}
