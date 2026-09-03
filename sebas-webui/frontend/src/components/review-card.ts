/**
 * Review cards: the operator surface for gated tool calls. Subscribes to
 * the shared WebSocket; every `permission.requested` frame becomes a card
 * keyed by `request_id` (duplicate frames never create a second card).
 * Answering POSTs the decision to `/api/permissions/{request_id}/answer` —
 * success removes the card, a 404 marks it expired (the pending request is
 * gone server-side: answered, timed out or unknown), any other error keeps
 * the card retryable with the failure surfaced inline.
 *
 * When `sessionKey` is set only frames for that (encoded) session key are
 * rendered — the session-detail view passes its key; `null` (default)
 * renders every request, whatever session raised it.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, ApiError, type PermissionDecision } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { icon } from './icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'

interface ReviewCard {
  request_id: string
  session_id: string
  tool_name: string
  args: unknown
  reason: string
  /**
   * `pending` → buttons live; `answering` → POST in flight, buttons
   * disabled; `expired` → the server no longer holds this request, the
   * card is kept visible but inert.
   */
  state: 'pending' | 'answering' | 'expired'
  /** Last submit failure (transport/server), '' when none. */
  error: string
  /** Reason typed for the one-shot escalate decision. */
  escalateReason: string
}

@customElement('sebas-review-cards')
export class SebasReviewCards extends LitElement {
  /** Encoded session key to filter on; null renders every request. */
  @property({ attribute: false }) sessionKey: string | null = null

  @state() private cards: ReviewCard[] = []
  private unsubscribe?: () => void

