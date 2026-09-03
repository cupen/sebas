/**
 * Preview gateway stub.
 */

import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'

@customElement('sebas-preview-gateway')
export class SebasPreviewGateway extends LitElement {
  static styles = [
    viewStyles,
    css`
      :host { display: block; padding: var(--sebas-space-8) var(--sebas-space-10); }
      @media (max-width: 640px) { :host { padding: var(--sebas-space-4); } }
    `,
  ]

  render() {
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Gateway</h1>
          <p class="page-sub">API gateway provider list and base URLs.</p>
        </div>
      </header>

      <section class="panel">
        <div class="panel-head"><h2 class="panel-title">Gateway info</h2></div>
        <dl style="margin:0;padding:var(--sebas-space-2) var(--sebas-space-4);">
          <div class="kv"><dt>Listen</dt><dd class="mono">127.0.0.1:8787</dd></div>
          <div class="kv"><dt>Providers</dt><dd class="mono">2</dd></div>
          <div class="kv"><dt>Debug</dt><dd style="color:var(--sebas-status-done);">yes</dd></div>
          <div class="kv"><dt>Auth</dt><dd style="color:var(--sebas-status-done);">configured</dd></div>
        </dl>
      </section>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-gateway': SebasPreviewGateway
  }
}