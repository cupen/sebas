/**
 * Session detail: status header (badge + ids + actions), transcript card
 * (markdown via the sanitize pipeline), composer pinned at the bottom
 * (send → clear), switch/focus, close with confirmation. Loads by encoded
 * key from the route; the read also focuses the session server-side.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, ApiError, type SessionDetail as Detail } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { navigate } from '../router.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '../components/status-badge.js'
import '../components/review-card.js'
import './transcript-view.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'

@customElement('sebas-session-detail')
export class SebasSessionDetail extends LitElement {
  @property() key = ''
  @state() private data: Detail | null = null
  @state() private error = ''
  @state() private message = ''
  @state() private sending = false
  @state() private confirmClose = false
  /** 中程切换模型（add-acp-model-selection）：非空 = 请求已发出，等事件回流。 */
  @state() private modelSwitching = false
  private unsubscribe?: () => void

  static styles = [
    viewStyles,
    css`
      /* 满幅面板（预览原型 workbench 同款）：宿主随 outlet 拉伸，
         状态头钉顶、transcript 吃满余高、composer 钉底。 */
      :host {
        display: flex;
        flex-direction: column;
        flex: 1;
        min-height: 0;
        min-width: 0;
      }
      .detail {
        display: flex;
        flex-direction: column;
        flex: 1;
        min-height: 0;
      }
      .head {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        flex-shrink: 0;
        padding: var(--sebas-space-3) var(--sebas-space-5);
        border-left: 3px solid var(--sebas-status-dormant);
        border-bottom: 1px solid var(--sebas-border);
        background: var(--sebas-surface);
        border-radius: 0;
        box-shadow: none;
        transition: border-color var(--sebas-dur) var(--sebas-ease);
      }
      .head[data-status='starting'] {
        border-left-color: var(--sebas-status-starting);
      }
      .head[data-status='queued'] {
        border-left-color: var(--sebas-status-queued);
      }
      .head[data-status='working'] {
        border-left-color: var(--sebas-status-working);
      }
      .head[data-status='done'] {
        border-left-color: var(--sebas-status-done);
      }
      .head[data-status='failed'] {
        border-left-color: var(--sebas-status-failed);
      }
      .head[data-status='dormant'] {
        border-left-color: var(--sebas-status-dormant);
      }
      .head .ident {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
      }
      .head .chat {
        font-family: var(--sebas-font-mono);
        font-size: 0.95rem;
        color: var(--sebas-text-bright);
        overflow-wrap: anywhere;
      }
      .head .chat .dim {
        color: var(--sebas-text-faint);
      }
      .head .meta {
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
        color: var(--sebas-text-dim);
        font-size: 0.78rem;
        font-variant-numeric: tabular-nums;
      }
      /* 中程模型选择器：meta 行内的紧凑下拉（add-acp-model-selection）。 */
      .head .model-pick {
        display: inline-flex;
        align-items: center;
      }
      .head .model-select {
        --wa-select-min-height: 24px;
        font-size: 0.75rem;
        max-width: 260px;
      }
      .head .actions {
        margin-left: auto;
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
      }
      .head .actions a {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 0.85rem;
      }
      /* The original prompt, quoted. */
      .prompt {
        margin: 0;
        padding: var(--sebas-space-3) var(--sebas-space-4);
        border-left: 3px solid var(--sebas-border-strong);
        border-radius: 0 var(--sebas-radius-md) var(--sebas-radius-md) 0;
        background: var(--sebas-surface-2);
        color: var(--sebas-text-dim);
        font-size: 0.9rem;
      }
      .prompt .who {
        display: block;
        font-size: 0.7rem;
        font-weight: 650;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--sebas-text-faint);
        margin-bottom: var(--sebas-space-1);
      }
      /* Transcript: 满幅面板区——flex 吃满余高，滚动由内层
         <sebas-transcript-view fill> 的 .scroll 负责（单一滚动区），
         composer 钉在底部。 */
      .transcript {
        flex: 1 1 auto;
        min-height: 0;
        display: flex;
        flex-direction: column;
      }
      .transcript .entry {
        padding: var(--sebas-space-2) 0;
      }
      .transcript .entry + .entry {
        border-top: 1px solid var(--sebas-border);
      }
      .body :is(p, pre, ul, ol, h1, h2, h3, h4) {
        overflow-wrap: break-word;
      }
      .body :first-child {
        margin-top: 0;
      }
      .body :last-child {
        margin-bottom: 0;
      }
      .body h1,
      .body h2,
      .body h3 {
        color: var(--sebas-text-bright);
        letter-spacing: -0.01em;
      }
      .body a {
        text-decoration: underline;
        text-underline-offset: 3px;
      }
      .body pre {
        background: var(--sebas-well);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-md);
        padding: var(--sebas-space-3);
        overflow-x: auto;
        font-family: var(--sebas-font-mono);
        font-size: 0.82rem;
        line-height: 1.55;
      }
      .body code {
        font-family: var(--sebas-font-mono);
        font-size: 0.88em;
      }
      .body :not(pre) > code {
        background: var(--sebas-surface-3);
        border-radius: var(--sebas-radius-sm);
        padding: 1px 5px;
      }
      .body blockquote {
        margin: 0.5em 0;
        padding: 0.1em 1em;
        border-left: 3px solid var(--sebas-border-strong);
        color: var(--sebas-text-dim);
      }
      /* Minimal highlight.js palette (no theme download). */
      .body .hljs-keyword,
      .body .hljs-selector-tag,
      .body .hljs-literal {
        color: #c792ea;
      }
      .body .hljs-string,
      .body .hljs-attr {
        color: #a5d6a7;
      }
      .body .hljs-number,
      .body .hljs-symbol {
        color: #f2b04e;
      }
      .body .hljs-title,
      .body .hljs-name,
      .body .hljs-function {
        color: #82aaff;
      }
      .body .hljs-comment,
      .body .hljs-quote {
        color: #5f6a80;
        font-style: italic;
      }
      .body .hljs-built_in,
      .body .hljs-type {
        color: #38d1dd;
      }
      /* Composer 底座：钉在面板底部的通栏（预览原型 composer-area 同款），
         内层 .composer-shell 才是 18px 圆角 shell。 */
      .composer {
        flex-shrink: 0;
        border-top: 1px solid var(--sebas-border);
        background: var(--sebas-bg);
        padding: 0 var(--sebas-space-5) var(--sebas-space-4);
      }
      .composer-shell {
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
         shared focus-visible rule can't reach it — ring the host instead. */
      .composer wa-textarea:focus-within {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
        border-radius: var(--sebas-radius-sm);
      }
      .composer-bottom {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        gap: var(--sebas-space-2);
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
      .dialog-body {
        margin: 0;
        color: var(--sebas-text);
        line-height: 1.55;
      }
      @media (max-width: 640px) {
        .head .actions {
          margin-left: 0;
          width: 100%;
        }
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.refetch()
    this.unsubscribe = sharedWs.subscribe(() => this.refetch())
    window.addEventListener('sebas:refetch', this.refetch)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetch)
    super.disconnectedCallback()
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    // Route parameter change → reload the view.
    if (changed.has('key') && this.key) this.refetch()
  }

  protected updated(changed: Map<string, unknown>): void {
    super.updated(changed)
    // Live streaming scroll is owned by `<sebas-transcript-view>` now —
    // its seen-boundary seam knows whether the reader is currently
    // looking at the bottom and only sticks when they are.
  }

  private refetch = (): void => {
    if (!this.key) return
    api
      .session(this.key)
      .then((d) => {
        this.data = d
        this.error = ''
      })
      .catch((e) => {
        if (e instanceof ApiError && e.status === 404) {
          this.error = 'Session not found (it may have been closed).'
          this.data = null
        } else {
          this.error = String(e)
        }
      })
  }

  private async send(): Promise<void> {
    if (!this.message.trim() || this.sending) return
    this.sending = true
    try {
      await api.sendMessage(this.key, this.message.trim())
      this.message = '' // success → clear the composer
      this.refetch()
    } catch (e) {
      this.error = String(e)
    } finally {
      this.sending = false
    }
  }

  /**
   * 中程切换模型（add-acp-model-selection 2.3）：把选择送后端 → 驱动发
   * `session/set_config_option{configId:"model"}`。wire 层失败（agent 拒绝
   * 无效模型）经非 terminal Error 事件回流，这里显示错误；快照的
   * `current_model` 在 `ModelChanged` 到达后由 refetch 刷新。
   */
  private async setModel(modelId: string): Promise<void> {
    if (!this.data || modelId === this.data.current_model || this.modelSwitching) return
    this.modelSwitching = true
    this.error = ''
    try {
      await api.setSessionModel(this.key, modelId)
      // 命令已送达驱动；连刷两次以捕捉 ModelChanged 之后的快照更新。
      await new Promise((r) => setTimeout(r, 400))
      this.refetch()
    } catch (e) {
      this.error = String(e)
    } finally {
      this.modelSwitching = false
    }
  }

  private async doClose(): Promise<void> {
    this.confirmClose = false
    try {
      const { active_session_key } = await api.closeSession(this.key)
      navigate(active_session_key ? `/sessions/${active_session_key}` : '/sessions')
    } catch (e) {
      this.error = String(e)
    }
  }

  render() {
    if (this.error)
      return html`
        <div class="callout callout-error" role="alert">${icon('alert')}<span>${this.error}</span></div>
        <!-- 会话死亡（如 spawn 后立即退出）不该把操作员困死在本页：
             给一条与 brand 链接同模式的返回工作台出口。 -->
        <p style="margin: var(--sebas-space-4) 0 0;">
          <a href="/" style="color: var(--sebas-text-faint); font-size: 0.85rem;"
            >← Back to workbench</a
          >
        </p>
      `
    if (!this.data)
      return html`
        <div class="panel" style="padding: var(--sebas-space-4)">
          ${[0, 1, 2].map(
            () => html`
              <div class="skel-row">
                <div class="skel skel-line" style="width:30%"></div>
                <div class="skel skel-line" style="width:55%"></div>
              </div>
            `,
          )}
        </div>
      `
    const d = this.data
    return html`
      <div class="detail">
        <div class="head" data-status=${d.status_slug}>
          <sebas-status-badge
            slug=${d.status_slug}
            label=${d.status_label}
            glyph=${d.status_glyph}
          ></sebas-status-badge>
          <div class="ident">
            <span class="chat"
              >${d.chat_id}${d.thread_id
                ? html`<span class="dim"> · ${d.thread_id}</span>`
                : nothing}</span
            >
            <span class="meta">
              ${d.session_id
                ? html`<span class="mono" title=${d.session_id}>${d.session_id.slice(0, 12)}</span>`
                : nothing}
              <!-- add-composer-agent-binding：agent 建会话时定死——只读展示
                   （null = 默认 kind）。 -->
              <span class="mono" title="This session's agent is fixed — chosen when it was created"
                >${d.agent_kind ?? 'acp · default'}</span
              >
              <span>last active ${d.last_active}</span>
              ${d.available_models && d.available_models.length > 0
                ? html`<span class="model-pick">
                    <!-- Web Awesome 3.x 派发标准 change 事件（不派发 wa-change）。 -->
                    <wa-select
                      class="model-select"
                      size="xs"
                      hoist
                      value=${d.current_model ?? ''}
                      ?disabled=${this.modelSwitching}
                      aria-label="Session model"
                      @change=${(e: Event) => {
                        const v = (e as unknown as { target: { value: string } }).target.value
                        if (v) void this.setModel(v)
                      }}
                    >
                      ${d.available_models.map(
                        (m) => html`<wa-option value=${m}>${m}</wa-option>`,
                      )}
                    </wa-select>
                  </span>`
                : nothing}
            </span>
          </div>
          <div class="actions">
            <a href="/sessions">${icon('back', 14)} All sessions</a>
            <wa-button
              size="s"
              variant="danger"
              appearance="outlined"
              aria-label="Close this session"
              @click=${() => (this.confirmClose = true)}
              >Close</wa-button
            >
          </div>
        </div>

        ${d.user_prompt
          ? html`<blockquote class="prompt">
              <span class="who">Original prompt</span>“${d.user_prompt}”
            </blockquote>`
          : nothing}

        <!-- Gated tool calls for this session: a card appears the moment
             the kernel asks for permission and disappears once answered. -->
        <sebas-review-cards .sessionKey=${d.encoded_key}></sebas-review-cards>

        <section
          class="transcript"
          aria-label="Session transcript"
        >
          ${d.body.length === 0
            ? html`
                <div class="empty">
                  <span class="glyph">${icon('message', 20)}</span>
                  <span class="title">Nothing yet</span>
                  <p class="hint">The agent has not produced output — say hello below.</p>
                </div>
              `
            : html`<sebas-transcript-view
                fill
                .entries=${d.body}
                sessionKey=${d.encoded_key}
              ></sebas-transcript-view>`}
        </section>

        <div class="composer">
          <div class="composer-shell">
            <wa-textarea
              placeholder="Message the agent…"
              aria-label="Message"
              resize="auto"
              value=${this.message}
              @input=${(e: Event) => (this.message = (e.target as HTMLTextAreaElement).value)}
              @keydown=${(e: KeyboardEvent) => {
                // 回车直接发送；Shift+Enter 换行；IME 组词中的回车不触发发送。
                if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
                  e.preventDefault()
                  void this.send()
                }
              }}
            ></wa-textarea>
            <div class="composer-bottom">
              <button
                class="send-button"
                aria-label="Send"
                ?disabled=${this.sending}
                @click=${this.send}
              >
                ${icon('forward', 14)}
              </button>
            </div>
          </div>
        </div>

        <wa-dialog label="Close session" ?open=${this.confirmClose}>
          <p class="dialog-body">
            Closing will terminate the agent child process and clear this chat's
            permission allowlist. This cannot be undone.
          </p>
          <wa-button slot="footer" appearance="plain" @click=${() => (this.confirmClose = false)}
            >Cancel</wa-button
          >
          <wa-button slot="footer" variant="danger" @click=${this.doClose}>Close session</wa-button>
        </wa-dialog>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-session-detail': SebasSessionDetail
  }
}
