/**
 * Workbench main area (IA v2 + workbench-conversation-view): the SINGLE
 * conversation surface. 项目树已上移到 app-shell 侧栏（sebas-project-rail），
 * 本视图承载——选中项目的头部（名称 + mono 分支 pill + `N sessions · ●
 * active` meta）、聚焦会话头（状态徽章 + chat id + agent 锁 + 会话内模型
 * 选择 + Close/归档——原 session-detail 的会话级操作全部
 * 迁到这里，session-detail 视图已退休）、turn-stream 舞台
 * （<sebas-transcript-view> 把 entries 渲染成两侧交替的对话）与 composer。
 *
 * 无聚焦会话时渲染预览原型的空态。聚焦来源有三条，全部就地渲染本视图：
 * rail switch（app-shell 停在 `/`）、`/sessions/:key` 深链（deepLinkKey，
 * 读 detail 即设置服务端焦点指针）、创建会话（服务端 set_focus）。Live
 * updates arrive over the shared WebSocket; a reconnect triggers a refetch.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type NodeInfo, type NodesResponse, type PendingSubmission, type Project, type SessionDetail, type SessionRow, type Summary } from '../api/client.js'
import type { WsEvent } from '../api/ws.js'
import { sharedWs } from '../api/shared-ws.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '../components/status-badge.js'
import '../components/review-card.js'
import '../components/pending-stack.js'
import './transcript-view.js'
import './workbench-composer.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

/** 本机节点标识（与后端 projects::LOCAL_NODE_ID 同一词表）。 */
const LOCAL_NODE = 'local'

/** 节点可用性轮询周期（8.2：节点回归/掉线免刷新反映到 composer 门禁）。 */
const NODE_POLL_MS = 10_000

@customElement('sebas-dashboard')
export class SebasDashboard extends LitElement {
  @state() private data: Summary | null = null
  @state() private allRows: SessionRow[] = []
  @state() private error = ''
  /**
   * Focused session's full detail (conversation entries + encoded key) for
   * the inline turn stream. Loaded from /api/sessions/:key whenever the
   * effective focus key changes; `null` while loading or when nothing is
   * focused.
   */
  @state() private focusedDetail: SessionDetail | null = null
  /** Set when the focused detail fetch failed (session vanished mid-flight). */
  @state() private focusedUnavailable = false
  /**
   * `/sessions/:key` 深链参数（app-shell 传入）：focus 指针尚未到达（summary
   * 未刷新）时先以它取 detail——读 detail 即在服务端设置焦点指针（display
   * pointer only）。会话关闭/焦点他移后由 effective-focus 收敛逻辑清空。
   */
  @property({ attribute: false }) deepLinkKey: string | null = null
  /**
   * Selected project path — owned by the app-shell（侧栏项目树驱动），
   * 这里只消费。`null` = 未选择项目。The selection only affects the
   * workbench main area — never the focused-session pointer or any
   * session state.
   */
  @property({ attribute: false })
  selectedPath: string | null = null
  /**
   * Branch of the selected project, fetched lazily for the project header
   * pill. Cleared on every selection change so a slow response can never
   * paint the previous project's branch; left null (pill hidden) when
   * lookup fails.
   */
  @state() private selectedBranch: string | null = null
  /**
   * Provider label rendered next to the composer (e.g. "anthropic / claude").
   * `null` while loading; `"no provider configured"` if no providers are
   * registered; `"provider status unavailable"` when the provider source
   * itself cannot be reached (honest degradation, fix-webui-detached-status).
   */
  @state() private providerLabel: string | null = null
  /**
   * 中程切换聚焦会话模型（add-acp-model-selection 语义）：非空 = 请求已发出，
   * 等事件回流。
   */
  @state() private modelSwitching = false
  /** Close 确认对话框（session-detail 迁移；workbench-turn-queue：点名丢弃条数）。 */
  @state() private confirmClose = false
  private unsubscribe?: () => void
  /**
   * 8.2：节点可用性轮询（节点上下线没有对应的会话事件）。composer 的节点门禁
   * 因此**免刷新**恢复/收紧。disconnectedCallback 清理。
   */
  private nodeTimer: number | undefined = undefined
  /**
   * 创建会话不再跳转（workbench-conversation-view 3.x：工作台是唯一对话面）
   * ——服务端 create 已 set_focus，就地重取让新会话的对话出现。
   */
  private onComposerCreated = (): void => {
    // 新会话创建后服务端焦点指向它：清掉深链参数，让焦点指针驱动视图。
    this.deepLinkKey = null
    this.refetch()
  }
  /**
   * add-composer-agent-binding：跟随模式下 composer 发出消息/切模型后乐观
   * 重取聚焦 detail——transcript 不等下一个 WS/summary 周期就能反映本轮。
   */
  private onComposerSent = (): void => {
    this.loadFocused(this.effectiveFocusKey())
  }
  /**
   * workbench-turn-queue 7.3：会话终结时未执行的待生效提交（一次性提示的
   * 数据源）。`session.pending_dropped` 帧携带逐条标注。提示归属刚终结的
   * 会话：焦点清空（会话已移除）时保留——堆叠区随会话消失，提示是唯一
   * 记录；聚焦切换到别的会话才清除。
   */
  @state() private droppedPending: PendingSubmission[] | null = null
  @state() private droppedPendingFor: string | null = null

