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
 *   - inside an agent turn, the text (markdown) segments stay outside in
 *     stream order while ALL thinking + tool entries collect into ONE
 *     process fold positioned at the turn's first process entry
 *     (workbench-agent-identity-and-process-folds 2.1/2.2) — expanding it
 *     reveals second-level per-entry folds (each thinking segment, each
 *     tool invocation), also collapsed by default, titled by the entry's
 *     structured `title` (generic label fallback, middle-truncated);
 *   - error entries (spawn failures) render as their own counted error
 *     bubbles, positioned in sequence;
 *   - the operator's newest submission shows a low-key "已收到" receipt
 *     badge while the session is Working and no agent output has arrived
 *     (3.2, derived state — no sender-side state machine);
 *   - the assistant author label resolves via `agentDisplay`
 *     (display → slug → assistant, D1) with a first-grapheme text avatar.
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
import { repeat } from 'lit/directives/repeat.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import type { ConversationEntryView } from '../api/client.js'
import { icon } from '../components/icons.js'
import { renderMarkdown } from '../components/markdown.js'
import { readAnchor, writeSeen as writeCursor } from './unread-cursor.js'

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

/** One thinking/tool entry inside the turn's single process block. */
export interface ProcessItem {
  /** `thinking` | `tool` — the source entry's element_type. */
  elementType: string
  content: string
  /**
   * Structured title built by the backend (tool name + key argument).
   * Absent (legacy entries) → the fold falls back to a generic label.
   */
  title?: string | null
  /** The source entry's transcript position — the DOM identity key. */
  position: number
}

/**
 * The turn's ONE process block (workbench-agent-identity-and-process-folds
 * 2.1): every thinking + tool entry of the agent turn collects here,
 * regardless of how many runs the old per-run chunker would have produced.
 */
export interface ProcessBlock {
  type: 'process'
  items: ProcessItem[]
  /** Position of the turn's FIRST process entry — where the fold sits. */
  position: number
}

export type AgentBlock = TextBlock | ProcessBlock

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
  // 当前 agent 回合内紧邻的前一条 entry（null = 回合开头）：文本段只与
  // 紧邻的文本条目相连——被过程条目隔开的文本是不同的段（2.1）。
  let prevInTurn: ConversationEntryView | null = null
  for (const e of entries) {
    if (!e.content) continue
    if (e.element_type === 'error') {
      units.push({ kind: 'error', entry: e })
      prevInTurn = null
      continue
    }
    if (e.kind === 'prompt') {
      units.push({ kind: 'operator', entry: e })
      prevInTurn = null
      continue
    }
    // Agent-side content: join the current agent turn or open one.
    const last = units[units.length - 1]
    if (last?.kind === 'agent') {
      appendAgentBlock(last, e, prevInTurn)
      last.maxTs = Math.max(last.maxTs, e.created_at_unix || 0)
    } else {
      const unit: AgentUnit = {
        kind: 'agent',
        blocks: [],
        position: e.position,
        startedAt: e.created_at_unix,
        maxTs: e.created_at_unix || 0,
      }
      appendAgentBlock(unit, e, null)
      units.push(unit)
    }
    prevInTurn = e
  }
  return units
}

/** The newest entry timestamp of a turn — the turn's seam edge (D5). */
export function unitMaxTs(unit: TurnUnit): number {
  if (unit.kind === 'operator' || unit.kind === 'error') return unit.entry.created_at_unix || 0
  return unit.maxTs
}

// ---- fold title helpers（2.3，D5）---------------------------------------

/** Titles longer than this many characters are middle-truncated. */
export const TITLE_MAX_CHARS = 64
/** Head characters preserved by the middle truncation. */
export const TITLE_HEAD_CHARS = 28
/** Tail characters preserved by the middle truncation. */
export const TITLE_TAIL_CHARS = 28

/**
 * Split a string into grapheme clusters — Intl.Segmenter where available
 * (keeps ZWJ emoji, flags and combining sequences whole), code points as
 * the fallback. 多字节字符绝不切半个。
 */
