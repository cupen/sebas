/**
 * Preview about stub.
 */

import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'

@customElement('sebas-preview-about')
export class SebasPreviewAbout extends LitElement {
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
          <h1 class="page-title">About</h1>
          <p class="page-sub">Version and runtime information.</p>
        </div>
      </header>

      <section class="panel">
        <div class="panel-head"><h2 class="panel-title">sebas agent router</h2></div>
        <dl style="margin:0;padding:var(--sebas-space-2) var(--sebas-space-4);">
          <div class="kv"><dt>Version</dt><dd class="mono">0.1.0</dd></div>
          <div class="kv"><dt>Node</dt><dd class="mono">cupen-dev</dd></div>
          <div class="kv"><dt>Core status</dt><dd style="color:var(--sebas-status-done);">connected</dd></div>
          <div class="kv"><dt>Uptime</dt><dd class="mono">2h 14m</dd></div>
        </dl>
      </section>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-about': SebasPreviewAbout
  }
}