/**
 * Workbench composer (add-composer-agent-binding): the conversation-area
 * input is SESSION-SCOPED, in one of two modes.
 *
 * Follow-up mode (a session is focused): submit sends the message to the
 * focused session (`POST /api/sessions/{key}/message`) — it never spawns a
 * new one. The focused session's agent is fixed at creation time, so the
 * bottom toolbar shows the agent as small read-only text next to a model
 * dropdown (switching = `session/set_config_option` via the model endpoint).
 *
 * Creation mode (no focused session, or the operator pressed the "new
 * session" chip): submit spawns a session bound to the selected project (or
 * the inbox) with the agent picked in the toolbar — the only place an agent
 * can be chosen, because after spawn the binding is immutable
 * (workbench-agent-wire-fix: agent 必填、取自 /api/agents 唯一真源、创建后
 * 不可变；项目 default_agent 预选)。
 *
 * Reaches the agent-core reachability report from /api/summary to gate
 * submit when the core is offline (a submit would only bounce), re-polled
 * on a 5s interval so the banner and disabled state follow reality. A
 * transient submit error is surfaced inline via the shared `.callout-error`
 * style and the message text is preserved so the operator can retry.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type AgentKindInfo,  } from '../api/client.js'
import { toModelCatalog, type ModelCatalog } from '../api/model-catalog.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

/** Reachability 轮询周期：断连横幅与 composer 禁用态的翻转延迟上限。 */
const WORKBENCH_REACHABILITY_POLL_MS = 5_000

@customElement('sebas-workbench-composer')
export class SebasWorkbenchComposer extends LitElement {
  /**
   * 选中项目的稳定 id（workbench-agent-wire-fix 2.5）；null = inbox，
   * 创建请求省略 project_id。路径标识符不再上 wire。
   */
  @property({ attribute: false }) projectId: string | null = null
  /** 选中项目的展示路径片段（binding 提示用；仅 UI 文案，非标识符）。 */
  @property({ attribute: false }) projectDir: string | null = null
  /** 项目级默认 agent（该项目最近一次创建会话所用；D5 预选用）。 */
  @property({ attribute: false }) projectDefaultAgent: string | null = null
  /**
   * Read-only label like "anthropic / claude-sonnet-4-5". May be null
   * while loading.
   */
  @property({ attribute: false }) providerLabel: string | null = null
  /**
   * Focused session's encoded key (add-composer-agent-binding). Non-null
   * puts the composer in follow-up mode; null is creation mode.
   */
  @property({ attribute: false }) sessionKey: string | null = null
  /**
   * Focused session's bound agent kind from the wire (`null` = the
   * configured default kind). Only rendered in follow-up mode.
   */
  @property({ attribute: false }) agentKind: string | null = null
  /** Focused session's selectable models (agent configOptions). */
  @property({ attribute: false }) sessionModels: string[] = []
  /** Focused session's current model id. */
  @property({ attribute: false }) currentModel: string | null = null

  @state() private text = ''
  @state() private sending = false
  @state() private error: string | null = null
  /**
   * 创建模式选定的 agent id（workbench-agent-wire-fix D2）：必填——词汇表
   * 是 /api/agents 的 id 列（配置键名或 "native"），无隐式默认；未选择时
   * 提交门禁禁用。会话创建后 agent 不可变。
   */
  @state() private agent = ''
  /** Agent catalog（/api/agents；唯一可用性真源，含 native 行）。 */
  @state() private agents: AgentKindInfo[] = []
  /**
   * 创建模式两级选择的模型 id（workbench-conversation-view 4.3）：目录里
   * 选定 provider 下的 model；提交时随创建请求下发。
   */
  @state() private model: string | null = null
  /** 创建模式两级选择的一级：选定的 provider 名。 */
  @state() private selectedProvider: string | null = null
  /**
   * Settings 目录（workbench-conversation-view 4.2/4.3，design D7/D8）：
   * adapter 只规整不解释；`null` = 目录尚未取得。
   */
  @state() private catalog: ModelCatalog | null = null
  /**
   * 目录显式不可得（4.4）：读取失败（router/core 状态库不在跑）或目录为空
   * ——下拉位置显示显式不可用提示并禁用，绝不伪造选项、绝不显示空列表。
   */
  @state() private catalogUnavailable = false
  /** Set when the agent core is unreachable; gates submit. */
  @state() private unreachable: { ok: false; cause: string } | null = null
  /** 中程切换聚焦会话模型时的在途标记（add-acp-model-selection 语义）。 */
  @state() private modelSwitching = false
  /**
   * Operator explicitly requested creation while a session is focused;
   * cleared whenever the focused key changes.
   */
  @state() private createRequested = false
  /** Reachability 轮询定时器（connectedCallback 启动，disconnectedCallback 清理）。 */
  private reachabilityTimer: number | undefined = undefined

