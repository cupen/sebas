/**
 * Conversation view with a per-session "seen" boundary
 * (workbench-conversation-view 2.1–2.5, design D3/D4/D5).
 *
 * The payload is one ordered entry sequence (`kind` prompt|content,
 * `element_type` markdown|thinking|tool|error). The view groups it into
 * TURNS — the display unit — client-side, leaving the core's chunk-level
 * transcript alone:
 *
 *   - a `kind === 'prompt'` entry opens an operator turn ("你" bubble);
 *   - the entries after it (until the next prompt) form ONE agent turn =
 *     ONE assistant bubble, no matter how many streamed chunks arrived;
 *   - inside an agent turn, contiguous entry runs chunk into: concatenated
 *     text (markdown), a folded thinking block, and an expandable
 *     "used N tools" group — each run keeps its position, so a tool call
 *     between two statements splits the text instead of gluing it;
 *   - error entries (spawn failures) render as their own counted error
 *     bubbles, positioned in sequence.
 *
 * The seen-boundary seam counts TURNS, never entries: it sits above the
 * first turn with an entry newer than the stored seen-timestamp and never
 * splits a turn (D5). The stored boundary is still the per-browser
 * `created_at_unix` timestamp in localStorage — anchoring by timestamp, not
 * array index, keeps the seam pinned to the same logical turn even when an
 * older card refreshes in place.
 *
 * Scroll behaviour (unchanged):
 *   - while `sticky` is true, the view auto-scrolls to the seam (when
 *     there are unseen turns) or to the bottom (when everything is seen)
 *   - a near-bottom scroll (within 80px) marks turns as seen (250ms
 *     debounce, monotonic — only ever advances the boundary)
 *   - scrolling up past the seam disengages sticky; returning re-engages
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import type { ConversationEntryView } from '../api/client.js'
import { icon } from '../components/icons.js'
import { renderMarkdown } from '../components/markdown.js'

/** Bottom-scroll threshold for "mark-as-seen" detection. */
const NEAR_BOTTOM_PX = 80
/** Debounce window for mark-as-seen writes. */
const MARK_SEEN_DEBOUNCE_MS = 250

/**
 * Window (in unix seconds) within which consecutive identical error entries
 * (fail-fast-on-startup-errors: repeated spawn failures) merge into a single
 * counted bubble instead of flooding the conversation. "相邻 N 秒内的同类失败
 * 事件合并为一条带计数的错误" — adjacency is measured between consecutive
 * error entries, and only identical content merges.
 */
export const ERROR_MERGE_WINDOW_SECS = 10

/** A conversation entry possibly carrying a merge count (errors only). */
export type ErrorCountedView = ConversationEntryView & { count?: number }

/**
 * Merge runs of identical `element_type === 'error'` entries that arrive
 * within {@link ERROR_MERGE_WINDOW_SECS} of each other into one entry with a
 * `count`. The merged entry keeps the FIRST timestamp of its run — that is
 * the stable identity the seen-boundary seam anchors to. Non-error entries
 * and non-adjacent errors pass through untouched.
 */
export function mergeSpawnErrors(entries: ConversationEntryView[]): ErrorCountedView[] {
  const out: ErrorCountedView[] = []
  // State of the current adjacent run: last error ts + content, and the
  // merged entry the run is counting into.
  let lastErrTs = 0
  let lastErrContent: string | null = null
  let runEntry: ErrorCountedView | null = null
  for (const e of entries) {
    if (e.element_type !== 'error') {
      out.push({ ...e })
      lastErrTs = 0
      lastErrContent = null
      runEntry = null
      continue
    }
    const canMerge =
      runEntry !== null &&
      lastErrContent === e.content &&
      lastErrTs > 0 &&
      e.created_at_unix > 0 &&
      e.created_at_unix >= lastErrTs &&
      e.created_at_unix - lastErrTs <= ERROR_MERGE_WINDOW_SECS
    if (canMerge && runEntry) {
      runEntry.count = (runEntry.count ?? 1) + 1
      lastErrTs = e.created_at_unix
      continue
    }
    const copy: ErrorCountedView = { ...e, count: 1 }
    out.push(copy)
    runEntry = copy
    lastErrTs = e.created_at_unix
    lastErrContent = e.content
  }
  return out
}

// ---- turn grouping（design D3）-----------------------------------------

/** A run of contiguous markdown entries, concatenated in position order. */
export interface TextBlock {
  type: 'text'
  content: string
  /** Position of the run's first entry. */
  position: number
}

