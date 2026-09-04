/**
 * Streaming transcript view with a per-session "seen" boundary.
 *
 * Renders each entry in `entries` as an avatar + chat bubble, mirroring
 * the approved workbench preview: 26px avatar circles (assistant =
 * accent gradient; user prompts, `kind === 'prompt'`, = accent-soft on a
 * reversed right-aligned bubble), 14px-radius bubbles with a 4px notch
 * toward the avatar, and the timestamp moved from the old left-rail
 * column into each bubble's meta row. A thin "seam" strip highlights
 * anything appended since the reader last looked at the session; it
 * flows inline between the last seen and first unseen bubble. The seam
 * is anchored to the largest `created_at_unix` the client has actually
 * scrolled past (or clicked through). Anchoring by timestamp — not by
 * array index — keeps the seam pinned to the same logical entry even
 * when an older card refreshes in place (spec 4.4: the router stamps
 * `created_at_unix` at push time, never on refresh).
 *
 * Scroll behaviour:
 *   - while `sticky` is true, the view auto-scrolls to the seam (when
 *     there are unseen entries) or to the bottom (when everything is
 *     already seen)
 *   - a near-bottom scroll (within 80px) marks entries as seen (250ms
 *     debounce, monotonic — only ever advances the boundary)
 *   - if the reader scrolls up past the seam, sticky flips off so we
 *     don't fight them; scrolling back down to the seam re-engages it
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import type { CardElementView } from '../api/client.js'
import { icon } from '../components/icons.js'
import { renderMarkdown } from '../components/markdown.js'

/** Bottom-scroll threshold for "mark-as-seen" detection. */
const NEAR_BOTTOM_PX = 80
/** Debounce window for mark-as-seen writes. */
const MARK_SEEN_DEBOUNCE_MS = 250

/**
 * Render a unix-seconds timestamp into the format dictated by the spec:
 *   - today          → HH:MM:SS
 *   - this year      → MM-DD HH:MM
 *   - older          → YYYY-MM-DD
 * Returns an empty string for falsy timestamps (legacy entries).
 */
