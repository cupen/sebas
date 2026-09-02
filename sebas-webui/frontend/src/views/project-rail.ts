/**
 * Project rail (workbench left rail).
 *
 * Lists registered projects in user-defined order, with HTML5 drag-and-drop
 * to reorder. Each row carries a session count, a wait-dot for sessions
 * that need operator attention, the project branch (lazy), and an
 * `unreachable` state when the directory disappears from disk.
 *
 * Ordering is persisted server-side via POST /api/projects/reorder.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { icon } from '../components/icons.js'
import { api, type Project, type ProjectBranchInfo, type SessionRow } from '../api/client.js'

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  /** Sessions snapshot the parent workbench pushes on each refresh. */
  @property({ attribute: false }) sessions: SessionRow[] = []
  /** Currently-selected project path; mirrors the parent's selectedPath. */
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  @state() private branchByPath: Record<string, ProjectBranchInfo> = {}
  @state() private dragIndex: number | null = null
  @state() private dragOverIndex: number | null = null
  @state() private error: string | null = null
  @state() private adding = false
  @state() private addPath = ''
  @state() private addError: string | null = null

  private fetchSeq = 0

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: 4px;
      min-width: 0;
    }
    .group-label {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 6px 8px 2px;
      font-size: 0.66rem;
      font-weight: 600;
      letter-spacing: 0.09em;
      text-transform: uppercase;
      color: var(--sebas-text-faint);
    }
    .group-label .count {
      margin-left: auto;
      font-weight: 500;
      color: var(--sebas-text-dim);
    }
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
      display: flex;
      flex-direction: column;
      gap: 1px;
    }
    li.row {
      position: relative;
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
      user-select: none;
    }
    li.row:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    li.row.active {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    li.row.dragging {
      opacity: 0.4;
    }
    li.row.drag-over {
      box-shadow: inset 0 2px 0 var(--sebas-accent);
    }
    .handle {
      display: grid;
      place-items: center;
      color: var(--sebas-text-faint);
      cursor: grab;
    }
    .handle:active {
      cursor: grabbing;
    }
    .name {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-weight: 500;
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    .meta {
      display: flex;
      align-items: center;
      gap: 6px;
      color: var(--sebas-text-faint);
      font-size: 0.7rem;
    }
    .meta .branch {
      font-family: var(--sebas-font-mono);
    }
    .meta .count {
      background: var(--sebas-surface-2);
      border-radius: 999px;
      padding: 1px 7px;
      font-weight: 500;
    }
    li.row.active .meta .count {
      background: var(--sebas-accent-strong);
      color: var(--sebas-accent-ink);
    }
    .wait-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--sebas-status-warn);
      display: inline-block;
    }
    li.row.unreachable .name {
      text-decoration: line-through;
      color: var(--sebas-text-faint);
    }
    .empty {
      padding: 10px 12px;
      color: var(--sebas-text-faint);
      font-size: 0.78rem;
    }
    .add-row {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 7px 9px;
      border-radius: var(--sebas-radius-md);
      font-size: 0.8rem;
      color: var(--sebas-text-dim);
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .add-row:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .add-form {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding: 8px 9px;
      background: var(--sebas-surface-2);
      border-radius: var(--sebas-radius-md);
      font-size: 0.78rem;
    }
    .add-form input {
      font: inherit;
      color: var(--sebas-text-bright);
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border-strong);
      border-radius: var(--sebas-radius-sm);
      padding: 6px 8px;
      outline: none;
    }
    .add-form input:focus {
      border-color: var(--sebas-accent);
      box-shadow: 0 0 0 2px var(--sebas-accent-soft);
    }
    .add-form .actions {
      display: flex;
      gap: 4px;
      justify-content: flex-end;
    }
    .add-form button {
      font: inherit;
      padding: 4px 10px;
      border-radius: var(--sebas-radius-sm);
      border: 1px solid var(--sebas-border-strong);
      background: var(--sebas-surface);
      color: var(--sebas-text-dim);
      cursor: pointer;
    }
    .add-form button.primary {
      background: var(--sebas-accent-strong);
      color: var(--sebas-accent-ink);
      border-color: transparent;
    }
    .add-form .err {
      color: var(--sebas-status-fail);
    }
    .error {
      padding: 8px 12px;
      color: var(--sebas-status-fail);
      font-size: 0.78rem;
    }
  `

  protected firstUpdated(_: PropertyValues): void {
    void this.refresh()
  }

  protected updated(changed: PropertyValues): void {
    if (changed.has('sessions')) {
      this.requestUpdate()
    }
  }

  async refresh() {
    const seq = ++this.fetchSeq
    try {
      const { projects } = await api.projects.list()
      if (seq !== this.fetchSeq) return
      this.projects = projects
      this.error = null
      for (const p of projects) {
        if (!this.branchByPath[p.path]) void this.loadBranch(p.path)
      }
    } catch (e) {
      if (seq !== this.fetchSeq) return
      this.error = e instanceof Error ? e.message : String(e)
    }
  }

  private async loadBranch(path: string) {
    try {
      const info = await api.projects.branch(path)
      if (info.path in this.branchByPath || true) {
        this.branchByPath = { ...this.branchByPath, [path]: info }
      }
    } catch {
      /* 404 = removed mid-flight; ignore */
    }
  }

  private onSelect(path: string) {
    this.dispatchEvent(
      new CustomEvent('rail-select', {
        detail: { path },
        bubbles: true,
        composed: true,
      }),
    )
  }

  private onDragStart(e: DragEvent, index: number) {
    this.dragIndex = index
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData('text/plain', String(index))
    }
  }

  private onDragOver(e: DragEvent, index: number) {
    if (this.dragIndex === null) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
    this.dragOverIndex = index
  }

  private onDragLeave(index: number) {
    if (this.dragOverIndex === index) this.dragOverIndex = null
  }

  private async onDrop(e: DragEvent, dropIndex: number) {
    e.preventDefault()
    const from = this.dragIndex
    this.dragIndex = null
    this.dragOverIndex = null
    if (from === null || from === dropIndex) return
    const next = [...this.projects]
    const [moved] = next.splice(from, 1)
    next.splice(dropIndex, 0, moved)
    this.projects = next
    try {
      const { projects } = await api.projects.reorder(next.map((p) => p.path))
      this.projects = projects
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
      void this.refresh()
    }
  }

  private onDragEnd() {
    this.dragIndex = null
    this.dragOverIndex = null
  }

  private openAdd() {
    this.adding = true
    this.addError = null
  }

  private cancelAdd() {
    this.adding = false
    this.addPath = ''
    this.addError = null
  }

  private async submitAdd() {
    const path = this.addPath.trim()
    if (!path) {
      this.addError = '请输入路径'
      return
    }
    try {
      const p = await api.projects.add(path)
      this.adding = false
      this.addPath = ''
      this.addError = null
      await this.refresh()
      this.onSelect(p.path)
    } catch (e) {
      this.addError = e instanceof Error ? e.message : String(e)
    }
  }

  private countsFor(path: string): { count: number; waiting: boolean } {
    let count = 0
    let waiting = false
    for (const r of this.sessions) {
      if (r.project_dir !== path) continue
      count += 1
      if (r.status_slug === 'queued' || r.status_slug === 'failed' || r.status_slug === 'starting') {
        waiting = true
      }
    }
    return { count, waiting }
  }

  private renderRow(p: Project, index: number) {
    const info = this.branchByPath[p.path]
    const branch = info?.branch ?? p.branch ?? null
    const accessible = info ? info.accessible : true
    const { count, waiting } = this.countsFor(p.path)
    const isActive = this.activePath === p.path
    const dragging = this.dragIndex === index
    const dragOver = this.dragOverIndex === index && this.dragIndex !== null && this.dragIndex !== index
    return html`
      <li
        class=${[
          'row',
          isActive ? 'active' : '',
          accessible ? '' : 'unreachable',
          dragging ? 'dragging' : '',
          dragOver ? 'drag-over' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        draggable="true"
        aria-current=${isActive ? 'true' : 'false'}
        @click=${() => this.onSelect(p.path)}
        @dragstart=${(e: DragEvent) => this.onDragStart(e, index)}
        @dragover=${(e: DragEvent) => this.onDragOver(e, index)}
        @dragleave=${() => this.onDragLeave(index)}
        @drop=${(e: DragEvent) => this.onDrop(e, index)}
        @dragend=${() => this.onDragEnd()}
      >
        <span class="handle" aria-hidden="true">${icon('drag', 14)}</span>
        <span class="name">
          <span>${p.name}</span>
          ${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}
        </span>
        <span class="meta">
          ${branch ? html`<span class="branch">${branch}</span>` : nothing}
          ${count > 0 ? html`<span class="count">${count}</span>` : nothing}
        </span>
      </li>
    `
  }

  render() {
    return html`
      <div class="group-label">Projects <span class="count">${this.projects.length}</span></div>
      ${this.error ? html`<div class="error">${this.error}</div>` : nothing}
      ${this.projects.length === 0 && !this.adding
        ? html`<div class="empty">尚未注册项目</div>`
        : html`<ul>
            ${this.projects.map((p, i) => this.renderRow(p, i))}
          </ul>`}
      ${this.adding
        ? html`
            <div class="add-form">
              <input
                type="text"
                placeholder="/absolute/path/to/repo"
                .value=${this.addPath}
                @input=${(e: Event) => (this.addPath = (e.target as HTMLInputElement).value)}
                @keydown=${(e: KeyboardEvent) => {
                  if (e.key === 'Enter') void this.submitAdd()
                  else if (e.key === 'Escape') this.cancelAdd()
                }}
              />
              ${this.addError ? html`<div class="err">${this.addError}</div>` : nothing}
              <div class="actions">
                <button @click=${this.cancelAdd}>取消</button>
                <button class="primary" @click=${() => void this.submitAdd()}>注册</button>
              </div>
            </div>
          `
        : html`<div class="add-row" @click=${this.openAdd}>
            <span aria-hidden="true">${icon('add', 14)}</span>
            <span>添加项目</span>
          </div>`}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}
