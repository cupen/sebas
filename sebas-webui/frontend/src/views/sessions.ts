/**
 * Sessions list: create form, card grid with per-status accents, focus/
 * switch, close (with confirmation), live updates over the shared WebSocket.
 */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, ApiError, type AgentKindInfo, type BackendHint, type SessionList } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { navigate } from '../router.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '../components/status-badge.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

@customElement('sebas-sessions')
export class SebasSessions extends LitElement {
  @state() private data: SessionList | null = null
  @state() private error = ''
  @state() private prompt = ''
  @state() private backend: BackendHint = 'acp'
  @state() private kinds: AgentKindInfo[] = []
  @state() private creating = false
  @state() private closeTarget: string | null = null
  /** Execution-backend hint forwarded with the spawn request. */
  @state() private backend: BackendHint = 'acp'
  private unsubscribe?: () => void

  static styles = [
    viewStyles,
    css`
      .chips {
        display: flex;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        margin-bottom: var(--sebas-space-5);
      }
      .composer {
        padding: var(--sebas-space-4);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
      }
      .composer .backend-select {
        font-size: 0.8rem;
        --wa-select-min-height: 32px;
        max-width: 220px;
      }
      .composer .composer-label {
        color: var(--sebas-text-dim);
        font-size: 0.8rem;
        font-weight: 550;
        text-transform: uppercase;
        letter-spacing: 0.07em;
      }
      .composer .row {
        display: flex;
        gap: var(--sebas-space-3);
        align-items: flex-start;
        flex-wrap: wrap;
      }
      .composer wa-input {
        flex: 1;
        min-width: 220px;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
        gap: var(--sebas-space-3);
      }
      .scard {
        position: relative;
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-3);
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
        transition:
          border-color var(--sebas-dur) var(--sebas-ease),
          transform var(--sebas-dur) var(--sebas-ease),
          box-shadow var(--sebas-dur) var(--sebas-ease);
      }
      .scard:hover {
        border-color: var(--sebas-border-strong);
        transform: translateY(-1px);
        box-shadow: var(--sebas-shadow-2);
      }
      /* Status accent line on the card's top edge (data-status hook). */
      .scard::before {
        content: '';
        position: absolute;
        top: -1px;
        left: 12px;
        right: 12px;
        height: 2px;
        border-radius: var(--sebas-radius-full);
        background: var(--sebas-status-dormant);
        opacity: 0.85;
      }
      .scard[data-status='starting']::before {
        background: var(--sebas-status-starting);
      }
      .scard[data-status='queued']::before {
        background: var(--sebas-status-queued);
      }
      .scard[data-status='working']::before {
        background: var(--sebas-status-working);
      }
      .scard[data-status='done']::before {
        background: var(--sebas-status-done);
      }
      .scard[data-status='failed']::before {
        background: var(--sebas-status-failed);
      }
      .scard[data-status='dormant']::before {
        background: var(--sebas-status-dormant);
      }
      .scard .top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--sebas-space-2);
      }
      .scard .chat {
        font-family: var(--sebas-font-mono);
        font-size: 0.88rem;
        overflow-wrap: anywhere;
      }
      .scard .meta {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        color: var(--sebas-text-dim);
        font-size: 0.8rem;
        font-variant-numeric: tabular-nums;
      }
      .scard .meta .sep {
        color: var(--sebas-text-faint);
      }
      .scard .foot {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        margin-top: auto;
        padding-top: var(--sebas-space-2);
        border-top: 1px solid var(--sebas-border);
      }
      .focused-chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 2px 9px;
        border-radius: var(--sebas-radius-full);
        background: var(--sebas-accent-soft);
        border: 1px solid var(--sebas-accent-border);
        color: var(--sebas-accent);
        font-size: 0.72rem;
        font-weight: 600;
        letter-spacing: 0.02em;
      }
      .foot .spacer {
        flex: 1;
      }
      .dialog-body {
        margin: 0;
        color: var(--sebas-text);
        line-height: 1.55;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.refetch()
    this.loadKinds()
    this.unsubscribe = sharedWs.subscribe(() => this.refetch())
    window.addEventListener('sebas:refetch', this.refetch)
  }

  private loadKinds(): void {
    api
      .agentKinds()
      .then((d) => {
        this.kinds = d.kinds.filter((k) => k.reachable)
      })
      .catch(() => {
        this.kinds = []
      })
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetch)
    super.disconnectedCallback()
  }

  private refetch = (): void => {
    api
      .sessions()
      .then((d) => {
        this.data = d
        this.error = ''
      })
      .catch((e) => {
        this.error = String(e)
      })
  }

  private async create(e: Event): Promise<void> {
    e.preventDefault()
    if (!this.prompt.trim() || this.creating) return
    this.creating = true
    try {
      const { key } = await api.createSession(this.prompt.trim(), null, this.backend)
      this.prompt = ''
      navigate(`/sessions/${key}`)
    } catch (err) {
      this.error = String(err)
    } finally {
      this.creating = false
    }
  }

  private async switchTo(encodedKey: string): Promise<void> {
    try {
      const { redirect } = await api.switchSession(encodedKey)
      navigate(redirect)
    } catch (err) {
      this.error = String(err)
    }
  }

  private async confirmClose(): Promise<void> {
    const target = this.closeTarget
    this.closeTarget = null
    if (!target) return
    try {
      await api.closeSession(target)
      this.refetch()
    } catch (err) {
      if (err instanceof ApiError && err.status === 404) this.refetch()
      else this.error = String(err)
    }
  }

  private renderLoading() {
    return html`
      <div class="grid">
        ${[0, 1, 2, 3, 4, 5].map(
          () => html`
            <div class="scard">
              <div class="skel skel-line" style="width:45%"></div>
              <div class="skel skel-line" style="width:70%"></div>
              <div class="skel skel-line" style="width:30%"></div>
            </div>
          `,
        )}
      </div>
    `
  }

  render() {
    if (this.error)
      return html`
        <div class="callout callout-error" role="alert">
          ${icon('alert')}<span>Failed to load: ${this.error}</span>
        </div>
      `
    if (!this.data) return this.renderLoading()
    const d = this.data
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Sessions</h1>
          <p class="page-sub">Create, focus and close agent sessions.</p>
        </div>
      </header>

      <div class="chips">
        <span class="chip"><span class="dot" style="background: var(--sebas-status-working)"></span><b>${d.active_count}</b> active</span>
        <span class="chip"><span class="dot" style="background: var(--sebas-status-dormant)"></span><b>${d.dormant_count}</b> dormant</span>
        <span class="chip"><span class="dot" style="background: var(--sebas-status-starting)"></span><b>${d.spawning_count}</b> spawning</span>
        <span class="chip"><b>${d.total_sessions}</b> total</span>
      </div>

      <form class="panel composer" @submit=${this.create}>
        <span class="composer-label">Start a new session</span>
        <div class="row">
          <wa-input
            placeholder="Describe the task for a new agent session…"
            aria-label="New session prompt"
            value=${this.prompt}
            @input=${(e: Event) => (this.prompt = (e.target as HTMLInputElement).value)}
          ></wa-input>
          <wa-select
            class="backend-select"
            aria-label="Execution backend"
            value=${this.backend}
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
            <wa-option value="native">native · built-in kernel</wa-option>
          </wa-select>
          <wa-button variant="brand" appearance="accent" ?loading=${this.creating} type="submit"
            >New session</wa-button
          >
        </div>
      </form>

      ${d.recent_sessions.length === 0
        ? html`
            <section class="panel" style="margin-top: var(--sebas-space-4)">
              <div class="empty">
                <span class="glyph">${icon('sessions', 20)}</span>
                <span class="title">Nothing running yet</span>
                <p class="hint">
                  Start your first session above — describe the task and the agent picks it up.
                </p>
              </div>
            </section>
          `
        : html`
            <div class="grid" style="margin-top: var(--sebas-space-4)">
              ${d.recent_sessions.map(
                (row) => html`
                  <article class="scard" data-status=${row.status_slug}>
                    <div class="top">
                      <a class="chat" href=${`/sessions/${row.encoded_key}`}>${row.chat_id}</a>
                      <sebas-status-badge
                        slug=${row.status_slug}
                        label=${row.status_label}
                        glyph=${row.status_glyph}
                      ></sebas-status-badge>
                    </div>
                    <div class="meta">
                      <span class="mono" title=${row.session_id ?? ''}
                        >${row.session_id_short ?? '—'}</span
                      >
                      <span class="sep">·</span>
                      <span>${row.last_active}</span>
                    </div>
                    <div class="foot">
                      ${row.is_active
                        ? html`<span class="focused-chip">focused</span>`
                        : html`<wa-button
                            size="s"
                            appearance="plain"
                            @click=${() => this.switchTo(row.encoded_key)}
                            >Focus</wa-button
                          >`}
                      <span class="spacer"></span>
                      <wa-button
                        size="s"
                        appearance="plain"
                        variant="danger"
                        aria-label=${`Close session ${row.chat_id}`}
                        @click=${() => (this.closeTarget = row.encoded_key)}
                        >Close</wa-button
                      >
                    </div>
                  </article>
                `,
              )}
            </div>
          `}

      <wa-dialog label="Close session" ?open=${this.closeTarget !== null}>
        <p class="dialog-body">
          Closing will terminate the agent child process and drop the session
          mapping. This cannot be undone.
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.closeTarget = null)}
          >Cancel</wa-button
        >
        <wa-button slot="footer" variant="danger" @click=${this.confirmClose}>Close session</wa-button>
      </wa-dialog>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-sessions': SebasSessions
  }
}