function formatTime(unixSecs: number): string {
  if (!unixSecs) return ''
  const d = new Date(unixSecs * 1000)
  const now = new Date()
  const sameYear = d.getFullYear() === now.getFullYear()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  const pad = (n: number) => String(n).padStart(2, '0')
  if (sameDay) {
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  }
  if (sameYear) {
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
  }
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** ISO 8601 string for `<time datetime=...>`. Empty string if no timestamp. */
function isoTime(unixSecs: number): string {
  if (!unixSecs) return ''
  return new Date(unixSecs * 1000).toISOString()
}

@customElement('sebas-transcript-view')
export class SebasTranscriptView extends LitElement {
  /** The transcript blocks from `SessionDetail.body`. */
  @property({ attribute: false }) entries: CardElementView[] = []
  /** Encoded session key; namespaces the seen-boundary in localStorage. */
  @property() sessionKey = ''
  /**
   * When true (default), auto-scroll on new entries. Flipped to false
   * internally when the reader scrolls up past the seam so we don't
   * fight deliberate scroll-up.
   */
  @property({ type: Boolean }) sticky = true

  /**
   * When true, host flexes to fill the workbench pane: the inner .scroll
   * lifts its 58vh cap and stretches (flex:1) so the pane owns the single
   * scroll region. Seam/seen-boundary logic is identical either way.
   */
  @property({ type: Boolean, reflect: true })
  fill = false

  /** Number of entries strictly after the seam. */
  @state() private unseenCount = 0
  /** Index of the first unseen entry; null when everything is seen. */
  @state() private seamIndex: number | null = 0

  /** Debounce timer for mark-as-seen writes. */
  private markSeenTimer: number | null = null
  /** Bound scroll handler so we can detach on disconnect. */
  private boundOnScroll = (): void => this.onScroll()
  /** Bound resize handler — scrollIntoView needs a layout flush. */
  private boundOnResize = (): void => {
    if (this.sticky) this.applyAutoScroll()
  }

  static styles = css`
    :host {
      display: block;
    }
    /* fill 模式：宿主随工作台面板拉伸，滚动容器交棒给面板框架
       （去掉 58vh 封顶，改 flex:1 吃满余高）。 */
    :host([fill]) {
      display: flex;
      flex-direction: column;
      flex: 1;
      min-height: 0;
      min-width: 0;
    }
    .scroll {
      max-height: 58vh;
      overflow-y: auto;
      padding: var(--sebas-space-4) var(--sebas-space-5);
      scroll-behavior: smooth;
      /* 预览稿 .turn-stream 同款纵向流布局：行间固定 gap，seam 作为
         整行分隔条自然落在两行气泡之间（不受气泡 max-width 约束）。 */
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3);
    }
    :host([fill]) .scroll {
      max-height: none;
      flex: 1;
      min-height: 0;
      padding: var(--sebas-space-5);
    }
    /* 未读边界 seam：预览稿同款细线分隔（两侧 1px 规则线 + 大写字距
       淡色标签），不再用整条着色横幅。DOM 文案与 class 钩子保持不变。 */
    .seam {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      margin: var(--sebas-space-2) 0;
      color: var(--sebas-text-faint);
      font-size: 0.72rem;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }
    .seam::before,
    .seam::after {
      content: '';
      flex: 1;
      height: 1px;
      background: var(--sebas-border);
    }
    .seam[hidden] {
      display: none;
    }
    .seam .pill {
      font-variant-numeric: tabular-nums;
      white-space: nowrap;
    }
    .seam .count {
      color: var(--sebas-accent);
      font-weight: 600;
    }
    .seam .link {
      color: var(--sebas-text-faint);
      text-decoration: none;
      cursor: pointer;
      background: none;
      border: none;
      padding: 0;
      font: inherit;
      transition: color var(--sebas-dur) var(--sebas-ease);
    }
    .seam .link:hover,
    .seam .link:focus-visible {
      color: var(--sebas-accent);
    }
    /* ── 对话气泡 ──
       预览稿 workbench.ts 同款：26px 头像圆（assistant = accent 渐变底，
       user = accent-soft 底），气泡 14px 圆角并朝头像一侧收 4px 小角，
       最宽 min(680px, 100% - 60px)；时间戳从旧版左侧时间轨移进气泡
       meta 行（作者名 weight 600 淡色 + 时间右对齐 tabular-nums）。 */
    .turn-block {
      display: flex;
      gap: 10px;
      align-items: flex-start;
      max-width: 100%;
    }
    .turn-block.is-user {
      flex-direction: row-reverse;
    }
    .turn-block .avatar {
      width: 26px;
      height: 26px;
      flex: 0 0 26px;
      border-radius: 50%;
      display: grid;
      place-items: center;
      font-size: 0.72rem;
      font-weight: 700;
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      color: var(--sebas-text-dim);
      margin-top: 2px;
    }
    .turn-block .avatar.assistant {
      background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
      color: var(--sebas-accent-ink);
      border-color: transparent;
    }
    .turn-block .avatar.user {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
      border-color: transparent;
    }
    .turn-block .bubble {
      flex: 1;
      min-width: 0;
      max-width: min(680px, calc(100% - 60px));
      padding: 9px 14px;
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: 14px;
      border-top-left-radius: 4px;
    }
    .turn-block.is-user .bubble {
      background: var(--sebas-accent-soft);
      border-color: var(--sebas-accent-border);
      color: var(--sebas-text);
      border-top-left-radius: 14px;
      border-top-right-radius: 4px;
    }
    .turn-block .meta {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-2);
      font-size: 0.7rem;
      color: var(--sebas-text-faint);
      margin-bottom: 4px;
    }
    .turn-block .meta .author {
      font-weight: 600;
      color: var(--sebas-text-dim);
    }
    .turn-block .meta .author.you {
      color: var(--sebas-accent);
    }
    .turn-block .meta .time {
      margin-left: auto;
      font-variant-numeric: tabular-nums;
      white-space: nowrap;
    }
    .turn-block .body {
      min-width: 0;
      font-size: 0.875rem;
      line-height: 1.65;
      color: var(--sebas-text);
    }
    .turn-block .body :is(p, pre, ul, ol, h1, h2, h3, h4) {
      overflow-wrap: break-word;
    }
    .turn-block .body :first-child {
      margin-top: 0;
    }
    .turn-block .body :last-child {
      margin-bottom: 0;
    }
    .turn-block .body h1,
    .turn-block .body h2,
    .turn-block .body h3 {
      color: var(--sebas-text-bright);
      letter-spacing: -0.01em;
    }
    .turn-block .body a {
      color: var(--sebas-accent);
      text-decoration: underline;
      text-underline-offset: 3px;
    }
    .turn-block .body pre {
      background: var(--sebas-well);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      padding: var(--sebas-space-3);
      overflow-x: auto;
      font-family: var(--sebas-font-mono);
      font-size: 0.82rem;
      line-height: 1.55;
    }
    .turn-block .body code {
      font-family: var(--sebas-font-mono);
      font-size: 0.88em;
    }
    .turn-block .body :not(pre) > code {
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-sm);
      padding: 1px 5px;
    }
    .turn-block .body blockquote {
      margin: 0.5em 0;
      padding: 0.1em 1em;
      border-left: 3px solid var(--sebas-border-strong);
      color: var(--sebas-text-dim);
    }
    /* thinking 折叠：details 整块搬进 assistant 气泡，折叠行沿用预览稿
       work-group 的出血条（surface-2 底 + 顶部 1px 分隔线，左右 -14px /
       底 -9px 抵消 bubble padding），18px accent-soft kind-icon 小徽章
       与文案保持不变。 */
    .turn-block details.thinking-fold {
      margin: var(--sebas-space-3) -14px -9px;
      border-top: 1px solid var(--sebas-border);
      background: var(--sebas-surface-2);
    }
    .turn-block details.thinking-fold summary {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 7px 14px;
      list-style: none;
      cursor: pointer;
      color: var(--sebas-text-dim);
      font-size: 0.78rem;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      user-select: none;
      transition: background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .turn-block details.thinking-fold summary::-webkit-details-marker {
      display: none;
    }
    .turn-block details.thinking-fold summary .kind-icon {
      display: grid;
      place-items: center;
      width: 18px;
      height: 18px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .turn-block details.thinking-fold summary:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    /* 展开内容：预览稿 work-block-body 同款（0.82rem/1.6 + 虚线顶边）。
       同时挂 .body 以复用上面的 markdown 排版规则（后写的字号覆盖之）。 */
    .turn-block .thinking-body {
      padding: 8px 14px 12px;
      font-size: 0.82rem;
      line-height: 1.6;
      border-top: 1px dashed var(--sebas-border);
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    this.recomputeSeam()
    window.addEventListener('resize', this.boundOnResize)
  }

  disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('resize', this.boundOnResize)
    if (this.markSeenTimer !== null) {
      clearTimeout(this.markSeenTimer)
      this.markSeenTimer = null
    }
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    if (changed.has('entries') || changed.has('sessionKey')) this.recomputeSeam()
  }

  protected updated(changed: Map<string, unknown>): void {
    super.updated(changed)
    // The scroll listener must be re-attached whenever the scroll
    // container is replaced in the DOM; this happens every render. We
    // diff against a private field so we don't pile up duplicate
    // listeners on the same node.
    const el = this.renderRoot.querySelector<HTMLElement>('.scroll')
    if (el && el !== this.scrollEl) {
      this.scrollEl?.removeEventListener('scroll', this.boundOnScroll)
      this.scrollEl = el
      el.addEventListener('scroll', this.boundOnScroll, { passive: true })
    }
    if (changed.has('entries') || changed.has('sessionKey') || changed.has('seamIndex')) {
      // Wait one frame for layout to settle, then scroll. Without the
      // rAF, scrollHeight can lag the freshly-inserted entries.
      requestAnimationFrame(() => this.applyAutoScroll())
    }
  }

  /** The current scroll container, if any. */
  private scrollEl: HTMLElement | null = null

  // ---- localStorage helpers --------------------------------------------

  private static storageKey(sessionKey: string): string {
    return `sebas:seen:${sessionKey}`
  }

  private readSeen(): number {
    try {
      const raw = localStorage.getItem(SebasTranscriptView.storageKey(this.sessionKey))
      if (raw === null) return 0
      const n = Number(raw)
      return Number.isFinite(n) ? n : 0
    } catch {
      return 0
    }
  }

  private writeSeen(value: number): void {
    try {
      localStorage.setItem(SebasTranscriptView.storageKey(this.sessionKey), String(value))
    } catch {
      /* storage may be disabled; degrade silently */
    }
  }

  // ---- seam logic -------------------------------------------------------

  /**
   * Recompute `seamIndex` and `unseenCount` from the current entries
   * and the stored seen-boundary. The seam is the first index whose
   * `created_at_unix` is strictly greater than the stored value; entries
   * without a timestamp (legacy, value 0) are ignored — they cannot
   * advance the seam, because anchoring on them would let an
   * unknown-time entry push the seam onto a clearly-seen neighbour.
   */
  private recomputeSeam(): void {
    const seen = this.readSeen()
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    if (ts.length === 0) {
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const maxTs = ts.reduce((a, b) => (b > a ? b : a), 0)
    if (seen > 0 && seen >= maxTs) {
      // Everything is at or below the stored boundary.
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const idx = this.entries.findIndex((e) => (e.created_at_unix || 0) > seen)
    if (idx === -1) {
      this.seamIndex = null
      this.unseenCount = 0
    } else {
      this.seamIndex = idx
      this.unseenCount = this.entries.length - idx
    }
  }

  // ---- mark-all-seen ----------------------------------------------------

  private markAllSeen = (): void => {
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    const max = ts.length === 0 ? 0 : ts.reduce((a, b) => (b > a ? b : a), 0)
    this.writeSeen(max)
    this.seamIndex = null
    this.unseenCount = 0
  }

  // ---- scroll handling --------------------------------------------------

  private onScroll(): void {
    const el = this.scrollEl
    if (!el) return
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
    const nearBottom = distanceFromBottom <= NEAR_BOTTOM_PX
    const seam = this.renderRoot.querySelector<HTMLElement>('.seam')
    const seamTop = seam ? seam.offsetTop : Number.POSITIVE_INFINITY
    // Distance the reader has scrolled relative to the seam. Positive
    // when they're at or below the seam; negative when above it.
    const relativeToSeam = el.scrollTop - seamTop
    if (nearBottom || relativeToSeam >= 0) {
      // The reader is at-or-past the seam — re-engage sticky if we'd
      // disengaged it on a deliberate scroll-up.
      if (!this.sticky) this.sticky = true
      this.scheduleMarkSeen()
    } else {
      // The reader has scrolled above the seam — let them read old
      // content without us yanking them back.
      if (this.sticky) this.sticky = false
    }
  }

  private scheduleMarkSeen(): void {
    if (this.markSeenTimer !== null) return
    this.markSeenTimer = window.setTimeout(() => {
      this.markSeenTimer = null
      this.commitMarkSeen()
    }, MARK_SEEN_DEBOUNCE_MS)
  }

  /** Push the seen-boundary forward to the newest rendered entry. */
  private commitMarkSeen(): void {
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    if (ts.length === 0) return
    const max = ts.reduce((a, b) => (b > a ? b : a), 0)
    if (max > this.readSeen()) {
      this.writeSeen(max)
      // The seam may have moved or disappeared; update internal state
      // and re-render without scheduling another auto-scroll — the user
      // is already where they want to be.
      const prev = this.seamIndex
      this.recomputeSeam()
      if (prev !== this.seamIndex) this.requestUpdate('seamIndex')
    }
  }

  /** Apply the spec-mandated scroll behaviour for the current frame. */
  private applyAutoScroll(): void {
    const el = this.scrollEl
    if (!el || !this.sticky) return
    if (this.seamIndex !== null && this.unseenCount > 0) {
      const seam = this.renderRoot.querySelector<HTMLElement>('.seam')
      if (seam && typeof seam.scrollIntoView === 'function') {
        seam.scrollIntoView({ block: 'center' })
      }
    } else {
      // Already-seen case: stick to the newest entry.
      el.scrollTop = el.scrollHeight
    }
  }

  // ---- render -----------------------------------------------------------

  render() {
    const showSeam = this.unseenCount > 0
    // seam 仍是整行分隔条（文案 / localStorage 锚定 / 滚动锚点均不变），
    // 但现在内联落在最后一条已读与第一条未读气泡之间（index =
    // seamIndex）；全部已读时保留行首的 hidden 占位，供滚动逻辑
    // querySelector('.seam') 命中 —— 行为与旧版一致。
    const seam = showSeam
      ? html`
          <div class="seam" data-count=${this.unseenCount} role="status">
            <span class="pill"><span class="count">~${this.unseenCount} new</span> since you last viewed</span>
            <button type="button" class="link" @click=${this.markAllSeen}>
              mark all seen
            </button>
          </div>
        `
      : html`<div class="seam" hidden></div>`
    return html`
      <div class="scroll" role="log" aria-label="Session transcript">
        ${showSeam
          ? this.entries.map((e, i) =>
              i === this.seamIndex ? html`${seam}${this.renderEntry(e)}` : this.renderEntry(e),
            )
          : html`${seam}${this.entries.map((e) => this.renderEntry(e))}`}
      </div>
    `
  }

  private renderEntry(e: CardElementView) {
    if (!e.content) return nothing
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    if (e.element_type === 'thinking') {
      // thinking 仍走 details 折叠，但整块搬进 assistant 气泡：meta 行
      // 照常（作者 + 时间），折叠行用预览稿 work-group 出血条样式。
      return html`
        <div class="turn-block is-assistant">
          <div class="avatar assistant">AI</div>
          <div class="bubble">
            <div class="meta">
              <span class="author">assistant</span>
              <time class="time" datetime=${iso || nothing}>${ts}</time>
            </div>
            <details class="thinking-fold">
              <summary>
                <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
                <span class="label">thinking</span>
              </summary>
              <div class="body thinking-body">${unsafeHTML(renderMarkdown(e.content))}</div>
            </details>
          </div>
        </div>
      `
    }
    // kind === 'prompt' 的用户输入走右侧 user 气泡（accent-soft 底 +
    // 「你」头像）；其余（markdown 及未知遗留类型）一律按 agent 输出
    // 渲染为 assistant 气泡。未知 element_type 仍默认走 markdown，
    // 让内容不丢（legacy `collapsible`/`div` 形态照旧兜底）。
    const isUser = e.element_type === 'prompt'
    return html`
      <div class="turn-block ${isUser ? 'is-user' : 'is-assistant'}">
        <div class="avatar ${isUser ? 'user' : 'assistant'}">${isUser ? '你' : 'AI'}</div>
        <div class="bubble">
          <div class="meta">
            <span class="author ${isUser ? 'you' : ''}">${isUser ? 'you' : 'assistant'}</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">
            ${isUser ? html`<p>${e.content}</p>` : unsafeHTML(renderMarkdown(e.content))}
          </div>
        </div>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-transcript-view': SebasTranscriptView
  }
}
