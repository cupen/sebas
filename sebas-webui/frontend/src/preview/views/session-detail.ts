/**
 * Preview session detail stub.
 */

import { LitElement, css, html } from 'lit'
import { customElement, property } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'

@customElement('sebas-preview-session-detail')
export class SebasPreviewSessionDetail extends LitElement {
  @property() key = ''

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
          <div style="display:flex;align-items:center;gap:var(--sebas-space-3);margin-bottom:var(--sebas-space-2);">
            <a href="/sessions" style="color:var(--sebas-accent);font-size:0.85rem;text-decoration:none;">← Back</a>
          </div>
          <h1 class="page-title" style="font-family:var(--sebas-font-mono);font-size:0.95rem;">Session ${this.key || 'sess_9f2a'}</h1>
          <p class="page-sub">Session detail view (preview).</p>
        </div>
      </header>

      <div class="panel" style="padding:var(--sebas-space-4);">
        <p style="color:var(--sebas-text-dim);font-size:0.875rem;">
          This is a placeholder for the session detail view. In the production app, it would show the full transcript, status, and actions for this session.
        </p>
        <div style="display:flex;gap:var(--sebas-space-3);margin-top:var(--sebas-space-4);">
          <wa-button variant="brand" size="small">Resume session</wa-button>
          <wa-button appearance="plain" size="small">Close session</wa-button>
        </div>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-session-detail': SebasPreviewSessionDetail
  }
}