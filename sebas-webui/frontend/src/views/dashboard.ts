/**
 * Workbench main area (IA v2)：项目树已上移到 app-shell 侧栏
 * （sebas-project-rail），本视图只保留工作台主体——
 * 选中项目的头部（名称 + mono 分支 pill + `N sessions · ● active` meta）、
 * turn-stream 舞台与 composer。无聚焦会话时渲染预览原型的空态；有聚焦
 * 会话时给 spotlight 深链卡片并**就地内联**渲染 `<sebas-transcript-view>`
 * （聚焦会话的 detail 由 /api/sessions/:key 取得，随 summary 的
 * active_session_key 刷新——与 session-detail 同一套取数/装配方式）。
 * 统计卡条（Active/Dormant/Spawning/Uptime）与 "Recent sessions" 表已随
 * IA v2 移除。Live updates arrive over the shared WebSocket; a reconnect
 * triggers a refetch.
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type SessionDetail, type SessionRow, type Summary } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { icon } from '../components/icons.js'
import { navigate } from '../router.js'
import { viewStyles } from '../styles/shared.js'
import '../components/status-badge.js'
import './transcript-view.js'
import './workbench-composer.js'

@customElement('sebas-dashboard')
export class SebasDashboard extends LitElement {
  @state() private data: Summary | null = null
  @state() private allRows: SessionRow[] = []
  @state() private error = ''
  /**
   * Focused session's full detail (transcript entries + encoded key) for
   * the inline turn stream. Loaded from /api/sessions/:key whenever the
   * summary's `active_session_key` changes; `null` while loading or when
   * nothing is focused.
   */
  @state() private focusedDetail: SessionDetail | null = null
  /** Set when the focused detail fetch failed (session vanished mid-flight). */
  @state() private focusedUnavailable = false
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
   * `null` while loading; `"no provider configured"` if no providers
   * are registered.
   */
  @state() private providerLabel: string | null = null
  private unsubscribe?: () => void
  private onComposerCreated = (e: Event) => {
    const detail = (e as CustomEvent<{ key: string }>).detail
    navigate(`/sessions/${detail.key}`)
  }
  /**
   * add-composer-agent-binding：跟随模式下 composer 发出消息/切模型后乐观
   * 重取聚焦 detail——transcript 不等下一个 WS/summary 周期就能反映本轮。
   */
  private onComposerSent = (): void => {
    this.loadFocused(this.data?.active_session_key ?? null)
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

  protected willUpdate(changed: PropertyValues): void {
    // 侧栏选中项目切换 → 重取分支（面板 pill 用）。
    if (changed.has('selectedPath')) this.loadSelectedBranch()
  }

  private refetch = (): void => {
    api
      .summary()
      .then((d) => {
        this.data = d
        this.error = ''
        this.loadFocused(d.active_session_key)
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
        </div>
      `
    if (!this.data) return this.renderLoading()
    const d = this.data
    const rows = this.rowsForSelected()
    const hasActive = rows.some((r) => r.status_slug === 'working')
    const projectName = this.selectedPath
      ? (this.selectedPath.split('/').filter(Boolean).pop() ?? this.selectedPath)
      : null
    return html`
      <header class="project-header">
        ${projectName
          ? html`
              <span class="path" title=${this.selectedPath ?? ''}>${projectName}</span>
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
                  title="Open focused session"
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

      ${d.active_session
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
        <sebas-workbench-composer
          .projectDir=${this.selectedPath}
          .providerLabel=${this.providerLabel}
          .sessionKey=${d.active_session?.encoded_key ?? null}
          .agentKind=${d.active_session?.agent_kind ?? null}
          .sessionModels=${d.active_session?.available_models ?? []}
          .currentModel=${d.active_session?.current_model ?? null}
          @composer-created=${this.onComposerCreated}
          @composer-sent=${this.onComposerSent}
        ></sebas-workbench-composer>
      </div>
    `
  }

  private rowsForSelected(): SessionRow[] {
    if (this.selectedPath === null) return []
    return this.allRows.filter((r) => r.project_dir === this.selectedPath)
  }

  /**
   * Inline turn stream data: fetch the focused session's detail (same
   * endpoint session-detail uses). `null` key clears the stage; stale
   * responses (focus moved on while in flight) are dropped so the stream
   * never shows a session that is no longer focused.
   */
  private loadFocused(key: string | null): void {
    if (!key) {
      this.focusedDetail = null
      this.focusedUnavailable = false
      return
    }
    api
      .session(key)
      .then((d) => {
        if (this.data?.active_session_key === d.encoded_key) {
          this.focusedDetail = d
          this.focusedUnavailable = false
        }
      })
      .catch(() => {
        if (this.data?.active_session_key === key) {
          this.focusedDetail = null
          this.focusedUnavailable = true
        }
      })
  }

  /**
   * Inline turn-stream 舞台：聚焦会话就地的 transcript（复用
   * `<sebas-transcript-view fill>`，内部滚动/未读 seam 均归它管）。detail 尚在
   * 途时给骨架，取数失败（会话恰好被关闭）给一条温和空态而不是报错。
   * 容器是满幅面板区（flex 吃满余高），不再有 panel 卡片外观。
   */
  private renderTurnStream() {
    return html`
      <div class="turn-stream-area" aria-label="Focused session transcript">
        ${this.focusedDetail
          ? this.focusedDetail.body.length === 0
            ? html`
                <div class="empty">
                  <span class="glyph">${icon('message', 20)}</span>
                  <span class="title">Nothing yet</span>
                  <p class="hint">The agent has not produced output for this session.</p>
                </div>
              `
            : html`<sebas-transcript-view
                fill
                .entries=${this.focusedDetail.body}
                sessionKey=${this.focusedDetail.encoded_key}
              ></sebas-transcript-view>`
          : this.focusedUnavailable
            ? html`
                <div class="empty">
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

  /** 懒加载选中项目的分支（project-header 的 mono pill 用），选中即取，失败不渲染。 */
  private loadSelectedBranch(): void {
    const path = this.selectedPath
    this.selectedBranch = null
    if (path === null) return
    api.projects
      .branch(path)
      .then((info) => {
        // 选中项中途切换时丢弃过期响应，避免显示上一个项目的分支
        if (this.selectedPath === info.path) this.selectedBranch = info.branch
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
