/**
 * Sidebar project tree (app-shell 左侧栏, IA v2 对齐预览原型 preview-app.ts)。
 *
 * rail-declutter-unread：行操作收敛——项目行 = `…` 菜单（移除）+ `+` 新建；
 * 会话行 = 单个 `…` 菜单（归档 / 关闭，active 会话关闭需确认）。按钮默认
 * 隐藏，hover / focus-within 显现（沿用 .row-action 既有 CSS 契约）。会话
 * 行渲染未读徽标（服务端 msg_count − 共享读锚，聚焦清零，见 unread-cursor）。
 * 会话名改用首条用户消息预览（prompt_preview，40 码点截断，title 挂全文）。
 * 分支名不再显示（可达性探测保留，删除线告警不变）。Inbox 分组移除：无
 * 项目会话不再进 rail。History 组按归档时间倒序。
 *
 * wire（workbench-agent-wire-fix）：项目以稳定 id 引用（remove/branch/
 * reorder），会话行带 project_id；path 不再是标识符。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { navigate } from '../router.js'
import {
  api,
  type Project,
  type ProjectBranchInfo,
  type SessionRow,
  type ArchiveEntry,
  type NodeInfo,
} from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { unreadCount, writeFocusAnchor } from './unread-cursor.js'
import type { NewSessionDialogConfirm } from './new-session-dialog.js'
import '../components/folder-picker.js'
import './new-session-dialog.js'
import '@awesome.me/webawesome/dist/components/dropdown/dropdown.js'
import '@awesome.me/webawesome/dist/components/dropdown-item/dropdown-item.js'

/** 节点可用性轮询周期（add-remote-execution-node 8.2）：节点回归后**免刷新**
 * 恢复——项目行与「+」的可用态跟着真实状态翻转。兼任 rail-declutter-unread
 * 的徽标兜底刷新（session.updated 不逐条目触发）。 */
const NODE_POLL_MS = 10_000

/** 本机节点标识（与后端 projects::LOCAL_NODE_ID 同一词表）。 */
const LOCAL_NODE = 'local'

/** 会话名显示上限（码点）——超出截断加省略号，title 挂全文（D10）。 */
const NAME_CAP_CODEPOINTS = 40

/** 未读徽标数字封顶（design Open Question：任务内自决为 99+）。 */
const UNREAD_BADGE_CAP = 99

/** 会话名截断：超上限加 `…`（按码点，不切多字节字符）。 */
export function truncateName(label: string, cap = NAME_CAP_CODEPOINTS): string {
  const cps = [...label]
  if (cps.length <= cap) return label
  return cps.slice(0, cap).join('') + '…'
}

/**
 * 会话名 = 首条用户消息预览；零轮占位回退短 id / 键尾段（D10）。
 *
 * workbench-interaction-polish 3.2 修复：0-turn 占位（无 prompt、无
 * session_id）在 /api/sessions 行上三者全空（`chat_id` 本就不在该 payload
 * 的词表里）——回退到键的 reference 尾段，`[...undefined]` 曾把整棵 rail
 * 渲染炸掉。模块级导出：/sessions 表格的卡片链接同源复用。
 */
export function fullSessionLabel(row: SessionRow): string {
  return (
    row.prompt_preview ??
    row.session_id_short ??
    row.chat_id ??
    decodeSessionKeyTail(row.encoded_key)
  )
}

/** `web%00web-1709…-4` → `web-1709…-4`（键尾段 = 占位会话的可读短名）。 */
function decodeSessionKeyTail(encodedKey: string): string {
  try {
    const parts = decodeURIComponent(encodedKey).split('\0')
    return parts[parts.length - 1] || encodedKey
  } catch {
    return encodedKey
  }
}

