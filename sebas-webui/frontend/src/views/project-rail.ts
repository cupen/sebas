/**
 * Sidebar project tree (app-shell 左侧栏, IA v2 对齐预览原型 preview-app.ts)。
 *
 * 项目行右侧有「新建会话」+ 按钮，点击创建 0-turn placeholder 会话。
 * 会话行右侧有归档按钮。底部：Inbox 组 + History 组（归档会话，可恢复）。
 * 添加项目通过 wa-dialog 弹窗，内嵌 <sebas-folder-picker> 目录树。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { icon } from '../components/icons.js'
import { navigate } from '../router.js'
import {
  api,
  type Project,
  type ProjectBranchInfo,
  type SessionRow,
  type ArchiveEntry,
} from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import '../components/folder-picker.js'

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  @state() private sessions: SessionRow[] = []
  @state() private archivedSessions: ArchiveEntry[] = []
  @state() private expanded: Record<string, boolean> = {}
  @state() private historyOpen = false
  @state() private inboxOpen = false
  @state() private branchByPath: Record<string, ProjectBranchInfo> = {}
  @state() private dragIndex: number | null = null
  @state() private dragOverIndex: number | null = null
  @state() private error: string | null = null

  // Add project dialog state
  @state() private addDialogOpen = false
  @state() private addPath = ''
  @state() private addError: string | null = null

  private fetchSeq = 0
  private unsubscribe?: () => void
  private refetchBound = (): void => { void this.refresh() }

  static styles = css`
    :host { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
    .section-label {
      display: flex; align-items: center; gap: 6px;
      padding: var(--sebas-space-2) 8px var(--sebas-space-1);
      font-size: 0.7rem; font-weight: 600; text-transform: uppercase;
      letter-spacing: 0.08em; color: var(--sebas-text-faint);
    }
    .section-label .add-btn {
      margin-left: auto;
      background: var(--sebas-accent-strong); border: none;
      border-radius: var(--sebas-radius-sm);
      color: var(--sebas-accent-ink); cursor: pointer;
      font-size: 16px; line-height: 1; font-weight: 700;
      font-family: var(--sebas-font-mono); padding: 0;
      display: grid; place-items: center; width: 22px; height: 22px;
      transition: opacity var(--sebas-dur) var(--sebas-ease), filter var(--sebas-dur) var(--sebas-ease);
    }
    .section-label .add-btn:hover { filter: brightness(1.15); }
    .section-label .add-btn:focus-visible { outline: var(--sebas-focus-ring); outline-offset: 1px; }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
    .row {
      position: relative; display: grid;
      grid-template-columns: 14px 10px minmax(0, 1fr) auto auto;
      gap: 6px; align-items: center; padding: 6px 8px;
      border-radius: var(--sebas-radius-md); font-size: 0.85rem;
      color: var(--sebas-text-dim); cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      user-select: none;
    }
    .row:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    .row.active { background: var(--sebas-accent-soft); color: var(--sebas-accent); }
    .row.dragging { opacity: 0.4; }
    .row.drag-over { box-shadow: inset 0 2px 0 var(--sebas-accent); }
    .handle {
      display: grid; place-items: center; color: var(--sebas-text-faint);
      cursor: grab; opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .row:hover .handle, .row:focus-within .handle, .row.dragging .handle { opacity: 1; }
    .handle:active { cursor: grabbing; }
    .chevron { display: grid; place-items: center; width: 10px; color: var(--sebas-text-faint); font-size: 9px; line-height: 1; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .chevron.open { transform: rotate(90deg); }
    .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; display: inline-flex; align-items: center; gap: 6px; }
    .meta { display: flex; align-items: center; gap: 6px; color: var(--sebas-text-faint); font-size: 0.7rem; }
    .meta .branch { font-family: var(--sebas-font-mono); }
    .meta .count { background: var(--sebas-surface-2); border-radius: 999px; padding: 1px 7px; font-weight: 500; font-variant-numeric: tabular-nums; }
    .row.active .meta .count { background: var(--sebas-accent-strong); color: var(--sebas-accent-ink); }
    .wait-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--sebas-status-working); display: inline-block; }
    .row-action {
      width: 20px; height: 20px; background: none; border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-sm); color: var(--sebas-text-faint); cursor: pointer;
      font-size: 13px; line-height: 1; display: grid; place-items: center; padding: 0; opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease), border-color var(--sebas-dur) var(--sebas-ease);
    }
    .row:hover .row-action, .row:focus-within .row-action { opacity: 1; }
    .row:hover .row-action { color: var(--sebas-accent); border-color: var(--sebas-accent-border); }
    .row-action:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
    .row-action:focus-visible { opacity: 1; outline: var(--sebas-focus-ring); outline-offset: 1px; }
    ul.sessions { padding: 0; }
    li.session-item {
      display: flex; align-items: center; gap: 8px; padding: 4px 8px 4px 28px;
      border-radius: var(--sebas-radius-md); color: var(--sebas-text-dim); font-size: 0.8rem;
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    li.session-item:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    li.session-item.current { background: var(--sebas-accent-soft); color: var(--sebas-accent); }
    .session-dot { width: 6px; height: 6px; border-radius: 50%; flex: 0 0 auto; background: var(--sebas-text-faint); }
    .session-dot[data-status='starting'] { background: var(--sebas-status-starting); }
    .session-dot[data-status='queued'] { background: var(--sebas-status-queued); }
    .session-dot[data-status='working'] { background: var(--sebas-status-working); }
    .session-dot[data-status='done'] { background: var(--sebas-status-done); }
    .session-dot[data-status='failed'] { background: var(--sebas-status-failed); }
    .session-dot[data-status='dormant'] { background: var(--sebas-status-dormant); }
    .session-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--sebas-font-mono); font-size: 0.74rem; }
    li.session-item.unreachable .session-name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .row.unreachable .name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .empty { padding: 10px 12px; color: var(--sebas-text-faint); font-size: 0.78rem; }
    .session-archive-btn {
      width: 18px; height: 18px; background: none; border: none; color: var(--sebas-text-faint);
      cursor: pointer; padding: 0; display: grid; place-items: center; border-radius: var(--sebas-radius-sm);
      opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease);
    }
    li.session-item:hover .session-archive-btn { opacity: 1; }
    .session-archive-btn:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
    li.session-item.archived { opacity: 0.7; }
    li.session-item.archived:hover { opacity: 1; }
    .archive-meta { font-size: 0.66rem; color: var(--sebas-text-faint); font-family: var(--sebas-font-mono); }
    .group-section { margin-top: var(--sebas-space-3); }
    .group-head {
      display: flex; align-items: center; gap: 6px; padding: 4px 8px;
      font-size: 0.66rem; font-weight: 600; text-transform: uppercase;
      letter-spacing: 0.08em; color: var(--sebas-text-faint); cursor: pointer;
      user-select: none; border-radius: var(--sebas-radius-md);
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .group-head:hover { background: var(--sebas-surface-2); color: var(--sebas-text-dim); }
    .group-head .chevron { font-size: 9px; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .group-head .chevron.open { transform: rotate(90deg); }
    .group-head .group-count { margin-left: auto; font-variant-numeric: tabular-nums; background: var(--sebas-surface-3); border-radius: var(--sebas-radius-full); padding: 0 7px; font-size: 0.62rem; }
    .group-head:focus-visible { outline: var(--sebas-focus-ring); outline-offset: 1px; }
    .error { padding: 8px 12px; color: var(--sebas-status-failed); font-size: 0.78rem; }
  `

  connectedCallback(): void {
    super.connectedCallback()
    void this.refresh()
    this.unsubscribe = sharedWs.subscribe(this.refetchBound)
    window.addEventListener('sebas:refetch', this.refetchBound)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetchBound)
    super.disconnectedCallback()
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
    try {
      const list = await api.sessions()
      if (seq !== this.fetchSeq) return
      this.sessions = list.recent_sessions
    } catch { /* ignore */ }
    try {
      const { archived_sessions } = await api.archiveList()
      if (seq !== this.fetchSeq) return
      this.archivedSessions = archived_sessions
    } catch { /* ignore */ }
  }

  private async loadBranch(path: string) {
    try {
      const info = await api.projects.branch(path)
      if (info.path in this.branchByPath || true) {
        this.branchByPath = { ...this.branchByPath, [path]: info }
      }
    } catch { /* 404 = removed mid-flight */ }
  }

  private onSelect(path: string) {
    this.expanded = { ...this.expanded, [path]: true }
    this.dispatchEvent(new CustomEvent('rail-select', { detail: { path }, bubbles: true, composed: true }))
  }

  private toggleExpand(e: Event, path: string) {
    e.stopPropagation()
    this.expanded = { ...this.expanded, [path]: !(this.expanded[path] ?? false) }
  }

  private openSession(row: SessionRow) { navigate(`/sessions/${row.encoded_key}`) }

  sessionsFor(path: string) { return this.sessions.filter((r) => r.project_dir === path) }
  inboxSessions() { return this.sessions.filter((r) => r.project_dir === null) }

  private async createSession(e: Event, p: Project) {
    e.stopPropagation()
    this.onSelect(p.path)
    try {
      const { key } = await api.createSession(null, p.path, null)
      navigate(`/sessions/${key}`)
    } catch (err) { this.error = err instanceof Error ? err.message : String(err) }
  }

  private async archiveSession(e: Event, encodedKey: string) {
    e.stopPropagation()
    try {
      await api.archiveSession(encodedKey)
      void this.refresh()
    } catch (err) { this.error = err instanceof Error ? err.message : String(err) }
  }

  private async restoreSession(e: Event, encodedKey: string) {
    e.stopPropagation()
    try {
      await api.restoreSession(encodedKey)
      void this.refresh()
      navigate(`/sessions/${encodedKey}`)
    } catch (err) { this.error = err instanceof Error ? err.message : String(err) }
  }

  // ─── Drag & drop ────────────────────────────────────────────────
  private onDragStart(e: DragEvent, index: number) {
    this.dragIndex = index
    if (e.dataTransfer) { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', String(index)) }
  }
  private onDragOver(e: DragEvent, index: number) {
    if (this.dragIndex === null) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
    this.dragOverIndex = index
  }
  private onDragLeave(index: number) { if (this.dragOverIndex === index) this.dragOverIndex = null }
  private async onDrop(e: DragEvent, dropIndex: number) {
    e.preventDefault()
    const from = this.dragIndex
    this.dragIndex = null; this.dragOverIndex = null
    if (from === null || from === dropIndex) return
    const next = [...this.projects]; const [moved] = next.splice(from, 1); next.splice(dropIndex, 0, moved)
    this.projects = next
    try {
      const { projects } = await api.projects.reorder(next.map((p) => p.path))
      this.projects = projects
    } catch (err) { this.error = err instanceof Error ? err.message : String(err); void this.refresh() }
  }
  private onDragEnd() { this.dragIndex = null; this.dragOverIndex = null }

  // ─── Add project dialog ─────────────────────────────────────────
  private openAddDialog() {
    this.addDialogOpen = true
    this.addPath = ''
    this.addError = null
    const picker = this.shadowRoot?.querySelector('.folder-picker') as any
    if (picker?.reset) void picker.reset()
  }
  private closeAddDialog() { this.addDialogOpen = false; this.addPath = ''; this.addError = null }
  private onFolderSelected(e: CustomEvent) { this.addPath = e.detail.path }
  private async submitAddProject() {
    const path = this.addPath.trim()
    if (!path) { this.addError = '请输入路径'; return }
    try {
      const p = await api.projects.add(path)
      this.closeAddDialog()
      await this.refresh()
      this.onSelect(p.path)
    } catch (e) { this.addError = e instanceof Error ? e.message : String(e) }
  }

  private countsFor(path: string): { count: number; waiting: boolean } {
    let count = 0; let waiting = false
    for (const r of this.sessions) {
      if (r.project_dir !== path) continue
      count += 1
      if (r.status_slug === 'queued' || r.status_slug === 'failed' || r.status_slug === 'starting') { waiting = true }
    }
    return { count, waiting }
  }

  // ─── Renderers ──────────────────────────────────────────────────
  private renderSessionRow(row: SessionRow) {
    const label = row.session_id_short ?? row.chat_id
    const current = location.pathname === `/sessions/${row.encoded_key}`
    return html`
      <li class="session-item ${current ? 'current' : ''}" title=${row.chat_id} aria-current=${current ? 'true' : 'false'} @click=${() => this.openSession(row)}>
        <span class="session-dot" data-status=${row.status_slug} aria-hidden="true"></span>
        <span class="session-name">${label}</span>
        <button class="session-archive-btn" title="Archive this session" aria-label="Archive ${label}" @click=${(e: Event) => this.archiveSession(e, row.encoded_key)}>${icon('inbox', 11)}</button>
      </li>`
  }

  private renderArchivedSessionRow(a: ArchiveEntry) {
    const current = location.pathname === `/sessions/${a.session_key}`
    return html`
      <li class="session-item archived ${current ? 'current' : ''}" title=${a.session_key} aria-current=${current ? 'true' : 'false'} @click=${(e: Event) => this.restoreSession(e, a.session_key)}>
        <span class="session-dot done" aria-hidden="true"></span>
        <span class="session-name">${a.label}</span>
        <span class="archive-meta">${a.project_path.split('/').filter(Boolean).pop() ?? ''}</span>
      </li>`
  }

  private renderRow(p: Project, index: number) {
    const info = this.branchByPath[p.path]
    const branch = info?.branch ?? p.branch ?? null
    const accessible = info ? info.accessible : true
    const { count, waiting } = this.countsFor(p.path)
    const isActive = this.activePath === p.path
    const isExpanded = this.expanded[p.path] ?? false
    const dragging = this.dragIndex === index
    const dragOver = this.dragOverIndex === index && this.dragIndex !== null && this.dragIndex !== index
    const projectSessions = this.sessionsFor(p.path)
    return html`
      <li>
        <div class=${['row', isActive ? 'active' : '', accessible ? '' : 'unreachable', dragging ? 'dragging' : '', dragOver ? 'drag-over' : ''].filter(Boolean).join(' ')} draggable="true" aria-current=${isActive ? 'true' : 'false'} aria-expanded=${isExpanded ? 'true' : 'false'} @click=${() => this.onSelect(p.path)} @dragstart=${(e: DragEvent) => this.onDragStart(e, index)} @dragover=${(e: DragEvent) => this.onDragOver(e, index)} @dragleave=${() => this.onDragLeave(index)} @drop=${(e: DragEvent) => this.onDrop(e, index)} @dragend=${() => this.onDragEnd()}>
          <span class="handle" aria-hidden="true">${icon('drag', 14)}</span>
          <span class="chevron ${isExpanded ? 'open' : ''}" aria-hidden="true" @click=${(e: Event) => this.toggleExpand(e, p.path)}>▶</span>
          <span class="name"><span>${p.name}</span>${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}</span>
          <span class="meta">${branch ? html`<span class="branch">${branch}</span>` : nothing}${count > 0 ? html`<span class="count">${count}</span>` : nothing}</span>
          <button class="row-action" title="New session in ${p.name}" aria-label="New session in ${p.name}" @click=${(e: Event) => this.createSession(e, p)}>+</button>
        </div>
        ${isExpanded ? (projectSessions.length > 0 ? html`<ul class="sessions">${projectSessions.map((r) => this.renderSessionRow(r))}</ul>` : html`<div class="empty">该项目暂无会话</div>`) : nothing}
      </li>`
  }

  private renderInbox() {
    const inbox = this.inboxSessions()
    if (inbox.length === 0) return nothing
    return html`
      <div class="group-section">
        <div class="group-head" role="button" tabindex="0" aria-expanded=${this.inboxOpen ? 'true' : 'false'} @click=${() => (this.inboxOpen = !this.inboxOpen)} @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.inboxOpen = !this.inboxOpen } }}>
          <span class="chevron ${this.inboxOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>Inbox</span><span class="group-count">${inbox.length}</span>
        </div>
        ${this.inboxOpen ? html`<ul class="sessions">${inbox.map((r) => this.renderSessionRow(r))}</ul>` : nothing}
      </div>`
  }

  private renderHistory() {
    if (this.archivedSessions.length === 0) return nothing
    return html`
      <div class="group-section">
        <div class="group-head" role="button" tabindex="0" aria-expanded=${this.historyOpen ? 'true' : 'false'} @click=${() => (this.historyOpen = !this.historyOpen)} @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.historyOpen = !this.historyOpen } }}>
          <span class="chevron ${this.historyOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>History</span><span class="group-count">${this.archivedSessions.length}</span>
        </div>
        ${this.historyOpen ? html`<ul class="sessions">${this.archivedSessions.map((a) => this.renderArchivedSessionRow(a))}</ul>` : nothing}
      </div>`
  }

  render() {
    return html`
      <div class="section-label">
        <span>Projects</span>
        <button class="add-btn" aria-label="Add project" title="添加项目" @click=${this.openAddDialog}>+</button>
      </div>
      ${this.error ? html`<div class="error">${this.error}</div>` : nothing}
      ${this.projects.length === 0 ? html`<div class="empty">尚未注册项目</div>` : html`<ul>${this.projects.map((p, i) => this.renderRow(p, i))}</ul>`}
      ${this.renderInbox()}
      ${this.renderHistory()}

      <wa-dialog label="Add project" style="--width: 480px;" .open=${this.addDialogOpen} @wa-hide=${() => this.closeAddDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">
          <p style="font-size:0.85rem;color:var(--sebas-text);margin:0;">Choose a directory to add as a project:</p>
          <sebas-folder-picker class="folder-picker" @folder-selected=${this.onFolderSelected}></sebas-folder-picker>
          <p style="font-size:0.8rem;color:var(--sebas-text-faint);margin:0;text-align:center;">or</p>
          <wa-input label="Project path" placeholder="/absolute/path/to/repo" .value=${this.addPath} @wa-input=${(e: any) => (this.addPath = e.target.value)}>
            <wa-icon slot="start" name="folder" aria-hidden="true"></wa-icon>
          </wa-input>
          ${this.addError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.addError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="brand" @click=${() => void this.submitAddProject()} ?disabled=${!this.addPath.trim()}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeAddDialog()}>Cancel</wa-button>
      </wa-dialog>`
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}