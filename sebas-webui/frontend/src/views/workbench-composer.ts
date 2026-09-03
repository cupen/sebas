/**
 * Workbench composer: pinned at the top of the dashboard right pane. Lets
 * the operator spin up a new session from the workbench without leaving the
 * overview; binds the new session to the currently-selected project, or
 * routes it to the inbox when nothing is selected.
 *
 * Reaches the agent-core reachability report from /api/summary to gate
 * submit when the core is offline (a submit would only bounce). A transient
 * submit error is surfaced inline via the shared `.callout-error` style and
 * the message text is preserved so the operator can retry.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type BackendHint } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

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
  /** Execution-backend hint forwarded with the spawn request. */
  @state() private backend: BackendHint = 'acp'
  @state() private error: string | null = null
  /** Set when the agent core is unreachable; gates submit. */
  @state() private unreachable: { ok: false; cause: string } | null = null

  static styles = [
    viewStyles,
    css`
      :host {
        display: block;
      }
      .provider-strip {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--sebas-space-3);
        color: var(--sebas-text-dim);
        font-size: 0.78rem;
        margin-bottom: var(--sebas-space-2);
      }
      .provider-strip .label {
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-dim);
      }
      .provider-strip .label.placeholder {
        color: var(--sebas-text-faint);
        letter-spacing: 0.15em;
      }
      .provider-strip a {
        font-size: 0.78rem;
        color: var(--sebas-text-faint);
      }
      .binding {
        display: block;
        font-family: var(--sebas-font-mono);
        font-size: 0.75rem;
        color: var(--sebas-text-faint);
        margin-bottom: var(--sebas-space-2);
      }
      /* Composer card: matches the dark-mode aesthetic of the
       * session-detail composer so the operator feels they're using the
       * same control in both places. */
      .composer {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-up);
        padding: var(--sebas-space-3);
        display: flex;
        gap: var(--sebas-space-3);
        align-items: flex-end;
      }
      .composer .backend-select {
        flex: 0 0 auto;
        width: 175px;
      }
      .composer wa-textarea {
        flex: 1;
      }
      /* 8.2: the native textarea lives in wa-textarea's shadow root, so the
         shared focus-visible rule can't reach it — ring the host instead. */
      .composer wa-textarea:focus-within {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
        border-radius: var(--sebas-radius-sm);
      }
      .composer .send-col {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
      }
      .composer kbd {
        font-family: var(--sebas-font-mono);
        font-size: 0.62rem;
        color: var(--sebas-text-faint);
        background: var(--sebas-surface-2);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-sm);
        padding: 1px 5px;
        white-space: nowrap;
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
  }

  private async loadReachability(): Promise<void> {
    try {
      const data = await api.summary()
      if (data.reachability && data.reachability.ok === false) {
        this.unreachable = { ok: false, cause: data.reachability.cause ?? 'core not connected' }
      } else {
        this.unreachable = null
      }
    } catch {
      /* If summary itself fails, leave reachability null so the operator
       * can still try to submit (the server-side call will give a more
       * accurate error than a stale gate). */
      this.unreachable = null
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
      const { key } = await api.createSession(prompt, this.projectDir, this.backend)
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

  render() {
    const disabled = this.disabled()
    const placeholder = this.providerLabel ?? null
    return html`
      <div class="provider-strip">
        ${placeholder
          ? html`<span class="label">${placeholder}</span>`
          : html`<span class="label placeholder">· · ·</span>`}
        <a href="/settings">settings →</a>
      </div>
      ${this.renderBinding()}
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
            if (value === 'acp' || value === 'native') this.backend = value
          }}
        >
          <wa-option value="acp">acp · Claude Code bridge</wa-option>
          <wa-option value="native">native · built-in kernel</wa-option>
        </wa-select>
        <wa-textarea
          placeholder="Message the agent…"
          aria-label="Message"
          resize="auto"
          ?disabled=${disabled}
          .value=${this.text}
          @input=${(e: Event) => (this.text = (e.target as HTMLTextAreaElement).value)}
          @keydown=${(e: KeyboardEvent) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void this.submit()
          }}
        ></wa-textarea>
        <div class="send-col">
          <wa-button
            variant="brand"
            appearance="accent"
            ?disabled=${disabled}
            ?loading=${this.sending}
            @click=${() => void this.submit()}
            >Send</wa-button
          >
          <kbd>⌘/Ctrl ⏎</kbd>
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
