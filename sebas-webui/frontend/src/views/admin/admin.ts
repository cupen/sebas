/**
 * Admin cluster views: status (incl. restart), events, update (release /
 * dry-run / dev / rollback), services. A 401 anywhere routes to the login
 * view; 503 on mutations surfaces the honest "control plane not connected"
 * message instead of a dead button.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, ApiError } from '../../api/client.js'
import { navigate } from '../../router.js'
import { icon } from '../../components/icons.js'
import { viewStyles } from '../../styles/shared.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/badge/badge.js'

const VIEWS = ['status', 'events', 'update', 'services'] as const
type AdminView = (typeof VIEWS)[number]

@customElement('sebas-admin')
export class SebasAdmin extends LitElement {
  /** Sub-view from the route: /admin/{status,events,update,services}. */
  @property() view = 'status'
  @state() private error = ''
  @state() private notice = ''

  static styles = [
    viewStyles,
    css`
      .tabs {
        display: flex;
        gap: 4px;
        margin-bottom: var(--sebas-space-5);
        flex-wrap: wrap;
        padding: 4px;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        width: fit-content;
      }
      .tabs a {
        padding: 6px 14px;
        border-radius: var(--sebas-radius-md);
        text-decoration: none;
        color: var(--sebas-text-dim);
        font-size: 0.85rem;
        font-weight: 550;
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .tabs a:hover {
        color: var(--sebas-text-bright);
        background: var(--sebas-surface-2);
        text-decoration: none;
      }
      .tabs a[aria-current='page'] {
        background: var(--sebas-accent-soft);
        color: var(--sebas-accent);
        font-weight: 600;
      }
      .card {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
      }
      .card p {
        margin: 0 0 var(--sebas-space-3);
        color: var(--sebas-text);
        line-height: 1.55;
      }
      .card h3 {
        margin: var(--sebas-space-4) 0 var(--sebas-space-2);
        font-size: 0.95rem;
        color: var(--sebas-text-bright);
      }
      .row {
        display: flex;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
      }
      td.status-cell {
        font-variant-numeric: tabular-nums;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    window.addEventListener('sebas:refetch', this.refetch)
  }

  disconnectedCallback(): void {
    window.removeEventListener('sebas:refetch', this.refetch)
    super.disconnectedCallback()
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    if (changed.has('view')) {
      // Sync the internal mirror BEFORE fetching: willUpdate runs before
      // render() (which also assigns currentView), so a late fetch would
      // still read the previous view and load stale-tab data.
      this.currentView = (VIEWS as readonly string[]).includes(this.view)
        ? (this.view as AdminView)
        : 'status'
      this.refetch()
    }
  }

  private onUnauthorized(): void {
    navigate('/admin/login')
  }

  private refetch = (): void => {
    this.reloadCurrent()
  }

  private async reloadCurrent(): Promise<void> {
    this.error = ''
    this.notice = ''
    try {
      switch (this.currentView) {
        case 'status': {
          this.statusData = await api.adminStatus()
          break
        }
        case 'events': {
          this.eventsData = await api.adminEvents()
          break
        }
        case 'update': {
          this.updateAdapterOk = (await api.adminStatus()).adapter_ok
          break
        }
        case 'services': {
          this.servicesData = await api.adminServices()
          break
        }
      }
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) return this.onUnauthorized()
      this.error = String(e)
    }
  }

  // @state(): assigned after the initial render (async fetches), so each
  // needs the reactive decorator — without it Lit never re-renders and every
  // admin view stays stuck on "Loading…".
  @state() private statusData: Awaited<ReturnType<typeof api.adminStatus>> | null = null
  @state() private eventsData: Awaited<ReturnType<typeof api.adminEvents>> | null = null
  @state() private servicesData: Awaited<ReturnType<typeof api.adminServices>> | null = null
  @state() private updateAdapterOk: boolean | null = null
  private currentView: AdminView = 'status'

  private async runMutation(
    action: () => Promise<{ operation_id: string; message: string }>,
    label: string,
  ): Promise<void> {
    this.error = ''
    this.notice = ''
    try {
      const result = await action()
      this.notice = `${label} accepted (${result.operation_id}): ${result.message}`
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) return this.onUnauthorized()
      if (e instanceof ApiError && e.status === 503) {
        this.error = 'Control plane not connected — run the core with SEBAS_CONTROL_SECRET.'
      } else {
        this.error = String(e)
      }
    }
  }

  render() {
    const current = (VIEWS as readonly string[]).includes(this.view)
      ? (this.view as AdminView)
      : 'status'
    this.currentView = current
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Admin</h1>
          <p class="page-sub">Core control plane.</p>
        </div>
      </header>
      <nav class="tabs" aria-label="Admin sections">
        ${VIEWS.map(
          (v) =>
            html`<a href=${`/admin/${v}`} aria-current=${v === current ? 'page' : nothing}
              >${v}</a
            >`,
        )}
      </nav>
      ${this.error
        ? html`<div class="callout callout-error" role="alert">
            ${icon('alert')}<span>${this.error}</span>
          </div>`
        : nothing}
      ${this.notice
        ? html`<div class="callout callout-info" role="status">${icon('zap', 14)}<span>${this.notice}</span></div>`
        : nothing}
      ${this.renderBody(current)}
    `
  }

  private renderBody(view: AdminView) {
    switch (view) {
      case 'status':
        return this.renderStatus()
      case 'events':
        return this.renderEvents()
      case 'update':
        return this.renderUpdate()
      case 'services':
        return this.renderServices()
    }
  }

  private renderLoading() {
    return html`
      <div class="card">
        ${[0, 1, 2].map(
          () => html`
            <div class="skel-row">
              <div class="skel skel-line" style="width:25%"></div>
              <div class="skel skel-line" style="width:45%"></div>
            </div>
          `,
        )}
      </div>
    `
  }

  private renderStatus() {
    const d = this.statusData
    if (!d) return this.renderLoading()
    return html`
      ${!d.adapter_ok
        ? html`<div class="callout callout-warn" role="status">
            ${icon('alert', 14)}<span>
              Control plane not connected — reads are stale, mutations unavailable.
            </span>
          </div>`
        : nothing}
      <div class="card">
        <p>
          Core version <span class="mono">${d.status.version}</span> · uptime
          <span class="tnum">${d.uptime_display}</span>
        </p>
        <h3>Operations</h3>
        ${d.status.operations.length === 0
          ? html`<p class="dim">No operations recorded.</p>`
          : html`
              <div class="panel" style="box-shadow: none; margin-bottom: var(--sebas-space-4)">
                <table>
                  <thead>
                    <tr>
                      <th>ID</th>
                      <th>Type</th>
                      <th>Status</th>
                      <th>Message</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${d.status.operations.map(
                      (op) => html`
                        <tr>
                          <td class="mono">${op.operation_id}</td>
                          <td>${op.request_type}</td>
                          <td class="status-cell">${op.status}</td>
                          <td>${op.message}</td>
                        </tr>
                      `,
                    )}
                  </tbody>
                </table>
              </div>
            `}
        <div class="row">
          <wa-button
            size="s"
            variant="warning"
            aria-label="Restart core"
            ?disabled=${!d.adapter_ok}
            @click=${() => this.runMutation(api.adminRestart, 'Restart')}
            >Restart core</wa-button
          >
        </div>
      </div>
    `
  }

  private renderEvents() {
    const d = this.eventsData
    if (!d) return this.renderLoading()
    return html`
      <div class="card">
        ${d.events.length === 0
          ? html`<p class="dim">No events.</p>`
          : html`
              <div class="panel" style="box-shadow: none">
                <table>
                  <thead>
                    <tr>
                      <th>Seq</th>
                      <th>Kind</th>
                      <th>Message</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${d.events.map(
                      (ev) => html`
                        <tr>
                          <td class="mono tnum">${ev.seq}</td>
                          <td>
                            <wa-badge variant="${ev.kind === 'error' ? 'danger' : 'neutral'}"
                              >${ev.kind}</wa-badge
                            >
                          </td>
                          <td>${ev.message}</td>
                        </tr>
                      `,
                    )}
                  </tbody>
                </table>
              </div>
            `}
      </div>
    `
  }

  private renderUpdate() {
    if (this.updateAdapterOk === null) return this.renderLoading()
    const disabled = !this.updateAdapterOk
    return html`
      ${disabled
        ? html`<div class="callout callout-warn" role="status">
            ${icon('alert', 14)}<span>Control plane not connected — updates unavailable.</span>
          </div>`
        : nothing}
      <div class="card">
        <p>
          Run an update against the watchdog control plane. Mutations execute directly, without a
          confirmation round-trip.
        </p>
        <div class="row">
          <wa-button
            variant="brand"
            appearance="accent"
            ?disabled=${disabled}
            @click=${() => this.runMutation(api.adminUpdate, 'Update')}
            >Update (release)</wa-button
          >
          <wa-button
            appearance="outlined"
            ?disabled=${disabled}
            @click=${() => this.runMutation(api.adminUpdateDryRun, 'Dry-run')}
            >Dry-run</wa-button
          >
          <wa-button
            appearance="outlined"
            ?disabled=${disabled}
            @click=${() => this.runMutation(api.adminUpdateDev, 'Dev update')}
            >Update (dev)</wa-button
          >
          <wa-button
            variant="warning"
            ?disabled=${disabled}
            @click=${() => this.runMutation(api.adminRollback, 'Rollback')}
            >Rollback</wa-button
          >
        </div>
      </div>
    `
  }

  private renderServices() {
    const d = this.servicesData
    if (!d) return this.renderLoading()
    return html`
      <div class="card">
        ${d.services.length === 0
          ? html`<p class="dim">No services.</p>`
          : html`
              <div class="panel" style="box-shadow: none">
                <table>
                  <thead>
                    <tr>
                      <th>Name</th>
                      <th>Status</th>
                      <th>Desired</th>
                      <th>Uptime</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${d.services.map(
                      (s) => html`
                        <tr>
                          <td class="mono">${s.name}</td>
                          <td class="status-cell">${s.status}</td>
                          <td>${s.desired || '—'}</td>
                          <td class="tnum">
                            ${s.uptime_secs != null ? `${Math.floor(s.uptime_secs / 60)}m` : '—'}
                          </td>
                        </tr>
                      `,
                    )}
                  </tbody>
                </table>
              </div>
            `}
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-admin': SebasAdmin
  }
}