  /** Follow-up when a session is focused and creation wasn't requested. */
  private get isFollowMode(): boolean {
    return this.sessionKey !== null && !this.createRequested
  }

  static styles = [
    viewStyles,
    css`
      :host {
        display: block;
      }
      /* Composer: ONE rounded shell (preview 工作台同款), the wa-textarea
       * inside is chrome-stripped so the shell is the only visible card. */
      .composer {
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
      }
      /* 剥掉 wa-textarea 自带的底色/边框/阴影，只留纯文本输入区。 */
      .composer wa-textarea::part(base) {
        background: transparent;
        border: none;
        box-shadow: none;
        min-height: 36px;
        max-height: 200px;
        padding: 4px 8px;
      }
      /* 8.2: the native textarea lives in wa-textarea's shadow root, so the
         shared focus-visible rule can't reach it — ring the host instead.
         5.5 / design D5: composer focus is "your move", so the ring is the
         --signal accent, not the shared indigo focus ring. */
      .composer wa-textarea:focus-within {
        outline: 2px solid var(--sebas-signal);
        outline-offset: 2px;
        border-radius: var(--sebas-radius-sm);
      }
      /* Bottom toolbar: agent/binding/model on the left, send on the right. */
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
      .composer-bottom .binding {
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-faint);
      }
      .composer-bottom a {
        font-size: 0.78rem;
        color: var(--sebas-text-faint);
      }
      /* Toolbar selects (agent/model): slim selects inside the toolbar. */
      .composer-bottom .backend-select {
        font-size: 0.78rem;
        --wa-select-min-height: 24px;
        max-width: 220px;
      }
      /* add-composer-agent-binding：会话切换 chips（新会话/取消新建）。与
       * settings-link 同款弱化外观，避免在输入框旁喧宾夺主。 */
      .composer-bottom .mode-chip {
        border: 1px solid var(--sebas-border);
        background: none;
        border-radius: 999px;
        padding: 1px 10px;
        font: inherit;
        font-size: 0.72rem;
        color: var(--sebas-text-faint);
        cursor: pointer;
        transition: color var(--sebas-dur) var(--sebas-ease);
      }
      .composer-bottom .mode-chip:hover {
        color: var(--sebas-text-bright);
      }
      .composer-bottom .mode-chip:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      /* IA v2：settings → 打开居中设置弹窗（冒泡 open-settings 事件，由
       * app-shell 监听）；按钮外观与原链接一致。 */
      .composer-bottom .settings-link {
        border: none;
        background: none;
        padding: 0;
        font: inherit;
        font-size: 0.78rem;
        color: var(--sebas-text-faint);
        cursor: pointer;
        transition: color var(--sebas-dur) var(--sebas-ease);
      }
      .composer-bottom .settings-link:hover {
        color: var(--sebas-text-bright);
      }
      .composer-bottom .settings-link:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
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
        transition: opacity var(--sebas-dur) var(--sebas-ease);
      }
      .send-button:disabled {
        opacity: 0.35;
        cursor: not-allowed;
      }
      .send-button:hover:enabled {
        filter: brightness(1.05);
      }
      .divider {
        border: none;
        border-top: 1px solid var(--sebas-border);
        margin: var(--sebas-space-4) 0;
      }
    `,
  ]

