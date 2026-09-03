/**
 * Dashboard: counts, uptime, focused session, recent sessions. Live updates
 * arrive over the shared WebSocket; a reconnect triggers a refetch.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, type SessionRow, type Summary } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { icon } from '../components/icons.js'
import { navigate } from '../router.js'
import { viewStyles } from '../styles/shared.js'
import '../components/status-badge.js'
import './project-rail.js'
import './workbench-composer.js'

@customElement('sebas-dashboard')
export class SebasDashboard extends LitElement {
  @state() private data: Summary | null = null
  @state() private allRows: SessionRow[] = []
  @state() private error = ''
  /**
   * Selected project path. `null` = "All sessions" view (the legacy
   * dashboard default). The selection only affects the workbench right
   * pane — never the focused-session pointer or any session state.
   */
  @state() private selectedPath: string | null = null
  /**
   * Provider label rendered next to the composer (e.g. "anthropic / claude").
   * `null` while loading; `"no provider configured"` if no providers
   * are registered.
   */
  @state() private providerLabel: string | null = null
  private unsubscribe?: () => void
  private onRailSelect = (e: Event) => {
    const ce = e as CustomEvent<{ path: string | null }>
    this.selectedPath = ce.detail.path
  }
  private onComposerCreated = (e: Event) => {
    const detail = (e as CustomEvent<{ key: string }>).detail
    navigate(`/sessions/${detail.key}`)
  }

  static styles = [
    viewStyles,
    css`
      .stats {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
        gap: var(--sebas-space-3);
        margin-bottom: var(--sebas-space-5);
      }
      .stat {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-1);
        transition:
          border-color var(--sebas-dur) var(--sebas-ease),
          transform var(--sebas-dur) var(--sebas-ease);
      }
      .stat:hover {
        border-color: var(--sebas-border-strong);
        transform: translateY(-1px);
      }
      .stat .cap {
        display: flex;
        align-items: center;
        gap: 7px;
        color: var(--sebas-text-dim);
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.07em;
        font-weight: 550;
      }
      .stat .cap .pin {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--sebas-text-faint);
      }
      .stat[data-pin='active'] .pin {
        background: var(--sebas-status-working);
      }
      .stat[data-pin='dormant'] .pin {
        background: var(--sebas-status-dormant);
      }
      .stat[data-pin='spawning'] .pin {
        background: var(--sebas-status-starting);
      }
      .stat[data-pin='uptime'] .pin {
        background: var(--sebas-accent);
      }
      .stat .num {
        font-size: 1.65rem;
        font-weight: 700;
        line-height: 1.1;
        color: var(--sebas-text-bright);
        font-variant-numeric: tabular-nums;
        letter-spacing: -0.01em;
      }
      .stat .num.small {
        font-size: 1.15rem;
        padding-top: 6px;
      }
      /* Focused-session spotlight. */
      .spotlight {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        background:
          linear-gradient(var(--sebas-surface), var(--sebas-surface)) padding-box,
          linear-gradient(100deg, var(--sebas-accent-border), transparent 55%) border-box;
        border: 1px solid transparent;
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-3) var(--sebas-space-4);
        margin-bottom: var(--sebas-space-5);
        color: var(--sebas-text);
        text-decoration: none;
        transition: border-color var(--sebas-dur) var(--sebas-ease);
      }
      .spotlight:hover {
        text-decoration: none;
        border-color: var(--sebas-accent-border);
      }
      .spotlight:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      .spotlight .label {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        color: var(--sebas-accent);
        font-size: 0.72rem;
        font-weight: 650;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .spotlight .key {
        font-family: var(--sebas-font-mono);
        font-size: 0.85rem;
        color: var(--sebas-text-bright);
      }
      .spotlight .arrow {
        margin-left: auto;
        color: var(--sebas-text-faint);
      }
      td.time {
        color: var(--sebas-text-dim);
        font-size: 0.85rem;
      }
      .workbench {
        display: grid;
        grid-template-columns: 240px minmax(0, 1fr);
        gap: var(--sebas-space-4);
        align-items: start;
      }
      @media (max-width: 900px) {
        .workbench {
          grid-template-columns: 1fr;
        }
      }
      .rail {
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-3);
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        padding: var(--sebas-space-3);
        box-shadow: var(--sebas-shadow-1);
        min-height: 200px;
      }
      .all-row {
        display: grid;
        grid-template-columns: 18px 1fr auto;
        gap: 8px;
        align-items: center;
        padding: 7px 9px;
        border-radius: var(--sebas-radius-md);
        font-size: 0.85rem;
        color: var(--sebas-text-dim);
        cursor: pointer;
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .all-row:hover {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .all-row.selected {
        background: var(--sebas-accent-soft);
        color: var(--sebas-accent);
      }
      .all-row .all-count {
        background: var(--sebas-surface-2);
        border-radius: 999px;
        padding: 1px 7px;
        font-size: 0.7rem;
        font-weight: 500;
      }
      .all-row.selected .all-count {
        background: var(--sebas-accent-strong);
        color: var(--sebas-accent-ink);
      }
      .all-row .all-glyph {
        display: inline-flex;
        color: var(--sebas-text-faint);
      }
      .all-row.selected .all-glyph {
        color: var(--sebas-accent);
      }
      .all-row .all-label {
        font-weight: 500;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      /* Demoted /sessions entry point: the primary nav no longer carries it,
       * the rail keeps it reachable for old deep links. */
      .rail-foot {
        display: flex;
        align-items: center;
        gap: 6px;
        margin-top: auto;
        padding: 7px 9px;
        border-radius: var(--sebas-radius-md);
        font-size: 0.8rem;
        color: var(--sebas-text-faint);
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .rail-foot:hover {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .right-pane {
        margin: 0;
      }
      .skel-line.w60 {
        width: 60%;
      }
      .skel-line.w25 {
        width: 25%;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.refetch()
    this.unsubscribe = sharedWs.subscribe(() => this.refetch())
    window.addEventListener('sebas:refetch', this.refetch)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetch)
    super.disconnectedCallback()
  }

  private refetch = (): void => {
    api
      .summary()
      .then((d) => {
        this.data = d
        this.error = ''
      })
      .catch((e) => {
        this.error = String(e)
      })
    api
      .sessions()
      .then((list) => {
        this.allRows = list.recent_sessions
      })
      .catch(() => {
        /* summary already surfaces failures */
      })
    // Lazy provider-label fetch: cheap, only fetches once because the
    // composer reads the cached field — refetches on WS push keep the
    // label fresh if the operator reconfigures providers mid-session.
    api
      .settings()
      .then((s) => {
        const first = s.gateway?.providers?.[0]
        this.providerLabel = first ? first.name : 'no provider configured'
      })
      .catch(() => {
        /* leave existing label in place */
      })
  }

  private renderLoading() {
    return html`
      <div class="stats">
        ${[0, 1, 2, 3].map(
          () => html`
            <div class="stat">
              <div class="skel skel-line" style="width:52px"></div>
              <div class="skel skel-line" style="width:38px;height:22px"></div>
            </div>
          `,
        )}
      </div>
      <div class="panel">
        ${[0, 1, 2, 3].map(
          () => html`
            <div class="skel-row">
              <div class="skel skel-line w25"></div>
              <div class="skel skel-line w25"></div>
              <div class="skel skel-line w60"></div>
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
    if (!this.data) return this.renderLoading()
    const d = this.data
    return html`
      <header class="page-head">
        <div>
          <h1 class="page-title">Workbench</h1>
          <p class="page-sub">
            ${this.selectedPath
              ? 'Sessions in the selected project.'
              : 'Live overview across every project.'}
          </p>
        </div>
      </header>

      <div class="stats">
        <div class="stat" data-pin="active">
          <span class="cap"><span class="pin"></span>Active</span>
          <span class="num">${d.active_count}</span>
        </div>
        <div class="stat" data-pin="dormant">
          <span class="cap"><span class="pin"></span>Dormant</span>
          <span class="num">${d.dormant_count}</span>
        </div>
        <div class="stat" data-pin="spawning">
          <span class="cap"><span class="pin"></span>Spawning</span>
          <span class="num">${d.spawning_count}</span>
        </div>
        <div class="stat" data-pin="uptime">
          <span class="cap"><span class="pin"></span>Uptime</span>
          <span class="num small">${d.uptime}</span>
        </div>
      </div>

      ${d.active_session
        ? html`
            <a class="spotlight" href=${`/sessions/${d.active_session.encoded_key}`}>
              <span class="label">${icon('zap', 12)} Focused now</span>
              <span class="key">${d.active_session.chat_id}</span>
              <sebas-status-badge
                slug=${d.active_session.status_slug}
                label=${d.active_session.status_label}
                glyph=${d.active_session.status_glyph}
              ></sebas-status-badge>
              <span class="arrow">${icon('forward', 14)}</span>
            </a>
          `
        : nothing}

      <div class="workbench">
        <aside class="rail">
          <div
            class="all-row ${this.selectedPath === null ? 'selected' : ''}"
            @click=${() => (this.selectedPath = null)}
            role="button"
            tabindex="0"
          >
            <span class="all-glyph" aria-hidden="true">${icon('inbox', 14)}</span>
            <span class="all-label">All sessions</span>
            <span class="all-count">${d.total_sessions}</span>
          </div>
          <sebas-project-rail
            .sessions=${this.allRows}
            .activePath=${this.selectedPath}
            @rail-select=${this.onRailSelect}
          ></sebas-project-rail>
          <a class="rail-foot" href="/sessions">所有会话 / all sessions</a>
        </aside>

        <section class="panel right-pane">
          <sebas-workbench-composer
            .projectDir=${this.selectedPath}
            .providerLabel=${this.providerLabel}
            @composer-created=${this.onComposerCreated}
          ></sebas-workbench-composer>
          <div class="panel-head">
            <h2 class="panel-title">
              ${this.selectedPath
                ? `Sessions · ${this.selectedPath.split('/').filter(Boolean).pop() ?? this.selectedPath}`
                : 'Recent sessions'}
            </h2>
            <span class="panel-caption tnum">
              ${this.rowsForSelected().length} shown
            </span>
          </div>
          ${this.renderSessionTable(this.rowsForSelected(), this.selectedPath === null)}
        </section>
      </div>
    `
  }

  private rowsForSelected(): SessionRow[] {
    if (this.selectedPath === null) return this.allRows
    return this.allRows.filter((r) => r.project_dir === this.selectedPath)
  }

  private renderSessionTable(rows: SessionRow[], _showAll: boolean) {
    if (rows.length === 0)
      return html`
        <div class="empty">
          <span class="glyph">${icon('inbox', 20)}</span>
          <span class="title">No sessions</span>
          <p class="hint">
            ${this.selectedPath
              ? 'This project has no sessions yet. Create one from the composer.'
              : 'Spin up an agent from the Sessions page and it will show up here.'}
          </p>
          ${this.selectedPath === null
            ? html`<a class="cta" href="/sessions">Go to Sessions</a>`
            : nothing}
        </div>
      `
    return html`
      <table>
        <thead>
          <tr>
            <th>Chat</th>
            <th>Session</th>
            <th>Status</th>
            <th>Last active</th>
          </tr>
        </thead>
        <tbody>
          ${rows.map(
            (row) => html`
              <tr data-status=${row.status_slug}>
                <td>
                  <a class="mono" href=${`/sessions/${row.encoded_key}`}>${row.chat_id}</a>
                </td>
                <td class="mono dim" title=${row.session_id ?? ''}>
                  ${row.session_id_short ?? '—'}
                </td>
                <td>
                  <sebas-status-badge
                    slug=${row.status_slug}
                    label=${row.status_label}
                    glyph=${row.status_glyph}
                  ></sebas-status-badge>
                </td>
                <td class="time tnum">${row.last_active}</td>
              </tr>
            `,
          )}
        </tbody>
      </table>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-dashboard': SebasDashboard
  }
}
