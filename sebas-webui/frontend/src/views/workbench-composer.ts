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
 * the inbox) with the execution backend + model picked in the toolbar — the
 * only place an agent can be chosen, because after spawn the binding is
 * immutable.
 *
 * Reaches the agent-core reachability report from /api/summary to gate
 * submit when the core is offline (a submit would only bounce), re-polled
 * on a 5s interval so the banner and disabled state follow reality. A
 * transient submit error is surfaced inline via the shared `.callout-error`
 * style and the message text is preserved so the operator can retry.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type AgentKindInfo, type BackendHint } from '../api/client.js'
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
   * Currently-selected project path. When non-null, the new session is
   * bound to that directory; when null, the session is "inbox" and the
   * `project_dir` field is omitted on the wire.
   */
  @property({ attribute: false }) projectDir: string | null = null
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
  /** Execution-backend hint forwarded with the spawn request (creation). */
  @state() private backend: BackendHint = 'acp'
  /** Reachable third-party agent kinds for the create-session dropdown. */
  @state() private kinds: AgentKindInfo[] = []
  /** 创建会话时请求的模型 id（add-acp-model-selection）；仅当数据源（最近
   *  会话的可用模型列表）非空时显示下拉。 */
  @state() private model: string | null = null
  /** 模型下拉的数据源：最近一个暴露 available_models 的会话的模型列表。 */
  @state() private modelOptions: string[] = []
  /** Set when the agent core is unreachable; gates submit. */
  @state() private unreachable: { ok: false; cause: string } | null = null
  /**
   * wire-webui-sebas-agent-e2e: per-execution-body reachability from
   * `/api/summary.execution_bodies`. `native.ok=false` → 后端下拉中 "native"
   * 选项渲染为 disabled + cause（spec：不可用的执行体不应让操作员提交到
   * 才能发现的失败）。
   */
  @state() private nativeAvailability: { ok: boolean; cause?: string } | null = null
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

  connectedCallback(): void {
    super.connectedCallback()
    void this.loadReachability()
    void this.loadKinds()
    void this.loadModelOptions()
    // Reachability 只在挂载时求值一次会让断连横幅永不恢复（core 回来后
    // composer 仍被禁用）——周期性重查，横幅与禁用态随真实状态翻转。
    this.reachabilityTimer = window.setInterval(() => {
      void this.loadReachability()
    }, WORKBENCH_REACHABILITY_POLL_MS)
  }

  disconnectedCallback(): void {
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
  }

  /**
   * 模型下拉数据源（add-acp-model-selection D4）：创建会话表单的模型列表来自
   * 快照里最近一个暴露 `available_models` 的会话（agent 的 configOptions）。
   * 无模型选项的 agent（如 Claude）→ 空列表 → 不显示下拉、不报错。
   * （跟随模式不用这份借来的数据——它用聚焦会话自己的 `sessionModels`。）
   */
  private async loadModelOptions(): Promise<void> {
    try {
      const data = await api.sessions()
      const latest = data.recent_sessions
        .slice()
        .sort((a, b) => b.last_active_unix - a.last_active_unix)
        .find((r) => Array.isArray(r.available_models) && r.available_models.length > 0)
      if (latest && latest.available_models) {
        this.modelOptions = latest.available_models
        // 预选会话当前模型（若列表里含它），否则选第一项。
        this.model =
          latest.current_model && this.modelOptions.includes(latest.current_model)
            ? latest.current_model
            : (this.modelOptions[0] ?? null)
      } else {
        this.modelOptions = []
        this.model = null
      }
    } catch {
      this.modelOptions = []
      this.model = null
    }
  }

  private async loadKinds(): Promise<void> {
    try {
      const data = await api.agentKinds()
      this.kinds = data.kinds.filter((k) => k.reachable)
    } catch {
      // agent-kinds is advisory; a failure leaves the dropdown at default/native.
      this.kinds = []
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
      // wire-webui-sebas-agent-e2e：双执行体的逐体可用性。native 不可用时把后
      // 端下拉中的 "native" 选项渲染为 disabled + cause（spec：不可用执行体
      // 不应让操作员提交后才看到失败）；acp 不受 native 状态影响，整体
      // 提交门禁只看 reachability（core 可达性）。
      const bodies = data.execution_bodies
      const native = bodies?.find((b) => b.name === 'native')
      if (native) {
        this.nativeAvailability = native.ok
          ? { ok: true }
          : { ok: false, cause: native.cause ?? 'native backend unavailable' }
      } else {
        this.nativeAvailability = null
      }
    } catch {
      /* If summary itself fails, leave reachability null so the operator
       * can still try to submit (the server-side call will give a more
       * accurate error than a stale gate). */
      this.unreachable = null
      this.nativeAvailability = null
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
      const { key } = await api.createSession(prompt, this.projectDir, this.backend, this.model)
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
      const k = this.kinds.find((x) => x.slug === this.agentKind)
      return k?.name ?? this.agentKind
    }
    return 'acp · default'
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
    const modelList = follow ? this.sessionModels : this.modelOptions
    const modelValue = follow ? this.currentModel : this.model
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
                  title="This session's agent is fixed — chosen when it was created"
                  >${this.agentLabel()}</span
                >`
              : html`
                  ${this.providerLabel
                    ? html`<span class="label">${this.providerLabel}</span>`
                    : html`<span class="label placeholder">· · ·</span>`}
                  ${this.renderBinding()}
                `}
            ${modelList.length > 0
              ? html`<wa-select
                  class="backend-select model-select"
                  aria-label="Model"
                  value=${modelValue ?? ''}
                  ?disabled=${disabled || (follow && this.modelSwitching)}
                  @change=${(e: Event) => {
                    const v = (e.target as HTMLSelectElement).value || null
                    if (follow) {
                      if (v) void this.switchModel(v)
                    } else {
                      this.model = v
                    }
                  }}
                >
                  ${modelList.map((m) => html`<wa-option value=${m}>${m}</wa-option>`)}
                </wa-select>`
              : nothing}
            ${follow
              ? nothing
              : html`<wa-select
                  class="backend-select"
                  aria-label="Execution backend"
                  value=${this.backend}
                  ?disabled=${disabled}
                  @change=${(e: Event) => {
                    const value = (e.target as HTMLInputElement).value
                    if (value === 'acp' || value === 'native' || value.startsWith('acp:')) {
                      this.backend = value as BackendHint
                    }
                  }}
                >
                  <wa-option value="acp">acp · default kind</wa-option>
                  ${this.kinds.map(
                    (k) => html`<wa-option value=${`acp:${k.slug}`}>acp · ${k.name}</wa-option>`,
                  )}
                  ${this.nativeAvailability && !this.nativeAvailability.ok
                    ? html`<wa-option value="native" disabled
                        >native · built-in kernel (unavailable: ${this.nativeAvailability.cause ?? 'no provider credentials'})</wa-option
                      >`
                    : html`<wa-option value="native">native</wa-option>`}
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