  private reloadModelsBound = (): void => void this.loadModelOptions()

  connectedCallback(): void {
    super.connectedCallback()
    void this.loadReachability()
    void this.loadAgents()
    void this.loadModelOptions()
    // defaults/catalog 变更（管理页 set/clear）即时反映到选择器。
    window.addEventListener('sebas:refetch', this.reloadModelsBound)
    // Reachability 只在挂载时求值一次会让断连横幅永不恢复（core 回来后
    // composer 仍被禁用）——周期性重查，横幅与禁用态随真实状态翻转。
    this.reachabilityTimer = window.setInterval(() => {
      void this.loadReachability()
    }, WORKBENCH_REACHABILITY_POLL_MS)
  }

  disconnectedCallback(): void {
    window.removeEventListener('sebas:refetch', this.reloadModelsBound)
    super.disconnectedCallback()
    if (this.reachabilityTimer !== undefined) {
      window.clearInterval(this.reachabilityTimer)
      this.reachabilityTimer = undefined
    }
  }

  protected updated(changed: PropertyValues): void {
    // 聚焦会话变了（切换/关闭/新建跳转）——显式的"新会话"请求随之作废，
    // 让 composer 回到与新聚焦会话匹配的跟随模式。
    if (changed.has('sessionKey')) this.createRequested = false
    // 进入创建模式时重取模型数据源——defaults/catalog 可能刚在管理页设置过。
    if (changed.has('createRequested') && this.createRequested) void this.loadModelOptions()
    // 项目切换（D5）：预选该项目记住的 default_agent；无记录保持现选。
    if (changed.has('projectDefaultAgent') && this.projectDefaultAgent) {
      this.agent = this.projectDefaultAgent
    }
  }

  /**
   * 创建模式的模型目录（workbench-conversation-view 4.2/4.3/4.4，design
   * D7/D8）：BFF 读 Settings 目录（/router/api/providers 的 models）+
   * defaults（/router/api/defaults），adapter 规整成两级结构。读取失败或
   * 目录为空 → `catalogUnavailable`（显式「目录不可用」，不显示空列表）。
   * 会话内模型面与此无关——跟随模式只看聚焦会话自己的 `sessionModels`
   * （acp-model-selection：切换要发给该会话的执行体，目录它未必认）。
   */
  private async loadModelOptions(): Promise<void> {
    try {
      const [providers, defaults] = await Promise.all([
        api.routerProviders(),
        api.routerDefaults().catch(() => null),
      ])
      const catalog = toModelCatalog(providers.providers, defaults)
      if (catalog.pairs.length === 0) {
        // 目录为空 = 没有可选项：显式不可用，而非空下拉。
        this.catalog = catalog
        this.catalogUnavailable = true
        this.model = null
        this.selectedProvider = null
        return
      }
      this.catalog = catalog
      this.catalogUnavailable = false
      // 预选：配置的 default provider / default model 在目录内才用（不伪造
      // 选项）；否则取目录第一对。
      const provider =
        catalog.defaultProvider && catalog.pairs.some((p) => p.provider === catalog.defaultProvider)
          ? catalog.defaultProvider
          : catalog.pairs[0]!.provider
      this.selectedProvider = provider
      const models = this.modelsFor(provider)
      const wanted =
        catalog.defaultProvider === provider && catalog.defaultModel !== null
          ? catalog.defaultModel
          : null
      this.model = wanted && models.includes(wanted) ? wanted : (models[0] ?? null)
    } catch {
      // 目录不可得（providers 读取失败——core 状态库离线）：显式不可用。
      this.catalog = null
      this.model = null
      this.selectedProvider = null
      this.catalogUnavailable = true
    }
  }

  /** 一级选定的 provider 下的模型 id 列表（保持 payload 顺序）。 */
  private modelsFor(provider: string): string[] {
    return this.catalog?.pairs.filter((p) => p.provider === provider).map((p) => p.model) ?? []
  }