/** A run of contiguous thinking entries, folded into one block. */
export interface ThinkingBlock {
  type: 'thinking'
  content: string
  position: number
}

/** A run of contiguous tool entries, one expandable group. */
export interface ToolBlock {
  type: 'tools'
  items: { content: string; position: number }[]
  /** Position of the run's first entry. */
  position: number
}

export type AgentBlock = TextBlock | ThinkingBlock | ToolBlock

/** The operator's submission — its own turn. */
export interface OperatorUnit {
  kind: 'operator'
  entry: ConversationEntryView
}

/** One agent turn: everything the agent produced, chunked into blocks (D4). */
export interface AgentUnit {
  kind: 'agent'
  blocks: AgentBlock[]
  /** Position of the turn's first entry. */
  position: number
  /** Timestamp of the turn's first entry (bubble meta row). */
  startedAt: number
  /** Max entry timestamp within the turn (seam determination, D5). */
  maxTs: number
}

/** A merged error bubble (spawn failure etc.) — its own unit in sequence. */
export interface ErrorUnit {
  kind: 'error'
  entry: ErrorCountedView
}

export type TurnUnit = OperatorUnit | AgentUnit | ErrorUnit

/**
 * Group the ordered entry sequence into turn units (D3): each prompt opens
 * an operator turn; the entries until the next prompt belong to the
 * following agent turn; error entries render as standalone counted bubbles.
 * Empty-content entries are skipped (they carry nothing to display).
 */
export function groupConversation(entries: ErrorCountedView[]): TurnUnit[] {
  const units: TurnUnit[] = []
  for (const e of entries) {
    if (!e.content) continue
    if (e.element_type === 'error') {
      units.push({ kind: 'error', entry: e })
      continue
    }
    if (e.kind === 'prompt') {
      units.push({ kind: 'operator', entry: e })
      continue
    }
    // Agent-side content: join the current agent turn or open one.
    const last = units[units.length - 1]
    if (last?.kind === 'agent') {
      appendAgentBlock(last, e)
      last.maxTs = Math.max(last.maxTs, e.created_at_unix || 0)
    } else {
      const unit: AgentUnit = {
        kind: 'agent',
        blocks: [],
        position: e.position,
        startedAt: e.created_at_unix,
        maxTs: e.created_at_unix || 0,
      }
      appendAgentBlock(unit, e)
      units.push(unit)
    }
  }
  return units
}

/** The newest entry timestamp of a turn — the turn's seam edge (D5). */
export function unitMaxTs(unit: TurnUnit): number {
  if (unit.kind === 'operator' || unit.kind === 'error') return unit.entry.created_at_unix || 0
  return unit.maxTs
}

/**
 * Chunk one agent turn's entries into blocks (D4): contiguous markdown runs
 * concatenate into text; contiguous thinking entries fold together;
 * contiguous tool entries collect into one expandable group. A run break
 * (text→tool→text) splits the blocks so the tool group sits BETWEEN the two
 * text segments, in true sequence order.
 */
