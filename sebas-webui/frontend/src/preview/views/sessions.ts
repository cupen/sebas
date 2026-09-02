/**
 * Preview sessions list stub.
 */

import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'

@customElement('sebas-preview-sessions')
export class SebasPreviewSessions extends LitElement {
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
          <h1 class="page-title">Sessions</h1>
          <p class="page-sub">All agent sessions across projects.</p>
        </div>
      </header>

      <section class="panel">
        <div class="panel-head"><h2 class="panel-title">All sessions</h2><span class="panel-caption tnum">3 total</span></div>
        <table>
          <thead><tr><th>Project</th><th>Chat</th><th>Session</th><th>Status</th><th>Model</th><th>Last active</th></tr></thead>
          <tbody>
            <tr data-status="working"><td>sebas</td><td><a class="mono" href="/sessions/sess_9f2a">oc_9f2a</a></td><td class="mono dim">sess_9f2a…</td><td><span style="color:var(--sebas-status-working);font-size:0.85rem;">● working</span></td><td class="mono dim">claude-sonnet-4</td><td class="tnum dim" style="font-size:0.85rem;">2m ago</td></tr>
            <tr data-status="done"><td>beads</td><td><a class="mono" href="/sessions/sess_xyz">beads</a></td><td class="mono dim">sess_xyz…</td><td><span style="color:var(--sebas-status-done);font-size:0.85rem;">● done</span></td><td class="mono dim">claude-opus-4</td><td class="tnum dim" style="font-size:0.85rem;">12m ago</td></tr>
            <tr data-status="dormant"><td>—</td><td><a class="mono" href="/sessions/sess_abc">dotfiles</a></td><td class="mono dim">sess_abc…</td><td><span style="color:var(--sebas-status-dormant);font-size:0.85rem;">● dormant</span></td><td class="mono dim">deepseek-reasoner</td><td class="tnum dim" style="font-size:0.85rem;">2h ago</td></tr>
          </tbody>
        </table>
      </section>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-sessions': SebasPreviewSessions
  }
}