function graphemes(s: string): string[] {
  const Seg = (Intl as { Segmenter?: new (...a: unknown[]) => { segment(input: string): Iterable<{ segment: string }> } })
    .Segmenter
  if (typeof Seg === 'function') {
    return Array.from(new Seg(undefined, { granularity: 'grapheme' }).segment(s), (seg) => seg.segment)
  }
  return Array.from(s)
}

/**
 * Middle-truncate a fold title（2.3，D5）: over {@link TITLE_MAX_CHARS}
 * characters collapses the middle into a single `…`, keeping the head and
 * tail. Callers carry the full string on the `title` attribute for hover.
 */
export function middleTruncate(
  text: string,
  max: number = TITLE_MAX_CHARS,
  head: number = TITLE_HEAD_CHARS,
  tail: number = TITLE_TAIL_CHARS,
): string {
  const g = graphemes(text)
  if (g.length <= max) return text
  return g.slice(0, head).join('') + '…' + g.slice(-tail).join('')
}

/**
 * Second-level fold label fallback（2.2）: entries without the structured
 * `title` (legacy persisted data) show a generic stable label derived from
 * the element type instead.
 */
export function processItemLabel(item: ProcessItem): { label: string; full: string | null } {
  const title = item.title?.trim() ? item.title : null
  if (title) return { label: middleTruncate(title), full: title }
  return { label: item.elementType, full: null }
}

// ---- agent identity + receipt（3.1/3.2）---------------------------------

/**
 * Assistant 作者标签回退链（3.1，D1）: catalog display → slug → generic
 * `assistant`. The first two levels are resolved by the dashboard (which
 * holds the `/api/agents` catalog) before the value arrives as
 * {@link SebasTranscriptView.agentDisplay}; this covers the last step plus
 * blank/whitespace tolerance.
 */
export function resolveAgentDisplay(agentDisplay: string | null | undefined): string {
  const v = typeof agentDisplay === 'string' ? agentDisplay.trim() : ''
  return v || 'assistant'
}

/**
 * 已收到角标的派生判定（3.2，D4）: the entry sequence ends on the
 * operator's submission — i.e. the newest rendered unit is still the prompt
 * with no agent output yet. Pure entry-sequence semantics（spec「Operator
 * submission receipt」）: the live flow shows queued（非 working）while the
 * prompt is the last entry and flips working the moment the first agent
 * entry lands, so a Working gate made the badge unreachable in practice.
 * Purely derived: once any agent entry arrives the last unit stops being
 * the prompt and the badge disappears without any sender-side state machine.
 */
export function awaitingReceipt(units: TurnUnit[]): boolean {
  return units.length > 0 && units[units.length - 1].kind === 'operator'
}

/**
 * Chunk one agent turn's entries into blocks（2.1，D3）: contiguous markdown
 * runs concatenate into text; EVERY thinking/tool entry collects into the
 * turn's single process block, which sits at the position of the first
 * process entry. Text segments stay outside the fold, in stream order — a
 * run break (text→process→text) splits the text so the segments keep their
 * arrival sequence around the fold. `prev` is the immediately preceding
 * entry of the same turn (null = turn start) and gates text concatenation.
 */
function appendAgentBlock(
  unit: AgentUnit,
  e: ConversationEntryView,
  prev: ConversationEntryView | null,
): void {
  const last = unit.blocks[unit.blocks.length - 1]
  if (e.element_type === 'thinking' || e.element_type === 'tool') {
    // 过程条目：全部汇入本回合唯一的过程块（2.1）——不再按 run 平铺；
    // 块定位在首个过程条目的位置，后面的条目无论隔着多少文本段都并入它。
    const existing = unit.blocks.find((b): b is ProcessBlock => b.type === 'process')
    if (existing) {
      existing.items.push({
        elementType: e.element_type,
        content: e.content,
        title: e.title ?? null,
        position: e.position,
      })
      return
    }
    unit.blocks.push({
      type: 'process',
      items: [
        { elementType: e.element_type, content: e.content, title: e.title ?? null, position: e.position },
      ],
      position: e.position,
    })
    return
  }
  if (last?.type === 'text' && prev !== null && !isProcessEntry(prev)) {
    last.content += e.content
    return
  }
  unit.blocks.push({ type: 'text', content: e.content, position: e.position })
}