  // ─── 执行节点可用性（add-remote-execution-node 8.2/8.5）─────────────────
  @state() private nodes: NodeInfo[] = []
  /** 远端注册表是否可得（false = 状态未知，≠「没有远端节点」）。 */
  @state() private remoteNodesAvailable = true
  @state() private nodesCause: string | null = null

  private onWsEvent = (ev: WsEvent): void => {
    if (ev.type === 'session.pending_dropped' && ev.session_id === this.data?.active_session_key) {
      this.droppedPending = ev.dropped
      this.droppedPendingFor = ev.session_id
    }
  }

  static styles = [
    viewStyles,
    css`
      /* 满幅工作台面板（预览原型 workbench 同款）：宿主随 outlet
         拉伸，project-header 钉顶、turn-stream 吃满余高、composer 钉底。 */
      :host {
        display: flex;
        flex: 1;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
      }
      /* 项目头部（对齐预览原型 workbench 的 project-header）：钉在面板
         顶部的通栏条——border-bottom 分隔、无边框圆角/阴影/外边距，
         右侧聚焦会话深链与 "N sessions · ● active" meta 一起右对齐。 */
      .project-header {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        background: var(--sebas-surface);
        border: none;
        border-bottom: 1px solid var(--sebas-border);
        border-radius: 0;
        box-shadow: none;
        padding: var(--sebas-space-3) var(--sebas-space-5);
        margin-bottom: 0;
        flex-shrink: 0;
      }
      .project-header .path {
        font-weight: 600;
        font-size: 0.95rem;
        color: var(--sebas-text-bright);
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .project-header .path.muted {
        color: var(--sebas-text-faint);
        font-weight: 500;
      }
      /* 8.5：项目头部/会话头部的节点标注。 */
      .node-chip {
        font-family: var(--sebas-font-mono);
        font-size: 0.72rem;
        color: var(--sebas-text-faint);
        background: var(--sebas-surface-2);
        border-radius: var(--sebas-radius-full);
        padding: 1px 8px;
        white-space: nowrap;
      }
      .node-chip[data-node-status='offline'],
      .node-chip[data-node-status='revoked'],
      .node-chip[data-node-status='unknown'] {
        color: var(--sebas-status-failed);
        background: var(--sebas-status-failed-bg);
      }
      /* 8.3：mode 标签与 ungated 标记。 */
      .session-head .node-tag[data-node-status='offline'],
      .session-head .node-tag[data-node-status='revoked'],
      .session-head .node-tag[data-node-status='gone'],
      .session-head .node-tag[data-node-status='terminated'] {
        color: var(--sebas-status-failed);
      }
      .mode-tag {
        font-family: var(--sebas-font-mono);
        font-size: 0.7rem;
        color: var(--sebas-text-dim);
        background: var(--sebas-surface-2);
        border-radius: var(--sebas-radius-full);
        padding: 0 7px;
      }
      .mode-tag b {
        color: var(--sebas-status-failed);
        font-weight: 600;
      }
      .ungated {
        font-size: 0.68rem;
        font-weight: 600;
        letter-spacing: 0.03em;
        text-transform: uppercase;
        color: var(--sebas-status-failed);
        background: var(--sebas-status-failed-bg);
        border: 1px solid var(--sebas-status-failed-border);
        border-radius: var(--sebas-radius-full);
        padding: 0 7px;
      }
      .parked-banner {
        margin: var(--sebas-space-2) var(--sebas-space-5) 0;
      }
      .branch-pill {
        font-family: var(--sebas-font-mono);
        font-size: 0.75rem;
        color: var(--sebas-accent);
        background: var(--sebas-accent-soft);
        border-radius: var(--sebas-radius-full);
        padding: 1px 10px;
        white-space: nowrap;
      }
      .project-meta {
        margin-left: auto;
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        font-size: 0.8rem;
        color: var(--sebas-text-dim);
        font-variant-numeric: tabular-nums;
      }
      .project-meta .meta-item {
        display: flex;
        align-items: center;
        gap: 5px;
      }
      .project-meta .meta-sep {
        color: var(--sebas-text-faint);
      }
      .project-meta .active-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        display: inline-block;
        background: var(--sebas-status-dormant);
      }
      .project-meta .meta-item.is-active .active-dot {
        background: var(--sebas-status-working);
      }
      /* 聚焦会话深链（原 spotlight 卡片折叠进 header 的右段）：mono
         chat id + 状态徽章 + 前箭头，低调、悬停转 accent。 */
      .focused-link {
        display: inline-flex;
        align-items: center;
        gap: var(--sebas-space-2);
        margin-left: var(--sebas-space-2);
        padding-left: var(--sebas-space-3);
        border-left: 1px solid var(--sebas-border);
        font-size: 0.78rem;
        color: var(--sebas-text-faint);
        text-decoration: none;
        transition: color var(--sebas-dur) var(--sebas-ease);
      }
      .focused-link:hover {
        color: var(--sebas-accent);
      }
      .focused-link:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      .focused-link .fkey {
        font-family: var(--sebas-font-mono);
        font-size: 0.8rem;
        color: var(--sebas-text-dim);
        transition: color var(--sebas-dur) var(--sebas-ease);
      }
      .focused-link:hover .fkey {
        color: var(--sebas-accent);
      }
      .focused-link .arrow {
        font-size: 0.85rem;
      }
      /* ── 聚焦会话头（session-detail 迁移，3.3）──状态色左缘条 + 徽章 +
         身份 + 只读 agent 锁 + 会话内模型选择 + Close/归档动作。 */
      .session-head {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        flex-shrink: 0;
        padding: var(--sebas-space-2) var(--sebas-space-5);
        border-left: 3px solid var(--sebas-status-dormant);
        border-bottom: 1px solid var(--sebas-border);
        background: var(--sebas-surface);
      }
      .session-head[data-status='starting'] {
        border-left-color: var(--sebas-status-starting);
      }
      .session-head[data-status='queued'] {
        border-left-color: var(--sebas-status-queued);
      }
      .session-head[data-status='working'] {
        border-left-color: var(--sebas-status-working);
      }
      .session-head[data-status='done'] {
        border-left-color: var(--sebas-status-done);
      }
      .session-head[data-status='failed'] {
        border-left-color: var(--sebas-status-failed);
      }
      .session-head[data-status='dormant'] {
        border-left-color: var(--sebas-status-dormant);
      }
      .session-head .ident {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
      }
      .session-head .chat {
        font-family: var(--sebas-font-mono);
        font-size: 0.9rem;
        color: var(--sebas-text-bright);
        overflow-wrap: anywhere;
      }
      .session-head .chat .dim {
        color: var(--sebas-text-faint);
      }
      .session-head .meta {
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
        color: var(--sebas-text-dim);
        font-size: 0.74rem;
        font-variant-numeric: tabular-nums;
      }
      .session-head .meta .mono {
        font-family: var(--sebas-font-mono);
      }
      /* 中程模型选择器：meta 行内的紧凑下拉（add-acp-model-selection）。 */
      .session-head .model-pick {
        display: inline-flex;
        align-items: center;
      }
      .session-head .model-select {
        --wa-select-min-height: 24px;
        font-size: 0.75rem;
        max-width: 260px;
      }
      /* 中程模式切换下拉（add-agent-mode-selection）：与模型选择器同款紧凑
         形态，但 class 独立（测试按 .model-pick 计数，mode 面不得混入）。 */
      .session-head .mode-pick {
        display: inline-flex;
        align-items: center;
      }
      .session-head .actions {
        margin-left: auto;
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
      }
      .session-head .actions a {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 0.85rem;
      }
      /* turn-stream 舞台：聚焦会话的 transcript 面板，随面板 flex 吃满
         余高（滚动由 transcript-view 内部 .scroll 负责，fill 模式去掉
         58vh 封顶）。 */
      .turn-stream-area {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
      }
      /* turn-stream 舞台：无聚焦会话时的预览原型空态（48px glyph 圆）。 */
      .empty-stream {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--sebas-space-3);
        padding: var(--sebas-space-10) var(--sebas-space-6);
        color: var(--sebas-text-dim);
        text-align: center;
        flex: 1;
      }
      .empty-stream .glyph {
        display: grid;
        place-items: center;
        width: 48px;
        height: 48px;
        border-radius: var(--sebas-radius-full);
        background: var(--sebas-surface-2);
        border: 1px solid var(--sebas-border);
        color: var(--sebas-text-faint);
      }
      .empty-stream .title {
        font-weight: 600;
        font-size: 1rem;
        color: var(--sebas-text-bright);
      }
      .empty-stream .hint {
        font-size: 0.85rem;
        max-width: 36ch;
        margin: 0;
      }
      /* Composer 底座：钉在面板底部的通栏（预览原型 composer-area
         同款），内壳 18px 圆角 shell 由 workbench-composer 自绘。 */
      .composer-area {
        border-top: 1px solid var(--sebas-border);
        background: var(--sebas-bg);
        flex-shrink: 0;
        padding: 0 var(--sebas-space-5) var(--sebas-space-4);
      }
      /* 审批卡贴在输入框之上：整体限高内部滚动，卡再多也不挤占对话区。 */
      .composer-area sebas-review-cards {
        display: block;
        max-height: min(280px, 35vh);
        overflow-y: auto;
        margin-bottom: var(--sebas-space-2);
      }
      .skel-line.w60 {
        width: 60%;
      }
      .skel-line.w25 {
        width: 25%;
      }
      .dialog-body {
        margin: 0;
        color: var(--sebas-text);
        line-height: 1.55;
      }
      .dialog-body .discard-note {
        color: var(--sebas-status-failed);
        font-size: 0.8rem;
        margin: var(--sebas-space-2) 0 0;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.refetch()
    this.unsubscribe = sharedWs.subscribe((ev) => {
      this.onWsEvent(ev)
      this.refetch()
    })
    window.addEventListener('sebas:refetch', this.refetch)
    this.nodeTimer = window.setInterval(() => { void this.loadNodes() }, NODE_POLL_MS)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetch)
    if (this.nodeTimer !== undefined) {
      window.clearInterval(this.nodeTimer)
      this.nodeTimer = undefined
    }
    super.disconnectedCallback()
  }

  protected willUpdate(changed: PropertyValues): void {
    // 侧栏选中项目切换 → 重取分支（面板 pill 用）。
    if (changed.has('selectedPath')) this.loadSelectedBranch()
    // 深链参数变化（/sessions/A → /sessions/B 复用同一元素）：立即按新 key
    // 取 detail（读即设服务端焦点）。
    if (changed.has('deepLinkKey')) this.loadFocused(this.effectiveFocusKey())
  }

  /**
   * 生效的聚焦 key：深链优先（URL 决定视图——读 detail 即设置服务端焦点
   * 指针，summary 随后收敛到同一会话）；无深链时由 summary 的焦点指针驱动
   * （rail switch / 创建会话就地生效）。
   */
  private effectiveFocusKey(): string | null {
    return this.deepLinkKey ?? this.data?.active_session_key ?? null
  }

  private refetch = (): void => {
    void this.loadNodes()
    api
      .projects.list()
      .then((d) => {
        this.projects = d.projects
      })
      .catch(() => {
        /* 分组降级：列表不可得时保持旧值 */
      })
    api
      .summary()
      .then((d) => {
        this.data = d
        this.error = ''
        this.loadFocused(this.effectiveFocusKey())
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
        // fix-webui-detached-status：真源不可用（providers_available=false）
        // 如实呈现"状态不可用"，与"真的没配 provider"区分开。
        if ((s.router as unknown as { providers_available?: boolean })?.providers_available === false) {
          this.providerLabel = 'provider status unavailable'
        } else {
          const first = s.router?.providers?.[0]
          this.providerLabel = first ? first.name : 'no provider configured'
        }
      })
      .catch(() => {
        /* leave existing label in place */
      })
  }

  /**
   * 8.2：拉取节点可用性。`api.nodes` 缺失（老后端 / 测试替身）时如实降级为
   * 「状态不可得」，**不**当成「没有远端节点」。
   */
  private async loadNodes(): Promise<void> {
    const fn = (api as { nodes?: () => Promise<NodesResponse> }).nodes
    if (typeof fn !== 'function') {
      this.nodes = [{ id: LOCAL_NODE, status: 'online', local: true }]
      this.remoteNodesAvailable = false
      this.nodesCause = '此后端不提供节点可用性'
      return
    }
    try {
      const d = await fn()
      this.nodes = d?.nodes ?? []
      this.remoteNodesAvailable = d?.remote_available !== false
      this.nodesCause = d?.cause ?? null
    } catch (e) {
      this.nodes = [{ id: LOCAL_NODE, status: 'online', local: true }]
      this.remoteNodesAvailable = false
      this.nodesCause = e instanceof Error ? e.message : String(e)
    }
  }

  /** 一个节点的可判定状态（online | offline | revoked | unknown）+ 成因。 */
  private nodeStatus(nodeId: string | null | undefined): { status: string; cause: string | null } {
    const id = nodeId || LOCAL_NODE
    const found = this.nodes.find((n) => n.id === id)
    if (found) {
      if (found.status === 'online') return { status: 'online', cause: null }
      if (found.status === 'revoked') return { status: 'revoked', cause: '节点凭据已被吊销' }
      return { status: found.status, cause: `节点离线${found.last_seen_unix ? `（上次在线 ${found.last_seen_unix}）` : ''}` }
    }
    if (id === LOCAL_NODE) return { status: 'online', cause: null }
    if (!this.remoteNodesAvailable) {
      return { status: 'unknown', cause: `节点状态不可得${this.nodesCause ? `：${this.nodesCause}` : ''}` }
    }
    return { status: 'unknown', cause: `节点 ${id} 未注册` }
  }

  /**
   * 8.2：选中项目的节点门禁。非空 = composer 必须阻止提交并说明成因。
   * 本机项目永不为空（本机在回答这个页面）。
   */
  private selectedNodeGate(): { nodeId: string; status: string; cause: string } | null {
    const p = this.projects.find((x) => x.path === this.selectedPath)
    if (!p) return null
    const nodeId = p.node_id || LOCAL_NODE
    const st = this.nodeStatus(nodeId)
    if (st.status === 'online') return null
    return { nodeId, status: st.status, cause: st.cause ?? st.status }
  }

  private renderLoading() {
    return html`
      <div class="panel">
        ${[0, 1, 2, 3].map(
          () => html`
            <div class="skel-row">
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
          <button class="retry-btn" @click=${() => this.refetch()}>重试</button>
        </div>
      `
    if (!this.data) return this.renderLoading()
    const d = this.data
    const rows = this.rowsForSelected()
    const hasActive = rows.some((r) => r.status_slug === 'working')
    const focusKey = this.effectiveFocusKey()
    const nodeGate = this.selectedNodeGate()
    const selectedNodeId = this.projects.find((x) => x.path === this.selectedPath)?.node_id || LOCAL_NODE
    const projectName = this.selectedPath
      ? (this.selectedPath.split('/').filter(Boolean).pop() ?? this.selectedPath)
      : null
    return html`
      <header class="project-header">
        ${projectName
          ? html`
              <span class="path" title=${this.selectedPath ?? ''}>${projectName}</span>
              <span class="node-chip" data-testid="header-node" data-node-status=${nodeGate?.status ?? 'online'} title=${nodeGate ? `节点 ${nodeGate.nodeId} 不可用：${nodeGate.cause}` : `执行节点 ${selectedNodeId}`}>${nodeGate ? '⚠ ' : ''}${selectedNodeId}</span>
              ${this.selectedBranch
                ? html`<span class="branch-pill">${this.selectedBranch}</span>`
                : nothing}
            `
          : html`<span class="path muted">No project selected</span>`}
        <span class="project-meta">
          ${projectName
            ? html`
                <span class="meta-item">${rows.length} sessions</span>
                <span class="meta-sep" aria-hidden="true">·</span>
                <span class="meta-item ${hasActive ? 'is-active' : ''}">
                  <span class="active-dot"></span>${hasActive ? 'active' : 'idle'}
                </span>
              `
            : nothing}
          ${d.active_session
            ? html`
                <a
                  class="focused-link"
                  href=${`/sessions/${d.active_session.encoded_key}`}
                  title="Focused session"
                >
                  <span class="fkey">${d.active_session.chat_id}</span>
                  <sebas-status-badge
                    slug=${d.active_session.status_slug}
                    label=${d.active_session.status_label}
                    glyph=${d.active_session.status_glyph}
                  ></sebas-status-badge>
                  <span class="arrow">${icon('forward', 13)}</span>
                </a>
              `
            : nothing}
        </span>
      </header>

      ${focusKey
        ? this.renderTurnStream()
        : html`
            <div class="empty-stream">
              <span class="glyph">${icon('message', 20)}</span>
              <span class="title">No session focused</span>
              <p class="hint">
                Pick a session from the sidebar tree — or start a new one with the composer below.
              </p>
            </div>
          `}

      <div class="composer-area">
        <sebas-review-cards .sessionKey=${focusKey}></sebas-review-cards>
        <sebas-pending-stack
          .sessionKey=${focusKey}
          .pending=${this.focusedDetail?.pending ?? []}
          .dropped=${this.droppedPending}
          @pending-changed=${this.onComposerSent}
        ></sebas-pending-stack>
        <sebas-workbench-composer
          .projectDir=${this.selectedPath}
          .providerLabel=${this.providerLabel}
          .sessionKey=${focusKey}
          .nodeBlocked=${this.selectedNodeGate()}
          .agentKind=${this.focusedDetail?.agent_kind ?? d.active_session?.agent_kind ?? null}
          .sessionModels=${this.focusedDetail?.available_models ?? d.active_session?.available_models ?? []}
          .currentModel=${this.focusedDetail?.current_model ?? d.active_session?.current_model ?? null}
          @composer-created=${this.onComposerCreated}
          @composer-sent=${this.onComposerSent}
        ></sebas-workbench-composer>
      </div>
    `
  }

  private rowsForSelected(): SessionRow[] {
    if (this.selectedPath === null) return []
    return this.allRows.filter((r) => r.project_id === this.selectedProjectId)
  }

  /** 选中项目的稳定 id（workbench-agent-wire-fix 2.5）：会话行以它分组。 */
  private get selectedProjectId(): string | null {
    return this.projects.find((p) => p.path === this.selectedPath)?.id ?? null
  }

  private projects: Project[] = []

  /**
   * Inline turn stream data: fetch the focused session's detail. `null`
   * key clears the stage; stale responses (focus moved on while in flight)
   * are dropped so the stream never shows a session that is no longer
   * focused.
   */
  private loadFocused(key: string | null): void {
    // 7.3：焦点清空（会话移除）时保留提示；切到别的会话才清除。
    if (key !== null && key !== this.droppedPendingFor) this.droppedPending = null
    if (!key) {
      this.focusedDetail = null
      this.focusedUnavailable = false
      return
    }
    api
      .session(key)
      .then((d) => {
        if (this.effectiveFocusKey() === d.encoded_key) {
          this.focusedDetail = d
          this.focusedUnavailable = false
        }
      })
      .catch(() => {
        if (this.effectiveFocusKey() === key) {
          this.focusedDetail = null
          this.focusedUnavailable = true
        }
      })
  }

  /**
   * Inline conversation 舞台：聚焦会话头（状态/身份/Close/归档，3.3 迁移）
   * + 就地的对话（复用 `<sebas-transcript-view fill>`，内部滚动/未读 seam
   * 均归它管）。detail 尚在途时给骨架，取数失败（会话恰好被关闭）给一条
   * 温和空态而不是报错。审批卡（review-cards）不再在此渲染——它贴在
   * composer 之上（.composer-area 内），不再把整条对话往下推。
   */
  private renderTurnStream() {
    const d = this.focusedDetail
    const key = this.effectiveFocusKey()!
    return html`
      <div class="turn-stream-area" aria-label="Focused session conversation">
        ${d && d.encoded_key === key
          ? html`
              ${this.renderSessionHead(d)}
              ${(d.remote?.parked_approvals ?? 0) > 0
                ? html`<div
                    class="callout callout-warning parked-banner"
                    role="status"
                    data-testid="parked-approvals"
                  >
                    ${icon('alert')}<span
                      >等待操作员决定：<b>${d.remote!.parked_approvals}</b> 条悬空审批未决——该会话在等待，不在运行。决定入口就在下方。</span
                    >
                  </div>`
                : nothing}
              ${d.entries.length === 0
                ? html`
                    <div class="empty-stream">
                      <span class="glyph">${icon('message', 20)}</span>
                      <span class="title">Nothing yet</span>
                      <p class="hint">
                        The conversation starts when the next turn begins — say hello below.
                      </p>
                    </div>
                  `
                : html`<sebas-transcript-view
                    fill
                    .entries=${d.entries}
                    sessionKey=${d.encoded_key}
                  ></sebas-transcript-view>`}
            `
          : this.focusedUnavailable
            ? html`
                <div class="empty-stream">
                  <span class="glyph">${icon('message', 20)}</span>
                  <span class="title">Session unavailable</span>
                  <p class="hint">The focused session could not be loaded.</p>
                </div>
              `
            : html`
                ${[0, 1, 2].map(
                  () => html`
                    <div class="skel-row">
                      <div class="skel skel-line" style="width:24%"></div>
                      <div class="skel skel-line" style="width:52%"></div>
                    </div>
                  `,
                )}
              `}
      </div>
    `
  }

  /**
   * 聚焦会话头（3.3，从 session-detail 迁移）：状态徽章 + chat id +
   * session_id/agent 锁/last active + 会话内模型选择（available_models 非空
   * 才显示）+ Close/归档动作。
   */
  private renderSessionHead(d: SessionDetail) {
    // 8.3/8.4/8.5：远端会话的节点、desired/effective mode、悬空审批。
    const remote = d.remote ?? null
    const nodeId = remote?.node_id ?? LOCAL_NODE
    const nodeOffline = remote != null && remote.node_status !== 'online'
    // （add-agent-mode-selection）mode 对本机/远端会话同通道呈现：远端来自
    // remote 视图（节点回报），本机来自 detail 顶层字段（argv 应用值/
    // ModeChanged）——两处同源（core 侧投影/映射）。
    const desired = remote?.desired_mode ?? d.desired_mode ?? null
    const effective = remote?.effective_mode ?? d.effective_mode ?? null
    const parked = remote?.parked_approvals ?? 0
    const waiting = parked > 0
    // 执行体强制不了时两个值都显示并说明；绝不只显示期望值假装已生效。
    const modeDiffers = !!desired && !!effective && desired !== effective
    const ungated = effective === 'auto' || (desired === 'auto' && effective === null)
    return html`
      <div class="session-head" data-status=${waiting ? 'waiting' : d.status_slug}>
        <sebas-status-badge
          slug=${waiting ? 'waiting' : d.status_slug}
          label=${waiting ? 'Waiting' : d.status_label}
          glyph=${waiting ? '⏸' : d.status_glyph}
        ></sebas-status-badge>
        <div class="ident">
          <span class="chat"
            >${d.chat_id}${d.thread_id
              ? html`<span class="dim"> · ${d.thread_id}</span>`
              : nothing}</span
          >
          <span class="meta">
            ${d.session_id
              ? html`<span class="mono" title=${d.session_id}>${d.session_id.slice(0, 12)}</span>`
              : nothing}
            <span
              class="mono"
              data-testid="agent-lock"
              title="Agent is immutable — chosen when the session was created"
              >🔒 ${d.agent_kind ?? 'default agent'}</span
            >
            <!-- 8.5：所属执行节点；不可用时点名节点与成因。 -->
            <span
              class="mono node-tag"
              data-testid="session-head-node"
              data-node-status=${remote?.node_status ?? 'local'}
              title=${nodeOffline
                ? `节点 ${nodeId} 不可用：${remote?.node_cause ?? remote?.node_status ?? '不可用'}`
                : `执行节点 ${nodeId}`}
              >${nodeOffline ? '⚠ ' : ''}${nodeId}</span
            >
            <!-- 8.3：desired vs effective。不同 = 执行体无法强制，必须说出来；
                 auto = ungated，与需要审批的会话视觉上分开。 -->
            ${desired || effective
              ? html`<span
                  class="mode-tag"
                  data-testid=${modeDiffers ? 'mode-mismatch' : 'session-mode'}
                  data-mode=${effective ?? desired}
                  title=${modeDiffers
                    ? `期望 mode ${desired}，执行体实际强制 ${effective}`
                    : `mode ${effective ?? desired}`}
                >
                  ${modeDiffers
                    ? html`mode ${desired} → 实际 ${effective}<b>（执行体无法强制）</b>`
                    : html`mode ${effective ?? desired}`}
                </span>`
              : nothing}
            ${ungated
              ? html`<span class="ungated" data-testid="session-ungated" title="auto：该机器交给 agent 自主执行，不产生审批">ungated</span>`
              : nothing}
            <!-- （add-agent-mode-selection）mode 切换入口：提交走
                 POST /api/sessions/{key}/mode；执行体拒绝时错误经事件流
                 呈现，mode 标签保持原值。0-turn 占位（无 session_id）不可
                 切——会话还没建立，mode 由创建表单决定。 -->
            ${d.session_id
              ? html`<span class="mode-pick">
                  <wa-select
                    class="mode-select"
                    size="xs"
                    hoist
                    value=${desired ?? ''}
                    ?disabled=${this.modeSwitching}
                    aria-label="Session mode"
                    data-testid="mode-switch"
                    @change=${(e: Event) => {
                      const v = (e as unknown as { target: { value: string } }).target.value
                      if (v) void this.setMode(d.encoded_key, v)
                    }}
                  >
                    <wa-option value="ask">ask</wa-option>
                    <wa-option value="edit">edit</wa-option>
                    <wa-option value="allow">allow</wa-option>
                    <wa-option value="auto">auto</wa-option>
                  </wa-select>
                </span>`
              : nothing}
            <span>last active ${d.last_active}</span>
            ${d.available_models && d.available_models.length > 0
              ? html`<span class="model-pick">
                  <!-- Web Awesome 3.x 派发标准 change 事件（不派发 wa-change）。 -->
                  <wa-select
                    class="model-select"
                    size="xs"
                    hoist
                    value=${d.current_model ?? ''}
                    ?disabled=${this.modelSwitching}
                    aria-label="Session model"
                    @change=${(e: Event) => {
                      const v = (e as unknown as { target: { value: string } }).target.value
                      if (v) void this.setModel(d.encoded_key, v)
                    }}
                  >
                    ${d.available_models.map((m) => html`<wa-option value=${m}>${m}</wa-option>`)}
                  </wa-select>
                </span>`
              : nothing}
          </span>
        </div>
        <div class="actions">
          <a href="/sessions">All sessions</a>
          <wa-button
            size="s"
            appearance="outlined"
            aria-label="Archive this session"
            @click=${() => void this.archiveFocused()}
            >Archive</wa-button
          >
          <wa-button
            size="s"
            variant="danger"
            appearance="outlined"
            aria-label="Close this session"
            @click=${() => (this.confirmClose = true)}
            >Close</wa-button
          >
        </div>
      </div>
      <wa-dialog label="Close session" ?open=${this.confirmClose}>
        <p class="dialog-body">
          Closing will terminate the agent child process and clear this chat's
          permission allowlist. This cannot be undone.
          ${d.pending.length > 0
            ? html`<span class="discard-note" data-testid="close-discards-pending"
                >将丢弃 <b>${d.pending.length}</b> 条待执行消息，它们不会被执行。</span
              >`
            : nothing}
        </p>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.confirmClose = false)}
          >Cancel</wa-button
        >
        <wa-button slot="footer" variant="danger" @click=${() => void this.doClose(d.encoded_key)}
          >Close session</wa-button
        >
      </wa-dialog>
    `
  }

  /**
   * 中程切换聚焦会话模型（add-acp-model-selection 2.3）：把选择送后端 →
   * 驱动发 `session/set_config_option{configId:"model"}`。wire 层失败（agent
   * 拒绝无效模型）经非 terminal Error 事件回流；快照的 `current_model` 在
   * `ModelChanged` 到达后由 refetch 刷新。
   */
  /** 中程切换会话权限模式时的在途标记（add-agent-mode-selection）。 */
  @state() private modeSwitching = false

  /** （add-agent-mode-selection）mode 切换：命令送达后重取快照（effective
   * 随 ModeChanged 落定；拒绝则错误事件呈现、标签保持原值）。 */
  private async setMode(key: string, mode: string): Promise<void> {
    if (this.modeSwitching) return
    this.modeSwitching = true
    try {
      await api.setSessionMode(key, mode)
      await new Promise((r) => setTimeout(r, 400))
      this.refetch()
    } finally {
      this.modeSwitching = false
    }
  }

  private async setModel(key: string, modelId: string): Promise<void> {
    if (this.modelSwitching) return
    this.modelSwitching = true
    try {
      await api.setSessionModel(key, modelId)
      // 命令已送达驱动；连刷两次以捕捉 ModelChanged 之后的快照更新。
      await new Promise((r) => setTimeout(r, 400))
      this.refetch()
    } finally {
      this.modelSwitching = false
    }
  }

  /** Close（session-detail 迁移）：关闭后焦点指针随响应收敛，就地重取。 */
  private async doClose(key: string): Promise<void> {
    this.confirmClose = false
    try {
      await api.closeSession(key)
      // 关闭的正是深链会话时清掉深链参数，避免 effective-focus 又指向死会话。
      if (this.deepLinkKey === key) this.deepLinkKey = null
      this.refetch()
    } catch {
      /* close 失败（会话已被别处关闭）：refetch 收敛视图。 */
      this.refetch()
    }
  }

  /** 归档入口（3.3）：会话关闭并移入 /api/archive，就地清焦点重取。 */
  private async archiveFocused(): Promise<void> {
    const key = this.focusedDetail?.encoded_key ?? this.effectiveFocusKey()
    if (!key) return
    try {
      await api.archiveSession(key)
      if (this.deepLinkKey === key) this.deepLinkKey = null
      this.refetch()
    } catch {
      /* 归档失败：refetch 收敛视图。 */
      this.refetch()
    }
  }

  /** 懒加载选中项目的分支（project-header 的 mono pill 用），选中即取，失败不渲染。 */
  private loadSelectedBranch(): void {
    const id = this.selectedProjectId
    const path = this.selectedPath
    this.selectedBranch = null
    if (id === null || path === null) return
    api.projects
      .branch(id)
      .then((info) => {
        // 选中项中途切换时丢弃过期响应，避免显示上一个项目的分支
        if (this.selectedPath === path) this.selectedBranch = info.branch
      })
      .catch(() => {
        /* 分支信息不可得时保持无 pill */
      })
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-dashboard': SebasDashboard
  }
}