  /** 两级目录去重后的 provider 列表。 */
  private get catalogProviders(): string[] {
    const out: string[] = []
    for (const p of this.catalog?.pairs ?? []) {
      if (!out.includes(p.provider)) out.push(p.provider)
    }
    return out
  }

  private async loadAgents(): Promise<void> {
    try {
      const data = await api.agents()
      this.agents = data.agents
      // 预选（D5 兜底）：保持现选；无现选时取首个可达 agent。
      if (!this.agent || !this.agents.some((a) => a.id === this.agent)) {
        this.agent = data.agents.find((a) => a.reachable)?.id ?? ''
      }
    } catch {
      // catalog 不可达：下拉如实降级（禁用 + 提示），绝不伪造选项。
      this.agents = []
      this.agent = ''
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
      void data
      void this.loadAgents()
    } catch {
      /* add-webui-allowed-roots D6：summary 请求本身失败（服务进程死亡 /
       * 网络故障）与 reachability.ok = false 同款对待——进入不可达态禁用
       * 提交门，如实呈现而不是放行一次注定失败的提交。轮询恢复后自动
       * 解除。 */
      this.unreachable = { ok: false, cause: '无法获取服务状态（服务可能未运行）' }
    }
  }

  private disabled(): boolean {
    return this.sending || this.unreachable !== null
  }

  private async submit(): Promise<void> {
    if (this.disabled()) return
    const prompt = this.text.trim()
    if (!prompt) return
    if (this.isFollowMode) return void this.submitFollow(prompt)
    return void this.submitCreate(prompt)
  }