/** 过程条目（2.1）：进入过程大折叠的 element_type 词表。 */
function isProcessEntry(e: ConversationEntryView): boolean {
  return e.element_type === 'thinking' || e.element_type === 'tool'
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
   * rail-declutter-unread D3：会话当前的服务端可见回复段数（detail/summary
   * payload 的 `msg_count`）。标记已读时以它推进共享游标的 `anchor_count`，
   * 让 rail 徽标与 seam 同步清零；`null`（旧 payload / 测试）= 只推进
   * seen 时间戳、不动段数锚。
   */
  @property({ attribute: false }) msgCount: number | null = null
  /**
   * workbench-agent-identity 3.1（D1）：会话绑定 agent 的展示名。dashboard
   * 按 `agent_kind` 匹配 `/api/agents` 目录后传入（目录无 display 条目时传
   * raw slug）；`null` = 目录不可得 / 未绑定 → 组件回退通用 `assistant`。
   * 回退链：display → slug → assistant。
   */
  @property({ attribute: false }) agentDisplay: string | null = null
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
    /* workbench-agent-identity 3.2：已收到角标——操作者提交已被服务端接受、
       agent 输出尚未开始的在途提示。低调 pill（faint 色 + working 色小点），
       agent 回合开始即随派生条件消失。 */
    .turn-block .meta .receipt {
      display: inline-flex;
      align-items: center;
      gap: 5px;
      font-size: 0.66rem;
      line-height: 1;
      color: var(--sebas-text-faint);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-full);
      padding: 2px 8px;
      white-space: nowrap;
    }
    .turn-block .meta .receipt::before {
      content: '';
      width: 5px;
      height: 5px;
      border-radius: 50%;
      background: var(--sebas-status-working, var(--sebas-accent));
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
    /* 过程大折叠（2.1/2.2）：回合内全部 thinking + tool 收进一个 details，
       折叠行沿用 work-group 出血条样式；summary 是原生 details/summary，
       可键盘展开（2.5）。展开后为二级逐条折叠（.process-item）。 */
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
    /* 过程大折叠的二级折叠（2.2）：thinking 段 / 工具条目各一折，默认收起。
       summary 是条目标题（title 或通用标签）——mono 小字、不改大小写
       （路径参数被 uppercase 会变形）。相邻条目以虚线分隔，沿用同一视觉
       语言。 */
    .turn-block details.process-item + details.process-item {
      margin-top: var(--sebas-space-2);
      padding-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
    }
    .turn-block details.process-item summary {
      display: flex;
      align-items: center;
      gap: 6px;
      list-style: none;
      cursor: pointer;
      user-select: none;
      font-family: var(--sebas-font-mono);
      font-size: 0.76rem;
      color: var(--sebas-text-dim);
      transition: color var(--sebas-dur) var(--sebas-ease);
    }
    .turn-block details.process-item summary::-webkit-details-marker {
      display: none;
    }
    .turn-block details.process-item summary:hover,
    .turn-block details.process-item summary:focus-visible {
      color: var(--sebas-text-bright);
    }
    .turn-block details.process-item summary .item-title {
      min-width: 0;
      overflow-wrap: anywhere;
    }
    .turn-block details.process-item .item-body {
      padding-top: var(--sebas-space-2);
      font-size: 0.8rem;
      line-height: 1.6;
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

  // rail-declutter-unread 2.2：seen 存储迁移到共享游标模块 unread-cursor
  // （与 rail 徽标同一锚，`sebas:seen:<key>` 键与旧实现相同，旧数据原地
  // 迁移）。seam 的读写语义不变：无锚读 0，写入取单调 max。

  private readSeen(): number {
    return readAnchor(this.sessionKey)?.seenTs ?? 0
  }

  private writeSeen(value: number): void {
    // 段数锚只在 payload 带来 msg_count 时推进（单调 max 由游标模块保证）。
    writeCursor(this.sessionKey, value, this.msgCount ?? undefined)
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
    // 3.2（D4）：角标是派生态——纯 entry 序语义：最后一条渲染单元仍是
    // 操作者提交即显示（排队窗非 working 也成立）。agent entry 一到（收尾
    // 不再是 prompt）条件即不成立，角标消失。
    const receipt = awaitingReceipt(this.turnUnits)
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
              i === this.seamIndex
                ? html`${seam}${this.renderUnit(u, receipt && i === this.turnUnits.length - 1)}`
                : this.renderUnit(u, receipt && i === this.turnUnits.length - 1),
            )
          : html`${seam}${this.turnUnits.map((u, i) =>
              this.renderUnit(u, receipt && i === this.turnUnits.length - 1),
            )}`}
      </div>
    `
  }

  private renderUnit(u: TurnUnit, receipt: boolean) {
    if (u.kind === 'error') return this.renderErrorUnit(u)
    if (u.kind === 'operator') return this.renderOperatorUnit(u, receipt)
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
  private renderOperatorUnit(u: OperatorUnit, receipt: boolean) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    return html`
      <div class="turn-block is-user">
        <div class="avatar user">你</div>
        <div class="bubble">
          <div class="meta">
            <span class="author you">you</span>
            ${receipt
              ? html`<span class="receipt" data-receipt title="服务端已接受，等待 agent 开始输出"
                  >已收到</span
                >`
              : nothing}
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body"><p>${e.content}</p></div>
        </div>
      </div>
    `
  }

  /**
   * 2.1/2.2：一个 agent 回合 = 一个气泡。回合内全部过程条目收进单个过程
   * 大折叠，文本段按流序留在折叠外。作者标签走 display → slug → assistant
   * 回退链（3.1，D1），头像维持文本形态（展示名首字母；无展示名保持既有
   * AI 形态）。
   */
  private renderAgentUnit(u: AgentUnit) {
    const iso = isoTime(u.startedAt)
    const ts = formatTime(u.startedAt)
    const label = resolveAgentDisplay(this.agentDisplay)
    const avatar = label === 'assistant' ? 'AI' : graphemes(label)[0]?.toUpperCase() ?? 'AI'
    return html`
      <div class="turn-block is-assistant" data-turn-position=${u.position}>
        <div class="avatar assistant">${avatar}</div>
        <div class="bubble">
          <div class="meta">
            <span class="author">${label}</span>
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
    // 过程大折叠（2.1/2.2）：回合内全部 thinking + tool 的唯一折叠，默认
    // 收起；原生 details/summary 支持键盘展开（2.5）。展开后逐条二级折叠
    // 也默认收起，条目 DOM 以 position 为键保持身份（避免重渲染丢展开态）。
    return html`
      <details class="fold process-fold" data-process-count=${b.items.length}>
        <summary>
          <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
          <span class="label">process · ${b.items.length}</span>
        </summary>
        <div class="body fold-body">
          ${repeat(
            b.items,
            (it) => it.position,
            (it) => this.renderProcessItem(it),
          )}
        </div>
      </details>
    `
  }

  /** 二级折叠（2.2）：title 概要 + `title` 属性保全量；无 title 回退通用标签。 */
  private renderProcessItem(it: ProcessItem) {
    const { label, full } = processItemLabel(it)
    return html`
      <details class="process-item" data-position=${it.position} data-element-type=${it.elementType}>
        <summary title=${full ?? nothing}>
          <span class="item-title">${label}</span>
        </summary>
        <div class="body item-body">${unsafeHTML(renderMarkdown(it.content))}</div>
      </details>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-transcript-view': SebasTranscriptView
  }
}
