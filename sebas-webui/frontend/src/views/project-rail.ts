/**
 * Sidebar project tree (app-shell 左侧栏, IA v2 对齐预览原型 preview-app.ts)。
 *
 * 项目行右侧有「新建会话」+ 按钮与「移除项目」按钮（hover 显现），点击
 * 「+」创建 0-turn placeholder 会话（agent 取该项目 default_agent 或首个
 * 可达 agent；创建后留在工作台路由，focus 由后端 set_focus 驱动 composer
 * 进入跟随模式）。会话行右侧有归档与关闭按钮（inactive 直删 / active 需
 * 确认）。底部：Inbox 组 + History 组（归档会话，可恢复）。
 * 添加项目通过 wa-dialog 弹窗，内嵌 <sebas-folder-picker> 目录树。
 *
 * wire（workbench-agent-wire-fix）：项目以稳定 id 引用（remove/branch/
 * reorder），会话行带 project_id；path 不再是标识符。
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
  type AgentKindInfo,
} from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import '../components/folder-picker.js'

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  @state() private sessions: SessionRow[] = []
  /** Agent catalog（/api/agents）：「+」创建占位会话时的 agent 解析数据源。 */
  @state() private agents: AgentKindInfo[] = []
  /**
   * 焦点会话指针（workbench-conversation-view 3.2）：/api/sessions 响应的
   * `active_session_key`——rail 的「当前」标记看它，不看 location.pathname。
   */
  @state() private focusedKey: string | null = null
  @state() private archivedSessions: ArchiveEntry[] = []
  @state() private expanded: Record<string, boolean> = {}
  @state() private historyOpen = false
  @state() private inboxOpen = false
  @state() private branchByPath: Record<string, ProjectBranchInfo> = {}
  @state() private dragIndex: number | null = null
  @state() private dragOverIndex: number | null = null
  @state() private error: string | null = null
  /**
   * 项目注册降级提示（harden-core-channel-deployment 4.3/D7）：核心不可达时
   * 注册落本地注册表，就地提示如实文案；core 恢复后由任一次成功 refresh
   * （ws refetch / 重试）清除。
   */
  @state() private degradedHint: string | null = null

  // Add project dialog state
  @state() private addDialogOpen = false
  @state() private addPath = ''
  @state() private addError: string | null = null

  // Remove project dialog state（workbench-agent-wire-fix 5.1）
  @state() private removeTarget: Project | null = null
  @state() private removeError: string | null = null
  @state() private removing = false

  private fetchSeq = 0
  private unsubscribe?: () => void
  private refetchBound = (): void => { void this.refresh() }

  private async loadAgents(): Promise<void> {
    try {
      const d = await api.agents()
      this.agents = d.agents
    } catch { this.agents = [] }
  }

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
    .degraded-hint {
      margin: 2px 8px 4px;
      padding: 5px 8px;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-status-failed-bg);
      border: 1px solid var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
      font-size: 0.72rem;
      line-height: 1.35;
    }
    .row {
      position: relative; display: grid;
      grid-template-columns: minmax(0, 1fr) auto auto;
      gap: 6px; align-items: center; padding: 6px 10px;
      border-radius: var(--sebas-radius-md); font-size: 0.85rem;
      color: var(--sebas-text-dim); cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      user-select: none;
    }
    .row:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    .row.active { background: var(--sebas-accent-soft); color: var(--sebas-accent); }
    .row.dragging { opacity: 0.4; }
    .row.drag-over { box-shadow: inset 0 2px 0 var(--sebas-accent); }
    .chevron { display: inline-grid; place-items: center; width: 10px; color: var(--sebas-text-faint); font-size: 9px; line-height: 1; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .chevron.open { transform: rotate(90deg); }
    .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; display: inline-flex; align-items: center; gap: 6px; }
    .meta { display: flex; align-items: center; gap: 6px; color: var(--sebas-text-faint); font-size: 0.7rem; }
    .meta .branch { font-family: var(--sebas-font-mono); }
    .meta .count { background: var(--sebas-surface-2); border-radius: 999px; padding: 1px 7px; font-weight: 500; font-variant-numeric: tabular-nums; }
    .row.active .meta .count { background: var(--sebas-accent-strong); color: var(--sebas-accent-ink); }
    .wait-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--sebas-status-working); display: inline-block; }
    .row-actions {
      display: flex; align-items: center; gap: 4px;
    }
    .row-action {
      width: 20px; height: 20px; background: none; border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-sm); color: var(--sebas-text-faint); cursor: pointer;
      font-size: 13px; line-height: 1; display: grid; place-items: center; padding: 0; opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease), border-color var(--sebas-dur) var(--sebas-ease);
    }
    .row-remove:hover { color: var(--sebas-status-failed); background: var(--sebas-status-failed-bg); }
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
    .error .retry-btn {
      margin-left: 4px;
      padding: 1px 8px;
      border: 1px solid currentColor;
      border-radius: var(--sebas-radius-md);
      background: none;
      color: inherit;
      font: inherit;
      font-size: 0.72rem;
      cursor: pointer;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    void this.refresh()
    void this.loadAgents()
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
      this.degradedHint = null
      for (const p of projects) {
        if (!this.branchByPath[p.id]) void this.loadBranch(p.id)
      }
    } catch (e) {
      if (seq !== this.fetchSeq) return
      this.error = e instanceof Error ? e.message : String(e)
    }
    try {
      const list = await api.sessions()
      if (seq !== this.fetchSeq) return
      this.sessions = list.recent_sessions
      this.focusedKey = list.active_session_key
    } catch { /* ignore */ }
    try {
      const { archived_sessions } = await api.archiveList()
      if (seq !== this.fetchSeq) return
      this.archivedSessions = archived_sessions
    } catch { /* ignore */ }
  }

  private async loadBranch(id: string) {
    try {
      const info = await api.projects.branch(id)
      this.branchByPath = { ...this.branchByPath, [id]: info }
    } catch { /* 404 = removed mid-flight */ }
  }

  private onSelect(path: string) {
    this.expanded = { ...this.expanded, [path]: !(this.expanded[path] ?? false) }
    this.dispatchEvent(new CustomEvent('rail-select', { detail: { path }, bubbles: true, composed: true }))
  }

  /**
   * 点会话 = switch + 就地聚焦（workbench-conversation-view 3.1，design
   * D6）：POST switch 设置服务端焦点指针后停在 `/`，dashboard 就地渲染该
   * 会话——不再 navigate 到深链离开工作台。switch 404（会话恰好被关闭）
   * 时只刷新列表，不导航。
   */
  private async openSession(row: SessionRow) {
    try {
      await api.switchSession(row.encoded_key)
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
      void this.refresh()
      return
    }
    this.focusedKey = row.encoded_key
    if (location.pathname !== '/') navigate('/')
  }

  sessionsFor(id: string) { return this.sessions.filter((r) => r.project_id === id) }
  inboxSessions() { return this.sessions.filter((r) => r.project_id === null) }

  /**
   * 「+」创建 0-turn 占位会话（workbench-agent-wire-fix D5/D6）：agent 取
   * 项目 default_agent（该项目最近一次用过的 agent），无记录时取首个可达
   * agent；创建后留在工作台路由——create_session 的 set_focus 会让 summary
   * 的 active_session_key 驱动 composer 进入跟随模式，不再跳深链再跳回。
   */
  private async createSession(e: Event, p: Project) {
    e.stopPropagation()
    this.onSelect(p.path)
    try {
      const agent = p.default_agent || this.firstReachableAgent()
      if (!agent) {
        this.error = '没有可用 agent（/api/agents 列表为空或全部不可达）'
        return
      }
      await api.createSession({ projectId: p.id, agent })
      navigate('/')
    } catch (err) { this.error = err instanceof Error ? err.message : String(err) }
  }

  /** 首个可达 agent id（catalog 加载失败/全不可达时为空串）。 */
  private firstReachableAgent(): string {
    return this.agents.find((a) => a.reachable)?.id ?? ''
  }

  // ─── Remove project（5.1）────────────────────────────────────────
  private openRemoveDialog(e: Event, p: Project) {
    e.stopPropagation()
    this.removeTarget = p
    this.removeError = null
  }
  private closeRemoveDialog() { this.removeTarget = null; this.removeError = null }
  private async confirmRemoveProject() {
    const p = this.removeTarget
    if (!p || this.removing) return
    this.removing = true
    this.removeError = null
    try {
      await api.projects.remove(p.id)
      this.closeRemoveDialog()
      void this.refresh()
    } catch (err) {
      this.removeError = err instanceof Error ? err.message : String(err)
    } finally {
      this.removing = false
    }
  }

  // ─── Close session（5.2：inactive 直删 / active 需确认）────────────
  @state() private closeTarget: SessionRow | null = null
  @state() private closeError: string | null = null
  private static readonly ACTIVE_SLUGS = new Set(['starting', 'queued', 'working'])

  private async closeSession(e: Event, row: SessionRow) {
    e.stopPropagation()
    if (SebasProjectRail.ACTIVE_SLUGS.has(row.status_slug)) {
      // active 会话误杀不可逆——先内联确认。
      this.closeTarget = row
      this.closeError = null
      return
    }
    try {
      await api.closeSession(row.encoded_key)
      void this.refresh()
    } catch (err) { this.error = err instanceof Error ? err.message : String(err) }
  }
  private closeConfirmDialog() { this.closeTarget = null; this.closeError = null }
  private async confirmCloseSession() {
    const row = this.closeTarget
    if (!row) return
    try {
      await api.closeSession(row.encoded_key)
      this.closeConfirmDialog()
      void this.refresh()
    } catch (err) {
      this.closeError = err instanceof Error ? err.message : String(err)
    }
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
      // 恢复后就地聚焦该会话（与点会话同一条 switch 路径），停在工作台。
      await api.switchSession(encodedKey).catch(() => undefined)
      this.focusedKey = encodedKey
      void this.refresh()
      if (location.pathname !== '/') navigate('/')
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
      const { projects } = await api.projects.reorder(next.map((p) => p.id))
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
      // 降级标记就地呈现（refresh 已清 hint，add 的响应说了算）：核心不可达
      // 时项目仍落栏（本地注册表），但操作者不再直到新建会话才得知。
      this.degradedHint = p.degraded?.cause ? `核心不可达（${p.degraded.cause}），已写入本地注册表` : null
      this.onSelect(p.path)
    } catch (e) { this.addError = e instanceof Error ? e.message : String(e) }
  }

  private countsFor(id: string): { count: number; waiting: boolean } {
    let count = 0; let waiting = false
    for (const r of this.sessions) {
      if (r.project_id !== id) continue
      count += 1
      if (r.status_slug === 'queued' || r.status_slug === 'failed' || r.status_slug === 'starting') { waiting = true }
    }
    return { count, waiting }
  }

  // ─── Renderers ──────────────────────────────────────────────────
  private renderSessionRow(row: SessionRow) {
    const label = row.session_id_short ?? row.chat_id
    // 当前标记由焦点指针驱动（3.2）：不再比较 location.pathname。
    const current = this.focusedKey === row.encoded_key
    return html`
      <li class="session-item ${current ? 'current' : ''}" title=${row.chat_id} aria-current=${current ? 'true' : 'false'} @click=${() => this.openSession(row)}>
        <span class="session-dot" data-status=${row.status_slug} aria-hidden="true"></span>
        <span class="session-name">${label}</span>
        <button class="session-archive-btn" title="Archive this session" aria-label="Archive ${label}" @click=${(e: Event) => this.archiveSession(e, row.encoded_key)}>${icon('inbox', 11)}</button>
        <button class="session-archive-btn" title="Close (delete) this session" aria-label="Close ${label}" @click=${(e: Event) => this.closeSession(e, row)}>×</button>
      </li>`
  }

  private renderArchivedSessionRow(a: ArchiveEntry) {
    return html`
      <li class="session-item archived" title=${a.session_key} @click=${(e: Event) => this.restoreSession(e, a.session_key)}>
        <span class="session-dot done" aria-hidden="true"></span>
        <span class="session-name">${a.label}</span>
        <span class="archive-meta">${a.project_path.split('/').filter(Boolean).pop() ?? ''}</span>
      </li>`
  }

  private renderRow(p: Project, index: number) {
    const info = this.branchByPath[p.id]
    const branch = info?.branch ?? p.branch ?? null
    const accessible = info ? info.accessible : true
    const { count, waiting } = this.countsFor(p.id)
    const isActive = this.activePath === p.path
    const isExpanded = this.expanded[p.path] ?? false
    const dragging = this.dragIndex === index
    const dragOver = this.dragOverIndex === index && this.dragIndex !== null && this.dragIndex !== index
    const projectSessions = this.sessionsFor(p.id)
    return html`
      <li>
        <div class=${['row', isActive ? 'active' : '', accessible ? '' : 'unreachable', dragging ? 'dragging' : '', dragOver ? 'drag-over' : ''].filter(Boolean).join(' ')} draggable="true" aria-current=${isActive ? 'true' : 'false'} aria-expanded=${isExpanded ? 'true' : 'false'} @click=${() => this.onSelect(p.path)} @dragstart=${(e: DragEvent) => this.onDragStart(e, index)} @dragover=${(e: DragEvent) => this.onDragOver(e, index)} @dragleave=${() => this.onDragLeave(index)} @drop=${(e: DragEvent) => this.onDrop(e, index)} @dragend=${() => this.onDragEnd()}>
          <span class="name"><span>${p.name}</span>${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}</span>
          <span class="meta">${branch ? html`<span class="branch">${branch}</span>` : nothing}${count > 0 ? html`<span class="count">${count}</span>` : nothing}</span>
          <span class="row-actions">
            <button class="row-action" title="New session in ${p.name}" aria-label="New session in ${p.name}" @click=${(e: Event) => this.createSession(e, p)}>+</button>
            <button class="row-action row-remove" title="Remove ${p.name}" aria-label="Remove ${p.name}" @click=${(e: Event) => this.openRemoveDialog(e, p)}>×</button>
          </span>
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
    // workbench-turn-queue 7.4：关闭确认对话框点名将被丢弃的待执行条数。
    const closePendingCount = this.closeTarget?.pending_count ?? 0
    return html`
      <div class="section-label">
        <span>Projects</span>
        <button class="add-btn" aria-label="Add project" title="添加项目" @click=${this.openAddDialog}>+</button>
      </div>
      ${this.error ? html`<div class="error">${this.error} <button class="retry-btn" @click=${() => void this.refresh()}>重试</button></div>` : nothing}
      ${this.degradedHint ? html`<div class="degraded-hint" role="status" data-testid="project-degraded-hint">${this.degradedHint}</div>` : nothing}
      ${this.projects.length === 0 ? html`<div class="empty">尚未注册项目</div>` : html`<ul>${this.projects.map((p, i) => this.renderRow(p, i))}</ul>`}
      ${this.renderInbox()}
      ${this.renderHistory()}

      <wa-dialog label="Add project" style="--width: 480px;" .open=${this.addDialogOpen} @wa-hide=${() => this.closeAddDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">
          <p style="font-size:0.85rem;color:var(--sebas-text);margin:0;">Choose a directory to add as a project:</p>
          <sebas-folder-picker class="folder-picker" @folder-selected=${this.onFolderSelected}></sebas-folder-picker>
          <p style="font-size:0.8rem;color:var(--sebas-text-faint);margin:0;text-align:center;">or</p>
          <!-- Web Awesome 3.x 派发标准 input 事件（不派发 wa-input），手动路径才能联动启用提交按钮。 -->
          <wa-input label="Project path" placeholder="/absolute/path/to/repo" .value=${this.addPath} @input=${(e: any) => (this.addPath = e.target.value)}>
            <wa-icon slot="start" name="folder" aria-hidden="true"></wa-icon>
          </wa-input>
          ${this.addError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.addError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="brand" @click=${() => void this.submitAddProject()} ?disabled=${!this.addPath.trim()}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeAddDialog()}>Cancel</wa-button>
      </wa-dialog>

      <wa-dialog label="Remove project" style="--width: 440px;" .open=${this.removeTarget !== null} @wa-hide=${() => this.closeRemoveDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            移除项目 <b>${this.removeTarget?.name ?? ''}</b>？
          </p>
          <p style="font-size:0.8rem;color:var(--sebas-text-dim);margin:0;">
            该项目下的存活会话不会被终止，将迁移到 Inbox 分组继续运行。此操作只解除注册，可重新添加。
          </p>
          ${this.removeError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.removeError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="danger" ?loading=${this.removing} @click=${() => void this.confirmRemoveProject()}>移除</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeRemoveDialog()}>取消</wa-button>
      </wa-dialog>

      <wa-dialog label="Close session" style="--width: 440px;" .open=${this.closeTarget !== null} @wa-hide=${() => this.closeConfirmDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            关闭会话 <b>${this.closeTarget?.chat_id ?? ''}</b>？
          </p>
          <p style="font-size:0.8rem;color:var(--sebas-text-dim);margin:0;">
            该会话的 agent 子进程正在运行，关闭会终止子进程并移除会话映射，不可撤销。
          </p>
          ${closePendingCount > 0
            ? html`<p
                style="font-size:0.8rem;color:var(--sebas-status-failed);margin:0;"
                data-testid="close-discards-pending"
              >
                将丢弃 <b>${closePendingCount}</b> 条待执行消息，它们不会被执行。
              </p>`
            : nothing}
          ${this.closeError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.closeError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="danger" @click=${() => void this.confirmCloseSession()}>关闭会话</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeConfirmDialog()}>取消</wa-button>
      </wa-dialog>`
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}