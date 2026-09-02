/** About view: version, uptime, runtime info. */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, type About } from '../api/client.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'

@customElement('sebas-about')
export class SebasAbout extends LitElement {
  @state() private about: About | null = null
  @state() private error = ''

  static styles = [
    viewStyles,
    css`
      dl {
        margin: 0;
        padding: var(--sebas-space-2) var(--sebas-space-4);
      }
      dd {
        font-family: var(--sebas-font-mono);
        font-size: 0.88rem;
      }
      .version-chip {
        display: inline-block;
        padding: 1px 9px;
        border-radius: var(--sebas-radius-full);
        background: var(--sebas-accent-soft);
        border: 1px solid var(--sebas-accent-border);
        color: var(--sebas-accent);
        font-size: 0.8rem;
        font-weight: 600;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    api
      .about()
      .then((d) => (this.about = d))
      .catch((e) => (this.error = String(e)))
  }

  private renderLoading() {
    return html`
      <div class="panel">
        ${[0, 1, 2, 3].map(
          () => html`
            <div class="skel-row">
              <div class="skel skel-line" style="width:22%"></div>
              <div class="skel skel-line" style="width:38%"></div>
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
    if (!this.about) return this.renderLoading()
    const a = this.about
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">About</h1>
          <p class="page-sub">Runtime build information.</p>
        </div>
      </header>

      <div class="panel">
        <dl>
          <div class="kv">
            <dt>Version</dt>
            <dd><span class="version-chip">${a.version}</span></dd>
          </div>
          <div class="kv">
            <dt>Uptime</dt>
            <dd>${a.uptime}</dd>
          </div>
          <div class="kv">
            <dt>Rust toolchain</dt>
            <dd>${a.rustc_version}</dd>
          </div>
          <div class="kv">
            <dt>Gateway listen</dt>
            <dd>${a.gateway_listen ?? '—'}</dd>
          </div>
          <div class="kv">
            <dt>Providers</dt>
            <dd>${a.provider_count}</dd>
          </div>
        </dl>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-about': SebasAbout
  }
}