/** unix 秒 → 粗粒度相对时间（离线成因文案用；不引入日期库）。 */
function relativeTime(unixSecs: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs)
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  @state() private sessions: SessionRow[] = []
  /**
   * 焦点会话指针（workbench-conversation-view 3.2）：/api/sessions 响应的
   * `active_session_key`——rail 的「当前」标记看它，不看 location.pathname。
   */
  @state() private focusedKey: string | null = null
  @state() private archivedSessions: ArchiveEntry[] = []
  @state() private expanded: Record<string, boolean> = {}
  @state() private historyOpen = false
  /** 8.4：等待组默认展开（它就是要你看见）。 */
  @state() private waitingOpen = true
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
  /** 注册对话框选定的执行节点（`''` = 本机，隐式；8.1）。 */
  @state() private addNodeId = ''

  // ─── 执行节点可用性（add-remote-execution-node 8.2/8.5）──────────────
  /** `GET /api/nodes` 的节点列表（本机恒在列）。 */
  @state() private nodes: NodeInfo[] = []
  /** 远端注册表是否可得：`false` = 状态未知，**不**等于「没有远端节点」。 */
  @state() private remoteNodesAvailable = true
  @state() private nodesCause: string | null = null

  // Remove project dialog state（workbench-agent-wire-fix 5.1）
  @state() private removeTarget: Project | null = null
  @state() private removeError: string | null = null
  @state() private removing = false

  // ─── New session dialog（workbench-interaction-polish 3.2/D2）──────────
  /** 对话框当前绑定的项目（`null` = 关闭）。唯一创建入口：项目行「+」。 */
  @state() private newSessionTarget: Project | null = null
  /** 创建请求在途（防双击重复创建）。 */
  @state() private creatingSession = false
  /** 创建失败：留在对话框内就地呈现。 */
  @state() private newSessionError: string | null = null

  private fetchSeq = 0
  private unsubscribe?: () => void
  /** 节点可用性轮询定时器（8.2；disconnectedCallback 清理）。 */
  private nodeTimer: number | undefined = undefined
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
    .meta .count { background: var(--sebas-surface-2); border-radius: 999px; padding: 1px 7px; font-weight: 500; font-variant-numeric: tabular-nums; }
    .row.active .meta .count { background: var(--sebas-accent-strong); color: var(--sebas-accent-ink); }
    .wait-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--sebas-status-working); display: inline-block; }
    /* 8.5：项目/会话行上的节点标注。本机也显示（spec：命名每个项目所在的
       节点），但只有非在线态才带告警色。 */
    .node-chip {
      font-family: var(--sebas-font-mono); font-size: 0.66rem; font-weight: 500;
      color: var(--sebas-text-faint); background: var(--sebas-surface-2);
      border-radius: var(--sebas-radius-full); padding: 0 6px;
      max-width: 90px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .node-chip[data-node-status='offline'],
    .node-chip[data-node-status='revoked'],
    .node-chip[data-node-status='unknown'] {
      color: var(--sebas-status-failed);
      background: var(--sebas-status-failed-bg);
    }
    .node-cause {
      margin: 2px 10px 4px; padding: 3px 8px;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-status-failed-bg);
      border: 1px solid var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
      font-size: 0.7rem; line-height: 1.35;
    }
    .row.node-offline .name > span:first-child { color: var(--sebas-text-faint); }
    .row-action:disabled { opacity: 0.3; cursor: not-allowed; }
    .row:hover .row-action:disabled { color: var(--sebas-text-faint); border-color: var(--sebas-border); }
    .session-node {
      font-family: var(--sebas-font-mono); font-size: 0.62rem; color: var(--sebas-text-faint);
      max-width: 72px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    li.session-item.waiting .session-name { color: var(--sebas-status-waiting); }
    .wait-badge {
      font-size: 0.62rem; font-weight: 600; letter-spacing: 0.02em;
      color: var(--sebas-status-waiting); background: var(--sebas-status-waiting-bg);
      border: 1px solid var(--sebas-status-waiting-border);
      border-radius: var(--sebas-radius-full); padding: 0 6px; white-space: nowrap;
    }
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
    .session-dot[data-status='waiting'] { background: var(--sebas-status-waiting); }
    .session-dot[data-status='done'] { background: var(--sebas-status-done); }
    .session-dot[data-status='failed'] { background: var(--sebas-status-failed); }
    .session-dot[data-status='dormant'] { background: var(--sebas-status-dormant); }
    .session-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--sebas-font-mono); font-size: 0.74rem; }
    li.session-item.unreachable .session-name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .row.unreachable .name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .empty { padding: 10px 12px; color: var(--sebas-text-faint); font-size: 0.78rem; }
    /* rail-declutter-unread 2.3：未读徽标——高亮数字（accent 底），99+ 封顶。 */
    .unread-badge {
      flex: 0 0 auto;
      font-size: 0.62rem; font-weight: 700; line-height: 1.4;
      color: var(--sebas-accent-ink); background: var(--sebas-accent-strong);
      border-radius: var(--sebas-radius-full); padding: 0 6px;
      font-variant-numeric: tabular-nums; white-space: nowrap;
    }
    /* 会话行的「…」菜单触发钮：与项目行共用 .row-action 外观，hover/
       focus-within 显现规则在会话行上等价一份（3.2）。 */
    li.session-item wa-dropdown { display: inline-flex; flex: 0 0 auto; }
    .row-actions wa-dropdown { display: inline-flex; }
    li.session-item:hover .row-action,
    li.session-item:focus-within .row-action,
    .row-action:focus-visible { opacity: 1; }
    li.session-item .row-action:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
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
    this.unsubscribe = sharedWs.subscribe(this.refetchBound)
    window.addEventListener('sebas:refetch', this.refetchBound)
    // 8.2：节点离线/回归没有对应的会话事件，靠轮询让项目行与「+」的
    // 可用态**免刷新**翻转。
    this.nodeTimer = window.setInterval(() => { void this.refresh() }, NODE_POLL_MS)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetchBound)
    if (this.nodeTimer !== undefined) {
      window.clearInterval(this.nodeTimer)
      this.nodeTimer = undefined
    }
    super.disconnectedCallback()
  }

  async refresh() {
    const seq = ++this.fetchSeq
    // 节点可用性先取（项目行的离线呈现依赖它）。取不到时如实降级为
    // 「状态不可得」，绝不把「看不见」说成「离线」，更不说成「在线」。
    try {
      const d = await api.nodes()
      if (seq !== this.fetchSeq) return
      this.nodes = d?.nodes ?? []
      this.remoteNodesAvailable = d?.remote_available !== false
      this.nodesCause = d?.cause ?? null
    } catch (e) {
      if (seq !== this.fetchSeq) return
      this.nodes = [{ id: LOCAL_NODE, status: 'online', local: true }]
      this.remoteNodesAvailable = false
      this.nodesCause = e instanceof Error ? e.message : String(e)
    }
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

  /**
   * 一个节点的可判定状态（8.2/8.5）。返回 `status` ∈ online | offline |
   * revoked | unknown，以及不可用时的**成因**。
   *
   * `unknown` 是真实的一档：注册表不可得时我们不知道那台机器通不通，不能
   * 说它离线，也不能默认它在线。本机节点例外——本机在回答这个页面。
   */
  private nodeStatus(nodeId: string | null | undefined): { status: string; cause: string | null } {
    const id = nodeId || LOCAL_NODE
    const found = this.nodes.find((n) => n.id === id)
    if (found) {
      if (found.status === 'online') return { status: 'online', cause: null }
      if (found.status === 'revoked') return { status: 'revoked', cause: '节点凭据已被吊销' }
      const seen = found.last_seen_unix
      return {
        status: found.status,
        cause: seen ? `节点离线（上次在线 ${relativeTime(seen)}）` : '节点离线',
      }
    }
    if (id === LOCAL_NODE) return { status: 'online', cause: null }
    if (!this.remoteNodesAvailable) {
      return {
        status: 'unknown',
        cause: `节点状态不可得${this.nodesCause ? `：${this.nodesCause}` : ''}`,
      }
    }
    return { status: 'unknown', cause: `节点 ${id} 未注册` }
  }

  /** 项目是否注册在一个**可建会话**的节点上（online 才可）。 */
  private nodeOnline(nodeId: string | null | undefined): boolean {
    return this.nodeStatus(nodeId).status === 'online'
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
    // rail-declutter-unread D3：switch 成功 = 聚焦写锚——读锚推进到当前
    // msg_count，徽标清零（无锚点会话自此刻起开始累计未读）。
    writeFocusAnchor(row.encoded_key, row.msg_count)
    if (location.pathname !== '/') navigate('/')
  }

  sessionsFor(id: string) { return this.sessions.filter((r) => r.project_id === id) }

  // ─── New session dialog（workbench-interaction-polish 3.2，design D2）──
  // 项目行「+」是唯一创建入口：打开对话框（agent 必选 + 两级模型 + mode），
  // 确认后 POST /api/sessions 建 0-turn 占位（服务端 set_focus），沿用
  // create 后的就地聚焦链路——create_session 的焦点指针会让 summary 的
  // active_session_key 驱动 composer 进入跟随模式。取消则什么都不发生。
  private openNewSessionDialog(p: Project): void {
    this.newSessionTarget = p
    this.newSessionError = null
  }

  private closeNewSessionDialog(): void {
    this.newSessionTarget = null
    this.newSessionError = null
  }

  private async confirmNewSession(e: CustomEvent<NewSessionDialogConfirm>): Promise<void> {
    const p = this.newSessionTarget
    if (!p || this.creatingSession) return
    this.creatingSession = true
    this.newSessionError = null
    try {
      await api.createSession({
        projectId: p.id,
        agent: e.detail.agent,
        model: e.detail.model,
        mode: e.detail.mode,
      })
      this.closeNewSessionDialog()
      this.onSelect(p.path)
      void this.refresh()
      if (location.pathname !== '/') navigate('/')
    } catch (err) {
      // 失败留在对话框内就地呈现——不假装创建成功。
      this.newSessionError = err instanceof Error ? err.message : String(err)
    } finally {
      this.creatingSession = false
    }
  }

  // ─── Remove project（5.1；rail-declutter-unread D5 预检 + 后端强制）──
  // 菜单项路径：事件不再就地 stopPropagation——让它继续冒泡穿过 dropdown
  // 的 menu（handleMenuClick 负责收起菜单），阻断行的职责在 <wa-dropdown>
  // 本体的 @click 上。
  private openRemoveDialog(_e: Event, p: Project) {
    this.removeTarget = p
    this.removeError = null
  }
  private closeRemoveDialog() { this.removeTarget = null; this.removeError = null }
  /** 项目下非归档会话数（rail-declutter-unread D5 弹窗预检的数据源）。 */
  private liveSessionCountFor(id: string): number {
    return this.sessionsFor(id).length
  }
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
    void e
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
    void e
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
    // 默认选本机（隐式注册的既有行为）。
    this.addNodeId = ''
    const picker = this.shadowRoot?.querySelector('.folder-picker') as any
    if (picker?.reset) void picker.reset()
  }
  private closeAddDialog() { this.addDialogOpen = false; this.addPath = ''; this.addError = null }
  private onFolderSelected(e: CustomEvent) { this.addPath = e.detail.path }
  private async submitAddProject() {
    const path = this.addPath.trim()
    if (!path) { this.addError = '请输入路径'; return }
    // 8.1：节点维度随注册一起走。选定非本机节点时，路径可用性由**该节点**
    // 判定——后端拒绝会点名节点、路径与哪里不对，这里原样呈现。
    const nodeId = this.addNodeId || null
    if (nodeId && !this.nodeOnline(nodeId)) {
      const st = this.nodeStatus(nodeId)
      this.addError = `无法注册到节点 ${nodeId}：${st.cause ?? '节点不可用'}`
      return
    }
    try {
      const p = await api.projects.add(path, nodeId)
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
      // 8.4：等待（含悬空审批）也需要操作员介入，与 queued/failed 同类。
      if (
        r.status_slug === 'queued' ||
        r.status_slug === 'failed' ||
        r.status_slug === 'starting' ||
        r.status_slug === 'waiting' ||
        (r.remote?.parked_approvals ?? 0) > 0
      ) { waiting = true }
    }
    return { count, waiting }
  }

  /**
   * 8.4：等待操作员决定（悬空审批 > 0）的会话。它们单独成组——「在等人」与
   * 「在干活」必须是两个可分辨的集合，把等待埋在项目分组里就等于只有点开
   * 才发现。
   */
  waitingSessions(): SessionRow[] {
    return this.sessions.filter((r) => (r.remote?.parked_approvals ?? 0) > 0)
  }

  // ─── Renderers ──────────────────────────────────────────────────

  private renderSessionRow(row: SessionRow) {
    const fullLabel = fullSessionLabel(row)
    const label = truncateName(fullLabel)
    // 当前标记由焦点指针驱动（3.2）：不再比较 location.pathname。
    const current = this.focusedKey === row.encoded_key
    // rail-declutter-unread 2.3：未读徽标 = msg_count − 共享读锚；0 或负数
    // 不显示，99+ 封顶。
    const unread = unreadCount(row.encoded_key, row.msg_count)
    const badge = unread > UNREAD_BADGE_CAP ? `${UNREAD_BADGE_CAP}+` : String(unread)
    // 8.4：悬空审批 > 0 = 在等人，不是在跑（状态词由后端投影为 waiting，
    // 这里再按 remote 兜一层，老报文/直接 mock 的 remote 也能正确标）。
    const remote = row.remote ?? null
    const waiting = (remote?.parked_approvals ?? 0) > 0 || row.status_slug === 'waiting'
    // 8.5：会话标注所属节点；节点不可用时把成因写在 title 上。
    const nodeId = remote?.node_id ?? null
    const nodeLabel = nodeId ?? (row.project_id ? LOCAL_NODE : null)
    const nodeOffline = remote != null && remote.node_status !== 'online'
    const nodeTitle = nodeOffline
      ? `节点 ${nodeId}：${remote?.node_cause ?? remote?.node_status ?? '不可用'}`
      : nodeLabel
        ? `执行节点 ${nodeLabel}`
        : ''
    return html`
      <li class="session-item ${current ? 'current' : ''} ${waiting ? 'waiting' : ''} ${nodeOffline ? 'node-offline' : ''}" title=${fullLabel} aria-current=${current ? 'true' : 'false'} @click=${() => this.openSession(row)}>
        <span class="session-dot" data-status=${waiting ? 'waiting' : row.status_slug} aria-hidden="true"></span>
        <span class="session-name">${label}</span>
        ${unread > 0 ? html`<span class="unread-badge" data-testid="session-unread" title="${unread} 条未读回复">${badge}</span>` : nothing}
        ${nodeLabel ? html`<span class="session-node" data-testid="session-node" title=${nodeTitle}>${nodeOffline ? '⚠ ' : ''}${nodeLabel}</span>` : nothing}
        ${waiting ? html`<span class="wait-badge" data-testid="session-waiting" title="等待操作员决定（悬空审批 ${remote?.parked_approvals ?? 0}）">等待${(remote?.parked_approvals ?? 0) > 0 ? ` ${remote!.parked_approvals}` : ''}</span>` : nothing}
        <!-- 菜单：触发钮的 click 必须能冒泡到 dropdown 的 trigger slot（打开
             菜单的监听在那里）；阻断行级 click 的位置在 <wa-dropdown> 本体。 -->
        <wa-dropdown placement="bottom-end" @click=${(e: Event) => e.stopPropagation()}>
          <button
            slot="trigger"
            class="row-action"
            title="Session actions"
            aria-label="Session actions for ${fullLabel}"
            aria-haspopup="menu"
          >…</button>
          <wa-dropdown-item value="archive" @click=${(e: Event) => this.archiveSession(e, row.encoded_key)}>归档</wa-dropdown-item>
          <wa-dropdown-item value="close" variant="danger" @click=${(e: Event) => this.closeSession(e, row)}>关闭</wa-dropdown-item>
        </wa-dropdown>
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
    const accessible = info ? info.accessible : true
    const { count, waiting } = this.countsFor(p.id)
    const isActive = this.activePath === p.path
    const isExpanded = this.expanded[p.path] ?? false
    const dragging = this.dragIndex === index
    const dragOver = this.dragOverIndex === index && this.dragIndex !== null && this.dragIndex !== index
    const projectSessions = this.sessionsFor(p.id)
    // 8.2/8.5：项目标注所属节点；节点不可用时阻止新建（composer/rail 同一门禁）
    // 并把成因写出来，而不是等提交失败才说。
    const st = this.nodeStatus(p.node_id)
    const nodeLabel = p.node_id || LOCAL_NODE
    const nodeOk = st.status === 'online'
    const nodeTitle = nodeOk ? `执行节点 ${nodeLabel}` : `节点 ${nodeLabel}：${st.cause ?? st.status}`
    return html`
      <li>
        <div class=${['row', isActive ? 'active' : '', accessible ? '' : 'unreachable', nodeOk ? '' : 'node-offline', dragging ? 'dragging' : '', dragOver ? 'drag-over' : ''].filter(Boolean).join(' ')} draggable="true" aria-current=${isActive ? 'true' : 'false'} aria-expanded=${isExpanded ? 'true' : 'false'} @click=${() => this.onSelect(p.path)} @dragstart=${(e: DragEvent) => this.onDragStart(e, index)} @dragover=${(e: DragEvent) => this.onDragOver(e, index)} @dragleave=${() => this.onDragLeave(index)} @drop=${(e: DragEvent) => this.onDrop(e, index)} @dragend=${() => this.onDragEnd()}>
          <span class="name">
            <span>${p.name}</span>
            <span class="node-chip" data-testid="project-node" data-node-status=${st.status} title=${nodeTitle}>${nodeOk ? '' : '⚠ '}${nodeLabel}</span>
            ${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}
          </span>
          <span class="meta">${count > 0 ? html`<span class="count">${count}</span>` : nothing}</span>
          <span class="row-actions">
            <!-- rail-declutter-unread 3.1：「…」在前、「+」在后（顺序固定）。
                 移除动作收进「…」菜单（现阶段仅此一项，留扩展位）；分支名
                 不再显示，可达性探测保留（loadBranch/删除线告警不变）。
                 触发钮的 click 必须能冒泡到 dropdown 的 trigger slot（打开
                 菜单的监听在那里）；阻断行级 click 的位置在 <wa-dropdown>。 -->
            <wa-dropdown placement="bottom-end" @click=${(e: Event) => e.stopPropagation()}>
              <button
                slot="trigger"
                class="row-action"
                title="Project actions"
                aria-label="Project actions for ${p.name}"
                aria-haspopup="menu"
              >…</button>
              <wa-dropdown-item value="remove" @click=${(e: Event) => this.openRemoveDialog(e, p)}>移除项目</wa-dropdown-item>
            </wa-dropdown>
            <button
              class="row-action"
              title=${nodeOk ? `New session in ${p.name}` : `无法新建会话：${st.cause ?? `节点 ${nodeLabel} 不可用`}`}
              aria-label="New session in ${p.name}"
              ?disabled=${!nodeOk}
              @click=${(e: Event) => {
                e.stopPropagation()
                this.openNewSessionDialog(p)
              }}
            >+</button>
          </span>
        </div>
        ${nodeOk ? nothing : html`<div class="node-cause" data-testid="project-node-cause">节点 ${nodeLabel} 不可用：${st.cause ?? st.status}</div>`}
        ${isExpanded ? (projectSessions.length > 0 ? html`<ul class="sessions">${projectSessions.map((r) => this.renderSessionRow(r))}</ul>` : html`<div class="empty">该项目暂无会话</div>`) : nothing}
      </li>`
  }

  /**
   * 8.4：等待操作员决定的会话单独成组（跨项目）。它们本来也会出现在各自
   * 项目的展开列表里，但「有人在等你」不该要求操作员逐个展开才发现。
   */
  private renderWaiting() {
    const waiting = this.waitingSessions()
    if (waiting.length === 0) return nothing
    return html`
      <div class="group-section waiting-group">
        <div class="group-head" role="button" tabindex="0" aria-expanded=${this.waitingOpen ? 'true' : 'false'} @click=${() => (this.waitingOpen = !this.waitingOpen)} @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.waitingOpen = !this.waitingOpen } }}>
          <span class="chevron ${this.waitingOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>Waiting on you</span><span class="group-count">${waiting.length}</span>
        </div>
        ${this.waitingOpen ? html`<ul class="sessions">${waiting.map((r) => this.renderSessionRow(r))}</ul>` : nothing}
      </div>`
  }

  private renderHistory() {
    // rail-declutter-unread D7：History 按归档时间倒序（新的在前）。前端
    // 排序，/api/archive 保持插入序返回（wire 契约不变）。
    const archived = [...this.archivedSessions].sort((a, b) => b.archived_at - a.archived_at)
    if (archived.length === 0) return nothing
    return html`
      <div class="group-section">
        <div class="group-head" role="button" tabindex="0" aria-expanded=${this.historyOpen ? 'true' : 'false'} @click=${() => (this.historyOpen = !this.historyOpen)} @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.historyOpen = !this.historyOpen } }}>
          <span class="chevron ${this.historyOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>History</span><span class="group-count">${archived.length}</span>
        </div>
        ${this.historyOpen ? html`<ul class="sessions">${archived.map((a) => this.renderArchivedSessionRow(a))}</ul>` : nothing}
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
      ${this.renderWaiting()}
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
          <!-- 8.1：节点维度。空值 = 本机隐式注册（既有行为）；选远端时路径由
               那台节点判定。远端注册表不可得时如实说明，不假装没有远端节点。 -->
          <wa-select
            label="Execution node"
            data-testid="add-node-select"
            value=${this.addNodeId}
            @change=${(e: any) => (this.addNodeId = e.target.value ?? '')}
          >
            <wa-option value="">local（本机，隐式）</wa-option>
            ${this.nodes
              .filter((n) => !n.local && n.id !== LOCAL_NODE)
              .map((n) => html`<wa-option value=${n.id} ?disabled=${n.status !== 'online'}>${n.status === 'online' ? n.id : `${n.id}（${n.status}）`}</wa-option>`)}
          </wa-select>
          ${this.remoteNodesAvailable
            ? nothing
            : html`<p style="font-size:0.75rem;color:var(--sebas-text-faint);margin:0;">远端节点状态不可得${this.nodesCause ? `：${this.nodesCause}` : ''}</p>`}
          ${this.addError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;" data-testid="add-project-error">${this.addError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="brand" @click=${() => void this.submitAddProject()} ?disabled=${!this.addPath.trim()}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeAddDialog()}>Cancel</wa-button>
      </wa-dialog>

      <wa-dialog label="Remove project" style="--width: 440px;" .open=${this.removeTarget !== null} @wa-hide=${() => this.closeRemoveDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            移除项目 <b>${this.removeTarget?.name ?? ''}</b>？
          </p>
          ${this.removeTarget !== null && this.liveSessionCountFor(this.removeTarget.id) > 0
            ? html`<p
                style="font-size:0.8rem;color:var(--sebas-status-failed);margin:0;"
                data-testid="remove-blocked"
              >
                该项目下仍有 <b>${this.liveSessionCountFor(this.removeTarget.id)}</b>
                个未归档会话——请先在会话行的 <b>…</b> 菜单里归档或关闭它们（共
                ${this.liveSessionCountFor(this.removeTarget.id)} 个），再移除项目。
              </p>`
            : html`<p style="font-size:0.8rem;color:var(--sebas-text-dim);margin:0;">
                此操作只解除注册，可重新添加。
              </p>`}
          ${this.removeError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.removeError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="danger" ?loading=${this.removing} @click=${() => void this.confirmRemoveProject()}>移除</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeRemoveDialog()}>取消</wa-button>
      </wa-dialog>

      <wa-dialog label="Close session" style="--width: 440px;" .open=${this.closeTarget !== null} @wa-hide=${() => this.closeConfirmDialog()}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            关闭会话 <b>${this.closeTarget ? truncateName(fullSessionLabel(this.closeTarget)) : ''}</b>？
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
      </wa-dialog>

      <!-- 创建会话对话框（workbench-interaction-polish D2）：唯一可选 agent
           的地方；项目行「+」打开，确认后由 rail 落 POST /api/sessions。 -->
      <sebas-new-session-dialog
        data-testid="new-session-dialog"
        .open=${this.newSessionTarget !== null}
        .projectId=${this.newSessionTarget?.id ?? null}
        .projectName=${this.newSessionTarget?.name ?? null}
        .defaultAgent=${this.newSessionTarget?.default_agent ?? null}
        .error=${this.newSessionError}
        @dialog-confirm=${(e: CustomEvent<NewSessionDialogConfirm>) =>
          void this.confirmNewSession(e)}
        @dialog-cancel=${() => this.closeNewSessionDialog()}
      ></sebas-new-session-dialog>`
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}