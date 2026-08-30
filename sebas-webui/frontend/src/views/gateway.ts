/** Gateway view: provider list with their base URLs. */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, type GatewayInfo } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'

@customElement('sebas-gateway')
export class SebasGateway extends LitElement {
  @state() private gateway: GatewayInfo | null = null
  @state() private error = ''

  static styles = [
    viewStyles,
    css`
      .meta {
        display: flex;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        margin-bottom: var(--sebas-space-5);
      }
      .url {
        font-family: var(--sebas-font-mono);
        font-size: 0.82rem;
        color: var(--sebas-text-dim);
      }
      td.name {
        font-weight: 550;
        color: var(--sebas-text-bright);
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    api
      .gateway()
      .then((d) => (this.gateway = d.gateway))
      .catch((e) => (this.error = String(e)))
  }

  private renderLoading() {
    return html`
      <div class="panel">
        ${[0, 1, 2].map(
          () => html`
            <div class="skel-row">
              <div class="skel skel-line" style="width:20%"></div>
              <div class="skel skel-line" style="width:45%"></div>
              <div class="skel skel-line" style="width:45%"></div>
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
    if (!this.gateway) return this.renderLoading()
    const g = this.gateway
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Gateway</h1>
          <p class="page-sub">Provider routing configuration.</p>
        </div>
      </header>

      <div class="meta">
        <span class="chip"><span class="mono">${g.listen ?? '—'}</span></span>
        <span class="chip"><b>${g.provider_count}</b> provider(s)</span>
        <span class="chip">auth <b>${g.has_auth ? 'configured' : 'none'}</b></span>
        ${g.debug ? html`<span class="chip">debug</span>` : ''}
      </div>

      <div class="panel">
        ${g.providers.length === 0
          ? html`
              <div class="empty">
                <span class="glyph">${icon('gateway', 20)}</span>
                <span class="title">No providers configured</span>
                <p class="hint">Add providers to the gateway config to route traffic.</p>
              </div>
            `
          : html`
              <table>
                <thead>
                  <tr>
                    <th>Provider</th>
                    <th>Anthropic base URL</th>
                    <th>OpenAI base URL</th>
                  </tr>
                </thead>
                <tbody>
                  ${g.providers.map(
                    (p) => html`
                      <tr>
                        <td class="name">${p.name}</td>
                        <td class="url">${p.base_url_anthropic ?? '—'}</td>
                        <td class="url">${p.base_url_openai ?? '—'}</td>
                      </tr>
                    `,
                  )}
                </tbody>
              </table>
            `}
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-gateway': SebasGateway
  }
}
