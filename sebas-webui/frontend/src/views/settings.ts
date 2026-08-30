/**
 * Settings: card config (display-only contract, same as the SSR page) and
 * basic gateway info.
 */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, type CardConfig, type GatewayInfo } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'

@customElement('sebas-settings')
export class SebasSettings extends LitElement {
  @state() private card: CardConfig | null = null
  @state() private gateway: GatewayInfo | null = null
  @state() private error = ''

  static styles = [
    viewStyles,
    css`
      h2 {
        font-size: 1.05rem;
        margin: var(--sebas-space-6) 0 var(--sebas-space-3);
        color: var(--sebas-text-bright);
      }
      dl {
        margin: 0;
        padding: var(--sebas-space-2) var(--sebas-space-4);
      }
      dd.mono {
        font-family: var(--sebas-font-mono);
        font-size: 0.85rem;
      }
      dd.bool-yes {
        color: var(--sebas-status-done);
      }
      dd.bool-no {
        color: var(--sebas-text-dim);
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    api
      .settings()
      .then((d) => {
        this.card = d.card_config
        this.gateway = d.gateway
      })
      .catch((e) => (this.error = String(e)))
  }

  private renderLoading() {
    return html`
      <div class="panel" style="padding: var(--sebas-space-4)">
        ${[0, 1, 2].map(
          () => html`
            <div class="skel-row">
              <div class="skel skel-line" style="width:24%"></div>
              <div class="skel skel-line" style="width:40%"></div>
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
    if (!this.card || !this.gateway) return this.renderLoading()
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Settings</h1>
          <p class="page-sub">Card configuration and gateway info.</p>
        </div>
      </header>

      <h2>Card configuration</h2>
      <div class="panel">
        <dl>
          <div class="kv">
            <dt>Theme color</dt>
            <dd class="mono">${this.card.theme_color}</dd>
          </div>
          <div class="kv">
            <dt>Fold long output</dt>
            <dd class=${this.card.fold_long_output ? 'bool-yes' : 'bool-no'}>
              ${this.card.fold_long_output ? 'yes' : 'no'}
            </dd>
          </div>
          <div class="kv">
            <dt>Thinking display</dt>
            <dd>${this.card.thinking_display}</dd>
          </div>
          <div class="kv">
            <dt>Max user text chars</dt>
            <dd class="mono">${this.card.max_user_text_chars}</dd>
          </div>
          <div class="kv">
            <dt>Max tool output chars</dt>
            <dd class="mono">${this.card.max_tool_output_chars}</dd>
          </div>
        </dl>
      </div>

      <h2>Gateway</h2>
      <div class="panel">
        <dl>
          <div class="kv">
            <dt>Listen</dt>
            <dd class="mono">${this.gateway.listen ?? '—'}</dd>
          </div>
          <div class="kv">
            <dt>Providers</dt>
            <dd class="mono">${this.gateway.provider_count}</dd>
          </div>
          <div class="kv">
            <dt>Debug</dt>
            <dd class=${this.gateway.debug ? 'bool-yes' : 'bool-no'}>
              ${this.gateway.debug ? 'yes' : 'no'}
            </dd>
          </div>
          <div class="kv">
            <dt>Auth</dt>
            <dd class=${this.gateway.has_auth ? 'bool-yes' : 'bool-no'}>
              ${this.gateway.has_auth ? 'configured' : 'none'}
            </dd>
          </div>
        </dl>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-settings': SebasSettings
  }
}