  static styles = [
    viewStyles,
    css`
      :host {
        display: block;
      }
      .review-cards {
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-3);
      }
      .review-card {
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border-strong);
        border-left: 3px solid var(--sebas-status-working);
        border-radius: var(--sebas-radius-md);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
      }
      .review-card[data-state='expired'] {
        border-left-color: var(--sebas-status-dormant);
        opacity: 0.75;
      }
      .review-card .head {
        display: flex;
        align-items: baseline;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
      }
      .review-card .head svg {
        align-self: center;
        color: var(--sebas-status-working);
        flex: 0 0 auto;
      }
      .review-card[data-state='expired'] .head svg {
        color: var(--sebas-status-dormant);
      }
      .review-card .head .tool {
        font-family: var(--sebas-font-mono);
        font-weight: 650;
        color: var(--sebas-text-bright);
      }
      .review-card .head .why {
        color: var(--sebas-text-dim);
        font-size: 0.85rem;
      }
      .review-card .meta {
        display: flex;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        color: var(--sebas-text-faint);
        font-size: 0.72rem;
        overflow-wrap: anywhere;
      }
      /* The call's arguments: formatted JSON, internally scrollable. */
      .review-card .args {
        margin: 0;
        background: var(--sebas-well, var(--sebas-surface-2));
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-sm);
        padding: var(--sebas-space-2) var(--sebas-space-3);
        font-family: var(--sebas-font-mono);
        font-size: 0.78rem;
        line-height: 1.5;
        overflow-wrap: break-word;
        white-space: pre-wrap;
        max-height: 140px;
        overflow-y: auto;
      }
      .review-card .actions {
        display: flex;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        margin-top: var(--sebas-space-1);
      }
      .review-card .escalate {
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
      }
      .review-card .escalate wa-input {
        flex: 1;
        min-width: 180px;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.unsubscribe = sharedWs.subscribe((event) => {
      if (event.type !== 'permission.requested') return
      if (this.sessionKey && event.session_id !== this.sessionKey) return
      // Dedup by request_id: the frame is broadcast and can repeat after a
      // reconnect — one card per request, ever (an expired card stays in
      // the list precisely so a late repeat cannot resurrect it).
      if (this.cards.some((c) => c.request_id === event.request_id)) return
      this.cards = [
        ...this.cards,
        {
          request_id: event.request_id,
          session_id: event.session_id,
          tool_name: event.tool_name,
          args: event.args,
          reason: event.reason,
          state: 'pending',
          error: '',
          escalateReason: '',
        },
      ]
    })
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    super.disconnectedCallback()
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    // Switching the viewed session drops cards collected for the previous
    // one (a pending request re-announced later would rebuild its card).
    if (changed.has('sessionKey')) this.cards = []
  }

  private patch(requestId: string, patch: Partial<ReviewCard>): void {
    this.cards = this.cards.map((c) => (c.request_id === requestId ? { ...c, ...patch } : c))
  }

  private async answer(card: ReviewCard, decision: PermissionDecision): Promise<void> {
    if (card.state !== 'pending') return
    this.patch(card.request_id, { state: 'answering', error: '' })
    try {
      await api.answerPermission(card.request_id, decision)
      // Delivered: the card has done its job.
      this.cards = this.cards.filter((c) => c.request_id !== card.request_id)
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        // No pending request with that id — answered elsewhere, timed out
        // or unknown. Mark the card expired; it stays visible but inert.
        this.patch(card.request_id, { state: 'expired', error: '' })
      } else {
        // Transport/server hiccup: keep the card retryable.
        this.patch(card.request_id, { state: 'pending', error: String(e) })
      }
    }
  }

  private formatArgs(args: unknown): string {
    try {
      return JSON.stringify(args ?? {}, null, 2)
    } catch {
      return String(args)
    }
  }

  private renderCard(card: ReviewCard) {
    const busy = card.state !== 'pending'
    const escalateReason = card.escalateReason.trim()
    return html`
      <section class="review-card" data-request-id=${card.request_id} data-state=${card.state}>
        <header class="head">
          ${icon('shield', 16)}
          <span class="tool">${card.tool_name}</span>
          ${card.reason ? html`<span class="why">${card.reason}</span>` : nothing}
        </header>
        <div class="meta">
          <span class="session-id" title=${`session ${card.session_id}`}
            >session ${card.session_id}</span
          >
          <span class="request-id" title=${`request ${card.request_id}`}
            >request ${card.request_id}</span
          >
        </div>
        <pre class="args">${this.formatArgs(card.args)}</pre>
        ${card.state === 'expired'
          ? html`<div class="callout callout-warning" role="status">
              ${icon('alert')}<span
                >No longer pending — already answered, timed out or cleared.</span
              >
            </div>`
          : html`
              <div class="actions">
                <wa-button
                  size="s"
                  class="allow-once"
                  variant="brand"
                  appearance="accent"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'allow_once' })}
                  >Allow once</wa-button
                >
                <wa-button
                  size="s"
                  class="allow-session"
                  variant="success"
                  appearance="outlined"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'allow_session' })}
                  >Allow for session</wa-button
                >
                <wa-button
                  size="s"
                  class="deny"
                  variant="danger"
                  appearance="outlined"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'deny' })}
                  >Deny</wa-button
                >
              </div>
              <div class="escalate">
                <wa-input
                  class="escalate-reason"
                  size="s"
                  placeholder="Why raise this once? (escalate)"
                  aria-label="Escalation reason"
                  .value=${card.escalateReason}
                  ?disabled=${busy}
                  @input=${(e: Event) =>
                    this.patch(card.request_id, {
                      escalateReason: (e.target as HTMLInputElement).value,
                    })}
                ></wa-input>
                <wa-button
                  size="s"
                  class="escalate"
                  variant="warning"
                  appearance="outlined"
                  ?disabled=${busy || escalateReason === ''}
                  @click=${() =>
                    void this.answer(card, {
                      decision: 'escalate',
                      reason: escalateReason,
                    })}
                  >Escalate</wa-button
                >
              </div>
              ${card.error
                ? html`<div class="callout callout-error" role="alert">
                    ${icon('alert')}<span>${card.error}</span>
                  </div>`
                : nothing}
            `}
      </section>
    `
  }

  render() {
    if (this.cards.length === 0) return nothing
    return html`
      <div class="review-cards" role="region" aria-label="Permission review">
        ${this.cards.map((card) => this.renderCard(card))}
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-review-cards': SebasReviewCards
  }
}
