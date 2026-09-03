/**
 * Preview dashboard stub: simulated stats, recent sessions, and focused session.
 */

import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'

@customElement('sebas-preview-dashboard')
export class SebasPreviewDashboard extends LitElement {
  static styles = [
    viewStyles,
    css`
      :host { display: block; padding: var(--sebas-space-8) var(--sebas-space-10); }
      .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: var(--sebas-space-3); margin-bottom: var(--sebas-space-5); }
      .stat { background: var(--sebas-surface); border: 1px solid var(--sebas-border); border-radius: var(--sebas-radius-lg); box-shadow: var(--sebas-shadow-1); padding: var(--sebas-space-4); display: flex; flex-direction: column; gap: var(--sebas-space-1); }
      .stat .cap { display: flex; align-items: center; gap: 7px; color: var(--sebas-text-dim); font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.07em; font-weight: 550; }
      .stat .cap .pin { width: 7px; height: 7px; border-radius: 50%; }
      .stat .cap .pin.active { background: var(--sebas-status-working); }
      .stat .cap .pin.dormant { background: var(--sebas-status-dormant); }
      .stat .cap .pin.spawning { background: var(--sebas-status-starting); }
      .stat .num { font-size: 1.65rem; font-weight: 700; line-height: 1.1; color: var(--sebas-text-bright); font-variant-numeric: tabular-nums; }
      .spotlight { display: flex; align-items: center; gap: var(--sebas-space-3); flex-wrap: wrap; background: linear-gradient(var(--sebas-surface), var(--sebas-surface)) padding-box, linear-gradient(100deg, var(--sebas-accent-border), transparent 55%) border-box; border: 1px solid transparent; border-radius: var(--sebas-radius-lg); padding: var(--sebas-space-3) var(--sebas-space-4); margin-bottom: var(--sebas-space-5); color: var(--sebas-text); text-decoration: none; }
      .spotlight .label { display: inline-flex; align-items: center; gap: 6px; color: var(--sebas-accent); font-size: 0.72rem; font-weight: 650; text-transform: uppercase; letter-spacing: 0.08em; }
      .spotlight .key { font-family: var(--sebas-font-mono); font-size: 0.85rem; color: var(--sebas-text-bright); }
      @media (max-width: 640px) { :host { padding: var(--sebas-space-4); } }
    `,
  ]

  render() {
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Dashboard</h1>
          <p class="page-sub">Live overview of the agent router (preview).</p>
        </div>
      </header>

      <div class="stats">
        <div class="stat"><span class="cap"><span class="pin active"></span>Active</span><span class="num">2</span></div>
        <div class="stat"><span class="cap"><span class="pin dormant"></span>Dormant</span><span class="num">1</span></div>
        <div class="stat"><span class="cap"><span class="pin spawning"></span>Spawning</span><span class="num">0</span></div>
        <div class="stat"><span class="cap"><span class="pin" style="background:var(--sebas-accent);"></span>Uptime</span><span class="num" style="font-size:1.15rem;">2h 14m</span></div>
      </div>

      <a class="spotlight" href="/sessions/sess_9f2a">
        <span class="label">⚡ Focused now</span>
        <span class="key">oc_9f2a</span>
        <span style="background:var(--sebas-status-working-bg);color:var(--sebas-status-working);padding:2px 8px;border-radius:var(--sebas-radius-full);font-size:0.75rem;">working</span>
      </a>

      <section class="panel">
        <div class="panel-head"><h2 class="panel-title">Recent sessions</h2><span class="panel-caption tnum">3 total</span></div>
        <table>
          <thead><tr><th>Chat</th><th>Session</th><th>Status</th><th>Last active</th></tr></thead>
          <tbody>
            <tr data-status="working"><td><a class="mono" href="/sessions/sess_9f2a">oc_9f2a</a></td><td class="mono dim">sess_9f2a…</td><td><span style="color:var(--sebas-status-working);font-size:0.85rem;">● working</span></td><td class="tnum" style="color:var(--sebas-text-dim);font-size:0.85rem;">2m ago</td></tr>
            <tr data-status="done"><td><a class="mono" href="/sessions/sess_xyz">beads</a></td><td class="mono dim">sess_xyz…</td><td><span style="color:var(--sebas-status-done);font-size:0.85rem;">● done</span></td><td class="tnum" style="color:var(--sebas-text-dim);font-size:0.85rem;">12m ago</td></tr>
            <tr data-status="dormant"><td><a class="mono" href="/sessions/sess_abc">dotfiles</a></td><td class="mono dim">sess_abc…</td><td><span style="color:var(--sebas-status-dormant);font-size:0.85rem;">● dormant</span></td><td class="tnum" style="color:var(--sebas-text-dim);font-size:0.85rem;">2h ago</td></tr>
          </tbody>
        </table>
      </section>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-dashboard': SebasPreviewDashboard
  }
}