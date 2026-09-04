/**
 * Workbench composer: pinned at the top of the dashboard right pane. Lets
 * the operator spin up a new session from the workbench without leaving the
 * overview; binds the new session to the currently-selected project, or
 * routes it to the inbox when nothing is selected. Plain Enter sends
 * (Shift+Enter inserts a newline; an IME-composing Enter is ignored) via
 * the 28px accent send button.
 *
 * Reaches the agent-core reachability report from /api/summary to gate
 * submit when the core is offline (a submit would only bounce). A transient
 * submit error is surfaced inline via the shared `.callout-error` style and
 * the message text is preserved so the operator can retry.
 */

import { LitElement, css, html, nothing } from 'lit'
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

  @state() private text = ''
  @state() private sending = false
  @state() private error: string | null = null
  /** Execution-backend hint forwarded with the spawn request. */
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
  /** Reachability 轮询定时器（connectedCallback 启动，disconnectedCallback 清理）。 */
  private reachabilityTimer: number | undefined = undefined

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
      /* Bottom toolbar: provider label + binding + backend on the left, send
       * on the right — the loose text rows that used to sit above the card. */
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
      /* Backend picker: a slim select inside the toolbar. */
      .composer-bottom .backend-select {
        font-size: 0.78rem;
        --wa-select-min-height: 24px;
        max-width: 220px;
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

  /**
   * 模型下拉数据源（add-acp-model-selection D4）：创建会话表单的模型列表来自
   * 快照里最近一个暴露 `available_models` 的会话（agent 的 configOptions）。
   * 无模型选项的 agent（如 Claude）→ 空列表 → 不显示下拉、不报错。
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
    const placeholder = this.providerLabel ?? null
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
<wa-select
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
            : html`<wa-option value="native">native · built-in kernel</wa-option>`}
        </wa-select>
        <wa-textarea
          placeholder="Message the agent…"
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
            ${placeholder
              ? html`<span class="label">${placeholder}</span>`
              : html`<span class="label placeholder">· · ·</span>`}
            ${this.renderBinding()}
            ${this.modelOptions.length > 0
              ? html`<wa-select
                  class="backend-select model-select"
                  aria-label="Model"
                  value=${this.model ?? ''}
                  ?disabled=${disabled}
                  @change=${(e: Event) => {
                    this.model = (e.target as HTMLSelectElement).value || null
                  }}
                >
                  ${this.modelOptions.map(
                    (m) => html`<wa-option value=${m}>${m}</wa-option>`,
                  )}
                </wa-select>`
              : nothing}
            <wa-select
              class="backend-select"
              aria-label="Execution backend"
              value=${this.backend}
              ?disabled=${disabled}
              @change=${(e: Event) => {
                const value = (e.target as HTMLInputElement).value
                if (value === 'acp' || value === 'native') this.backend = value
              }}
            >
              <wa-option value="acp">acp</wa-option>
              ${this.nativeAvailability && !this.nativeAvailability.ok
                ? html`<wa-option value="native" disabled
                    >native (unavailable)</wa-option
                  >`
                : html`<wa-option value="native">native</wa-option>`}
            </wa-select>
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
