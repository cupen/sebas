/**
 * Sidebar project tree (app-shell 左侧栏, IA v2 对齐预览原型 preview-app.ts)。
 *
 * 列出已注册项目（用户自定义顺序，HTML5 拖拽排序，服务端经
 * POST /api/projects/reorder 持久化），每个项目行带实时会话计数、
 * 等待介入的 wait-dot、懒加载分支、目录不可达的 unreachable 态。
 * 项目行可展开为嵌套会话行（短 session id + 状态点，点击跳转
 * /sessions/:key 深链）；底部是可折叠的 History 组，收纳未绑定项目
 * （project_dir === null）的会话，组头提供 "All sessions →" 链接。
 *
 * 会话数据由本组件自取（api.sessions + sharedWs 推送/sebas:refetch 刷新），
 * 不再由父视图注入——rail 现在直接挂在 app-shell 侧栏里，跨路由存活。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { icon } from '../components/icons.js'
import { navigate } from '../router.js'
import { api, type Project, type ProjectBranchInfo, type SessionRow } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  /** Currently-selected project path; mirrors the shell's selectedPath. */
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  /** 最近会话快照，驱动项目计数 / 嵌套会话行 / History 组。 */
  @state() private sessions: SessionRow[] = []
  /** 每个项目的展开态（路径 → 是否展开），默认折叠。 */
  @state() private expanded: Record<string, boolean> = {}
  /** History 组折叠态：默认收起（对齐预览原型）。 */
  @state() private historyOpen = false
  @state() private branchByPath: Record<string, ProjectBranchInfo> = {}
  @state() private dragIndex: number | null = null
  @state() private dragOverIndex: number | null = null
  @state() private error: string | null = null
  @state() private adding = false
  @state() private addPath = ''
  @state() private addError: string | null = null

  private fetchSeq = 0
  private unsubscribe?: () => void
  private refetchBound = (): void => {
    void this.refresh()
  }

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: 2px;
      min-width: 0;
    }
    .section-label {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: var(--sebas-space-2) 8px var(--sebas-space-1);
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
    }
    .section-label .add-btn {
      margin-left: auto;
      background: none;
      border: none;
      color: var(--sebas-text-faint);
      cursor: pointer;
      padding: 2px;
      border-radius: var(--sebas-radius-sm);
      display: grid;
      place-items: center;
      width: 18px;
      height: 18px;
      transition:
        color var(--sebas-dur) var(--sebas-ease),
        background var(--sebas-dur) var(--sebas-ease);
    }
    .section-label .add-btn:hover {
      color: var(--sebas-accent);
      background: var(--sebas-accent-soft);
    }
    .section-label .add-btn:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
    }
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
      display: flex;
      flex-direction: column;
      gap: 1px;
    }
    .row {
      position: relative;
      display: grid;
      grid-template-columns: 14px 10px minmax(0, 1fr) auto auto;
      gap: 6px;
      align-items: center;
      padding: 6px 8px;
      border-radius: var(--sebas-radius-md);
      font-size: 0.85rem;
      color: var(--sebas-text-dim);
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
      user-select: none;
    }
    .row:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .row.active {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .row.dragging {
      opacity: 0.4;
    }
    .row.drag-over {
      box-shadow: inset 0 2px 0 var(--sebas-accent);
    }
    .handle {
      display: grid;
      place-items: center;
      color: var(--sebas-text-faint);
      cursor: grab;
      /* 把手默认隐藏，悬停/键盘聚焦/拖拽时浮现（对齐预览原型的 row-action 模式）。
         占位的 grid 列保持不变，行不会回流。 */
      opacity: 0;
      transition:
        opacity var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .row:hover .handle,
    .row:focus-within .handle,
    .row.dragging .handle {
      opacity: 1;
    }
    .handle:active {
      cursor: grabbing;
    }
    .chevron {
      display: grid;
      place-items: center;
      width: 10px;
      color: var(--sebas-text-faint);
      font-size: 9px;
      line-height: 1;
      transition: transform var(--sebas-dur) var(--sebas-ease);
    }
    .chevron.open {
      transform: rotate(90deg);
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
      font-variant-numeric: tabular-nums; /* 计数变化时不抖动 */
    }
    .row.active .meta .count {
      background: var(--sebas-accent-strong);
      color: var(--sebas-accent-ink);
    }
    .wait-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      /* --sebas-status-warn 不存在（会渲染成透明），用已定义的 working 琥珀色 */
      background: var(--sebas-status-working);
      display: inline-block;
    }
    /* 项目行悬停动作：预览原型的 project-add-btn 模式（真实 API 的建会话
     * 需要 prompt，因此这里只做"聚焦该项目"——composer 随即绑定它）。 */
    .row-action {
      width: 20px;
      height: 20px;
      background: none;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-sm);
      color: var(--sebas-text-faint);
      cursor: pointer;
      font-size: 13px;
      line-height: 1;
      display: grid;
      place-items: center;
      padding: 0;
      opacity: 0;
      transition:
        opacity var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease),
        background var(--sebas-dur) var(--sebas-ease),
        border-color var(--sebas-dur) var(--sebas-ease);
    }
    .row:hover .row-action,
    .row:focus-within .row-action {
      opacity: 1;
    }
    .row:hover .row-action {
      color: var(--sebas-accent);
      border-color: var(--sebas-accent-border);
    }
    .row-action:hover {
      color: var(--sebas-accent);
      background: var(--sebas-accent-soft);
    }
    .row-action:focus-visible {
      opacity: 1;
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
    }
    /* 嵌套在项目下的会话行（预览原型 session-item 模式）。 */
    ul.sessions {
      padding: 0;
    }
    li.session-item {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 4px 8px 4px 28px;
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim);
      font-size: 0.8rem;
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    li.session-item:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    li.session-item.current {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .session-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      flex: 0 0 auto;
      background: var(--sebas-text-faint);
    }
    .session-dot[data-status='starting'] {
      background: var(--sebas-status-starting);
    }
    .session-dot[data-status='queued'] {
      background: var(--sebas-status-queued);
    }
    .session-dot[data-status='working'] {
      background: var(--sebas-status-working);
    }
    .session-dot[data-status='done'] {
      background: var(--sebas-status-done);
    }
    .session-dot[data-status='failed'] {
      background: var(--sebas-status-failed);
    }
    .session-dot[data-status='dormant'] {
      background: var(--sebas-status-dormant);
    }
    .session-name {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-family: var(--sebas-font-mono);
      font-size: 0.74rem;
    }
    li.session-item.unreachable .session-name {
      text-decoration: line-through;
      color: var(--sebas-text-faint);
    }
    .row.unreachable .name {
      text-decoration: line-through;
      color: var(--sebas-text-faint);
    }
    .empty {
      padding: 10px 12px;
      color: var(--sebas-text-faint);
      font-size: 0.78rem;
    }
    /* History 组：收纳未绑定项目的会话（预览原型 history-section 模式）。 */
    .history-section {
      margin-top: var(--sebas-space-3);
    }
    .history-head {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 4px 8px;
      font-size: 0.66rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
      cursor: pointer;
      user-select: none;
      border-radius: var(--sebas-radius-md);
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .history-head:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-dim);
    }
    .history-head .chevron {
      font-size: 9px;
      transition: transform var(--sebas-dur) var(--sebas-ease);
    }
    .history-head .chevron.open {
      transform: rotate(90deg);
    }
    .history-head .history-count {
      margin-left: auto;
      font-variant-numeric: tabular-nums;
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-full);
      padding: 0 7px;
      font-size: 0.62rem;
    }
    .history-head .history-all {
      font-size: 0.66rem;
      font-weight: 500;
      letter-spacing: 0.02em;
      text-transform: none;
      color: var(--sebas-text-faint);
      white-space: nowrap;
    }
    .history-head .history-all:hover {
      color: var(--sebas-accent);
    }
    .history-head:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
    }
    .add-form {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding: 8px 9px;
      background: var(--sebas-surface-2);
      border-radius: var(--sebas-radius-md);
      font-size: 0.78rem;
      margin-top: 2px;
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
      color: var(--sebas-status-failed);
    }
    .error {
      padding: 8px 12px;
      color: var(--sebas-status-failed);
      font-size: 0.78rem;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    void this.refresh()
    // 与 dashboard 同款刷新模式：WS 推送 / 重连（sebas:refetch）都触发重取。
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
    } catch {
      /* 会话列表失败时项目树仍可用；计数按空处理 */
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
    // 选中即展开（预览原型 selectProject 行为）。
    this.expanded = { ...this.expanded, [path]: true }
    this.dispatchEvent(
      new CustomEvent('rail-select', {
        detail: { path },
        bubbles: true,
        composed: true,
      }),
    )
  }

  private toggleExpand(e: Event, path: string) {
    e.stopPropagation()
    this.expanded = { ...this.expanded, [path]: !(this.expanded[path] ?? false) }
  }

  /** 嵌套会话行点击 → /sessions/:key 深链（RAW encoded key）。 */
  private openSession(row: SessionRow) {
    navigate(`/sessions/${row.encoded_key}`)
  }

  sessionsFor(path: string): SessionRow[] {
    return this.sessions.filter((r) => r.project_dir === path)
  }

  /** 未绑定项目的会话（inbox）→ History 组。 */
  historySessions(): SessionRow[] {
    return this.sessions.filter((r) => r.project_dir === null)
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

  private renderSessionRow(row: SessionRow) {
    // 短 session id 优先；缺失时退回 chat_id（mono，便于辨认）。
    const label = row.session_id_short ?? row.chat_id
    const current = location.pathname === `/sessions/${row.encoded_key}`
    return html`
      <li
        class="session-item ${current ? 'current' : ''}"
        title=${row.chat_id}
        aria-current=${current ? 'true' : 'false'}
        @click=${() => this.openSession(row)}
      >
        <span class="session-dot" data-status=${row.status_slug} aria-hidden="true"></span>
        <span class="session-name">${label}</span>
      </li>
    `
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
        <div
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
          aria-expanded=${isExpanded ? 'true' : 'false'}
          @click=${() => this.onSelect(p.path)}
          @dragstart=${(e: DragEvent) => this.onDragStart(e, index)}
          @dragover=${(e: DragEvent) => this.onDragOver(e, index)}
          @dragleave=${() => this.onDragLeave(index)}
          @drop=${(e: DragEvent) => this.onDrop(e, index)}
          @dragend=${() => this.onDragEnd()}
        >
          <span class="handle" aria-hidden="true">${icon('drag', 14)}</span>
          <span
            class="chevron ${isExpanded ? 'open' : ''}"
            aria-hidden="true"
            @click=${(e: Event) => this.toggleExpand(e, p.path)}
            >▶</span
          >
          <span class="name">
            <span>${p.name}</span>
            ${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}
          </span>
          <span class="meta">
            ${branch ? html`<span class="branch">${branch}</span>` : nothing}
            ${count > 0 ? html`<span class="count">${count}</span>` : nothing}
          </span>
          <button
            class="row-action"
            title="聚焦此项目（在右侧输入框发起会话）"
            aria-label="Focus ${p.name}"
            @click=${(e: Event) => {
              e.stopPropagation()
              this.onSelect(p.path)
            }}
          >
            +
          </button>
        </div>
        ${isExpanded
          ? projectSessions.length > 0
            ? html`<ul class="sessions">${projectSessions.map((r) => this.renderSessionRow(r))}</ul>`
            : html`<div class="empty">该项目暂无会话</div>`
          : nothing}
      </li>
    `
  }

  private renderHistory() {
    const history = this.historySessions()
    // 空 History 组整体隐藏（对齐预览原型 renderHistoryTree）。
    if (history.length === 0) return nothing
    return html`
      <div class="history-section">
        <div
          class="history-head"
          role="button"
          tabindex="0"
          aria-expanded=${this.historyOpen ? 'true' : 'false'}
          @click=${() => (this.historyOpen = !this.historyOpen)}
          @keydown=${(e: KeyboardEvent) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              this.historyOpen = !this.historyOpen
            }
          }}
        >
          <span class="chevron ${this.historyOpen ? 'open' : ''}" aria-hidden="true">▶</span>
          <span>History</span>
          <span class="history-count">${history.length}</span>
          <a class="history-all" href="/sessions" @click=${(e: Event) => e.stopPropagation()}
            >All sessions →</a
          >
        </div>
        ${this.historyOpen
          ? html`<ul class="sessions">${history.map((r) => this.renderSessionRow(r))}</ul>`
          : nothing}
      </div>
    `
  }

  render() {
    return html`
      <div class="section-label">
        <span>Projects</span>
        <button class="add-btn" aria-label="Add project" title="添加项目" @click=${this.openAdd}>
          ${icon('add', 12)}
        </button>
      </div>
      ${this.error ? html`<div class="error">${this.error}</div>` : nothing}
      ${this.projects.length === 0 && !this.adding
        ? html`<div class="empty">尚未注册项目</div>`
        : html`<ul>
            ${this.projects.map((p, i) => this.renderRow(p, i))}
          </ul>`}
      ${this.renderHistory()}
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
        : nothing}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}