  /** 跟随模式：发给聚焦会话，绝不新建。 */
  private async submitFollow(prompt: string): Promise<void> {
    const key = this.sessionKey
    if (!key) return
    this.sending = true
    this.error = null
    try {
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

  /** 创建模式：选定的 agent + 模型在这里定死进新会话。 */
  private async submitCreate(prompt: string): Promise<void> {
    this.sending = true
    this.error = null
    try {
      const { key } = await api.createSession({
        prompt,
        projectId: this.projectId,
        agent: this.agent,
        model: this.model,
      })
      this.text = ''
      this.dispatchEvent(
        new CustomEvent<{ key: string }>('composer-created', {
          detail: { key },
          bubbles: true,
          composed: true,
        }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.sending = false
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
   * display name via /api/agent-kinds; unknown/unreachable kinds fall back
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

  private renderBinding() {
    if (this.projectDir) {
      const tail = this.projectDir.split('/').filter(Boolean).pop() ?? this.projectDir
      return html`<span class="binding">→ ${tail}</span>`
    }
    return html`<span class="binding">→ inbox</span>`
  }

  /**
   * "settings →" no longer navigates (the retired /settings route redirects
   * to /); it opens the shell's centered settings modal by dispatching a
   * bubbling composed event that app-shell listens for.
   */
  private openSettings(): void {
    this.dispatchEvent(new CustomEvent('open-settings', { bubbles: true, composed: true }))
  }

  render() {
    const disabled = this.disabled()
    const follow = this.isFollowMode
    // 跟随模式：会话 available_models（D8——会话已存在让位给会话面）。
    // 创建模式：两级 Settings 目录（4.3）。
    const sessionModelList = this.sessionModels
    const providerList = this.catalogProviders
    const providerModels = this.selectedProvider ? this.modelsFor(this.selectedProvider) : []
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
            <div class="callout callout-error" role="alert">
              ${icon('alert')}<span>${this.error}</span>
            </div>
          `
        : nothing}
      <div class="composer">
        <wa-textarea
          placeholder=${follow ? 'Ask for follow-up changes…' : 'Message the agent…'}
          aria-label="Message"
          resize="auto"
          ?disabled=${disabled}
          .value=${this.text}
          @input=${(e: Event) => (this.text = (e.target as HTMLTextAreaElement).value)}
          @keydown=${(e: KeyboardEvent) => {
            // 回车直接发送；Shift+Enter 换行；IME 组词中的回车不触发发送。
            if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
              e.preventDefault()
              void this.submit()
            }
          }}
        ></wa-textarea>
        <div class="composer-bottom">
          <div class="left-tools">
            ${follow
              ? html`<span
                  class="label"
                  data-testid="agent-lock"
                  title="Agent is immutable — chosen when the session was created"
                  >🔒 ${this.agentLabel()}</span
                >`
              : html`
                  ${this.providerLabel
                    ? html`<span class="label">${this.providerLabel}</span>`
                    : html`<span class="label placeholder">· · ·</span>`}
                  ${this.renderBinding()}
                `}
            ${follow
              ? sessionModelList.length > 0
                ? html`<wa-select
                    class="backend-select model-select"
                    aria-label="Model"
                    value=${this.currentModel ?? ''}
                    ?disabled=${disabled || this.modelSwitching}
                    @change=${(e: Event) => {
                      const v = (e.target as HTMLSelectElement).value || null
                      if (v) void this.switchModel(v)
                    }}
                  >
                    ${sessionModelList.map((m) => html`<wa-option value=${m}>${m}</wa-option>`)}
                  </wa-select>`
                : nothing
              : this.catalogUnavailable
                ? html`<span
                    class="label placeholder"
                    title="Configure a provider with models in Settings → Models"
                    role="status"
                    data-testid="catalog-unavailable"
                    >model catalog unavailable</span
                  >`
                : html`
                    <wa-select
                      class="backend-select model-select provider-select"
                      aria-label="Provider"
                      value=${this.selectedProvider ?? ''}
                      ?disabled=${disabled}
                      data-testid="provider-select"
                      @change=${(e: Event) => {
                        const v = (e.target as HTMLSelectElement).value
                        this.selectedProvider = v
                        const models = this.modelsFor(v)
                        this.model = models[0] ?? null
                      }}
                    >
                      ${providerList.map((p) => html`<wa-option value=${p}>${p}</wa-option>`)}
                    </wa-select>
                    <wa-select
                      class="backend-select model-select"
                      aria-label="Model"
                      value=${this.model ?? ''}
                      ?disabled=${disabled || providerModels.length === 0}
                      data-testid="model-select"
                      @change=${(e: Event) => {
                        this.model = (e.target as HTMLSelectElement).value || null
                      }}
                    >
                      ${providerModels.map((m) => html`<wa-option value=${m}>${m}</wa-option>`)}
                    </wa-select>
                  `}
            ${follow
              ? nothing
              : html`<wa-select
                  class="backend-select"
                  aria-label="Agent"
                  value=${this.agent}
                  ?disabled=${disabled}
                  @change=${(e: Event) => {
                    this.agent = (e.target as HTMLInputElement).value
                  }}
                >
                  ${this.agents.length === 0
                    ? html`<wa-option value="" disabled>agent catalog 不可用</wa-option>`
                    : nothing}
                  ${this.agents.map((a) =>
                    a.id === 'native' && !a.reachable
                      ? html`<wa-option value=${a.id} disabled
                          >${a.display} (unavailable: ${a.cause ?? 'unreachable'})</wa-option
                        >`
                      : html`<wa-option value=${a.id} ?disabled=${!a.reachable}
                          >${a.reachable ? a.display : `${a.display} (unavailable: ${a.cause ?? ''})`}</wa-option
                        >`,
                  )}
                </wa-select>`}
            ${this.sessionKey !== null
              ? html`<button
                  class="mode-chip"
                  type="button"
                  @click=${() => (this.createRequested = !this.createRequested)}
                >
                  ${follow ? '+ new session' : 'cancel'}
                </button>`
              : nothing}
          </div>
          <div class="right-tools">
            <button
              class="settings-link"
              type="button"
              aria-haspopup="dialog"
              @click=${this.openSettings}
            >
              settings →
            </button>
            <button
              class="send-button"
              aria-label="Send"
              ?disabled=${disabled}
              @click=${() => void this.submit()}
            >
              ${icon('forward', 14)}
            </button>
          </div>
        </div>
      </div>
      <hr class="divider" />
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-workbench-composer': SebasWorkbenchComposer
  }
}