function appendAgentBlock(unit: AgentUnit, e: ConversationEntryView): void {
  const last = unit.blocks[unit.blocks.length - 1]
  if (last) {
    // Contiguous run of the same kind merges into the open block.
    if (last.type === 'text' && e.element_type !== 'thinking' && e.element_type !== 'tool') {
      last.content += e.content
      return
    }
    if (last.type === 'thinking' && e.element_type === 'thinking') {
      last.content += e.content
      return
    }
    if (last.type === 'tools' && e.element_type === 'tool') {
      last.items.push({ content: e.content, position: e.position })
      return
    }
  }
  if (e.element_type === 'thinking') {
    unit.blocks.push({ type: 'thinking', content: e.content, position: e.position })
  } else if (e.element_type === 'tool') {
    unit.blocks.push({
      type: 'tools',
      items: [{ content: e.content, position: e.position }],
      position: e.position,
    })
  } else {
    unit.blocks.push({ type: 'text', content: e.content, position: e.position })
  }
}

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
  /** The conversation entries from the session payload (`entries`). */
  @property({ attribute: false }) entries: ConversationEntryView[] = []
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

  /** Number of turns strictly below the seam. */
  @state() private unseenCount = 0
  /** Index of the first unseen turn; null when everything is seen. */
  @state() private seamIndex: number | null = 0
  /**
   * 渲染管线输入：错误合并 → 回合分组（seam/滚动/渲染都以此为准，
   * 分组后索引与回合一一对应）。
   */
  @state() private turnUnits: TurnUnit[] = []

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
    /* 未读边界 seam：细线分隔（两侧 1px 规则线 + 大写字距淡色标签）。
       计数单位是回合（D5）：数字 = 边界下方的气泡数，与视觉一致。 */
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
       26px 头像圆（assistant = accent 渐变底，user = accent-soft 底），
       气泡 14px 圆角并朝头像一侧收 4px 小角，最宽 min(680px, 100% - 60px)；
       时间戳在气泡 meta 行（作者名 weight 600 淡色 + 时间右对齐
       tabular-nums）。一个 agent 回合 = 一个气泡（2.1）。 */
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
    /* fail-fast-on-startup-errors 3.3：spawn-failed 错误气泡——failed 色
       系（callout-error 同源 token），! 头像 + 计数徽标。 */
    .turn-block .avatar.error {
      background: var(--sebas-status-failed-bg, #fee2e2);
      color: var(--sebas-status-failed, #b91c1c);
      border-color: transparent;
    }
    .turn-block .bubble.error {
      background: var(--sebas-status-failed-bg, #fee2e2);
      border-color: var(--sebas-status-failed-border, #fecaca);
    }
    .turn-block .meta .author.error,
    .turn-block .meta .count {
      color: var(--sebas-status-failed, #b91c1c);
    }
    .turn-block .meta .count {
      font-weight: 700;
      font-variant-numeric: tabular-nums;
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
    /* 回合内多段文本（工具组切开的两段论述）：段间留一行呼吸，视觉上
       明确「中间发生过事」（design Risks：分段规则可读）。 */
    .turn-block .body + .body,
    .turn-block .body + details,
    .turn-block details + .body,
    .turn-block details + details {
      margin-top: var(--sebas-space-2);
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
    /* thinking 折叠：details 整块收在回合气泡内，折叠行沿用 work-group
       出血条样式；summary 是原生 details/summary，可键盘展开（2.5）。 */
    .turn-block details.fold {
      margin: var(--sebas-space-3) -14px -9px;
      border-top: 1px solid var(--sebas-border);
      background: var(--sebas-surface-2);
    }
    .turn-block details.fold summary {
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
    .turn-block details.fold summary::-webkit-details-marker {
      display: none;
    }
    .turn-block details.fold summary .kind-icon {
      display: grid;
      place-items: center;
      width: 18px;
      height: 18px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .turn-block details.fold summary:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    .turn-block details.fold summary .fold-count {
      color: var(--sebas-accent);
      font-variant-numeric: tabular-nums;
    }
    /* 展开内容：work-block-body 同款（0.82rem/1.6 + 虚线顶边）。挂 .body
       复用 markdown 排版规则（后写的字号覆盖之）。 */
    .turn-block .fold-body {
      padding: 8px 14px 12px;
      font-size: 0.82rem;
      line-height: 1.6;
      border-top: 1px dashed var(--sebas-border);
    }
    /* 工具组内逐条工具之间以虚线分隔，与 thinking 折叠同一视觉语言。 */
    .turn-block .fold-body .tool-item + .tool-item {
      margin-top: var(--sebas-space-2);
      padding-top: var(--sebas-space-2);
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
    if (changed.has('entries') || changed.has('sessionKey')) {
      this.turnUnits = groupConversation(mergeSpawnErrors(this.entries))
      this.recomputeSeam()
    }
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
   * Recompute `seamIndex` and `unseenCount` from the current turns and the
   * stored seen-boundary（D5，按回合计数）. A turn is unseen when ANY of
   * its entries is newer than the stored value — so the boundary can never
   * fall inside a turn: it sits above the first turn whose newest entry
   * crossed it, and the count is turns-below, not entries. Turns without
   * a timestamp (legacy, value 0) cannot advance the seam: anchoring on
   * them would let an unknown-time turn push the seam onto a clearly-seen
   * neighbour.
   */
  private recomputeSeam(): void {
    const seen = this.readSeen()
    if (this.turnUnits.length === 0) {
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const maxTs = this.turnUnits.reduce((m, u) => Math.max(m, unitMaxTs(u)), 0)
    if (seen > 0 && seen >= maxTs) {
      // Everything is at or below the stored boundary.
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const idx = this.turnUnits.findIndex((u) => unitMaxTs(u) > seen)
    if (idx === -1) {
      this.seamIndex = null
      this.unseenCount = 0
    } else {
      this.seamIndex = idx
      this.unseenCount = this.turnUnits.length - idx
    }
  }

  // ---- mark-all-seen ----------------------------------------------------

  private markAllSeen = (): void => {
    const max = this.turnUnits.reduce((m, u) => Math.max(m, unitMaxTs(u)), 0)
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

  /** Push the seen-boundary forward to the newest rendered turn. */
  private commitMarkSeen(): void {
    const max = this.turnUnits.reduce((m, u) => Math.max(m, unitMaxTs(u)), 0)
    if (this.turnUnits.length > 0 && max > this.readSeen()) {
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
      // Already-seen case: stick to the newest turn.
      el.scrollTop = el.scrollHeight
    }
  }

  // ---- render -----------------------------------------------------------

  render() {
    const showSeam = this.unseenCount > 0
    // seam 仍是整行分隔条（文案 / localStorage 锚定 / 滚动锚点均不变），
    // 但内联落在最后一条已读与第一条未读**回合**之间（index =
    // seamIndex）；全部已读时保留行首的 hidden 占位，供滚动逻辑
    // querySelector('.seam') 命中。
    const seam = showSeam
      ? html`
          <div class="seam" data-count=${this.unseenCount} role="status">
            <span class="pill"
              ><span class="count">~${this.unseenCount} new</span> since you last viewed</span
            >
            <button type="button" class="link" @click=${this.markAllSeen}>
              mark all seen
            </button>
          </div>
        `
      : html`<div class="seam" hidden></div>`
    return html`
      <div class="scroll" role="log" aria-label="Session conversation">
        ${showSeam
          ? this.turnUnits.map((u, i) =>
              i === this.seamIndex ? html`${seam}${this.renderUnit(u)}` : this.renderUnit(u),
            )
          : html`${seam}${this.turnUnits.map((u) => this.renderUnit(u))}`}
      </div>
    `
  }

  private renderUnit(u: TurnUnit) {
    if (u.kind === 'error') return this.renderErrorUnit(u)
    if (u.kind === 'operator') return this.renderOperatorUnit(u)
    return this.renderAgentUnit(u)
  }

  /** fail-fast-on-startup-errors 3.3：错误条目仍是独立计数气泡。 */
  private renderErrorUnit(u: ErrorUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    const count = e.count ?? 1
    return html`
      <div class="turn-block is-error" data-error-count=${count}>
        <div class="avatar error">!</div>
        <div class="bubble error">
          <div class="meta">
            <span class="author error">spawn failed</span>
            ${count > 1 ? html`<span class="count">×${count}</span>` : nothing}
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">${unsafeHTML(renderMarkdown(e.content))}</div>
        </div>
      </div>
    `
  }

  /** 2.3：operator 回合 = 「你」气泡（accent-soft 底，右对齐）。 */
  private renderOperatorUnit(u: OperatorUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    return html`
      <div class="turn-block is-user">
        <div class="avatar user">你</div>
        <div class="bubble">
          <div class="meta">
            <span class="author you">you</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body"><p>${e.content}</p></div>
        </div>
      </div>
    `
  }

  /**
   * 2.1/2.2：一个 agent 回合 = 一个气泡。回合内按 D4 分块：文本段 /
   * thinking 折叠 / 工具组按位置交替落放。
   */
  private renderAgentUnit(u: AgentUnit) {
    const iso = isoTime(u.startedAt)
    const ts = formatTime(u.startedAt)
    return html`
      <div class="turn-block is-assistant" data-turn-position=${u.position}>
        <div class="avatar assistant">AI</div>
        <div class="bubble">
          <div class="meta">
            <span class="author">assistant</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          ${u.blocks.map((b) => this.renderAgentBlock(b))}
        </div>
      </div>
    `
  }

  private renderAgentBlock(b: AgentBlock) {
    if (b.type === 'text') {
      return html`<div class="body">${unsafeHTML(renderMarkdown(b.content))}</div>`
    }
    if (b.type === 'thinking') {
      return html`
        <details class="fold thinking-fold">
          <summary>
            <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
            <span class="label">thinking</span>
          </summary>
          <div class="body fold-body">${unsafeHTML(renderMarkdown(b.content))}</div>
        </details>
      `
    }
    // 工具组（2.2）：「used N tools」可展开组；原生 details/summary 支持
    // 键盘展开（2.5）。
    return html`
      <details class="fold tools-fold" data-tool-count=${b.items.length}>
        <summary>
          <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
          <span class="label">used ${b.items.length} tool${b.items.length === 1 ? '' : 's'}</span>
        </summary>
        <div class="body fold-body">
          ${b.items.map(
            (it) => html`<div class="tool-item">${unsafeHTML(renderMarkdown(it.content))}</div>`,
          )}
        </div>
      </details>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-transcript-view': SebasTranscriptView
  }
}
