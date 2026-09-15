/**
 * Conversation view with a per-session "seen" boundary
 * (workbench-natural-conversation-flow; former workbench-conversation-view
 * 2.1–2.5 and workbench-agent-identity-and-process-folds 2.1–3.2).
 *
 * The payload is one ordered entry sequence (`kind` prompt|content,
 * `element_type` markdown|thinking|tool|error). The view groups it into
 * TURNS — the display unit — client-side, leaving the core's chunk-level
 * transcript alone:
 *
 *   - a `kind === 'prompt'` entry opens an operator turn, rendered as a
 *     lightly tinted block without card chrome (no border or shadow);
 *   - the entries after it (until the next prompt) form ONE agent turn,
 *     rendered as a natural conversation flow — no bubble/card wrapper;
 *     the author label row (agentDisplay, first-grapheme avatar) sits bare
 *     above the turn's content;
 *   - within an agent turn, entries split in ARRIVAL ORDER into alternating
 *     runs (splitAgentRuns, D1): contiguous markdown entries concatenate
 *     into a text run, contiguous thinking/tool entries form a process run
 *     rendered as ONE collapsed-by-default fold at the run's actual position
 *     between the text segments. Each fold keeps second-level per-entry
 *     folds (structured titles, generic fallback, middle-truncated); its
 *     summary row shows the running entry's title + the entry count and
 *     updates live as streaming refetches land (D3). Fold open state is
 *     tracked per run id (the run's first entry position) in a local Map so
 *     an expanded fold survives full regroupings and streamed entries append
 *     in place (D2);
 *   - error entries (spawn failures) do not join runs — they render as
 *     their own counted error bubbles, positioned in sequence (D5);
 *   - the operator's newest submission shows a low-key "已收到" receipt
 *     badge while no agent output has arrived (pure entry-sequence derived
 *     state — no sender-side state machine);
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
import { sharedWs } from '../api/shared-ws.js'

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

// ---- turn grouping（workbench-natural-conversation-flow D1/D5）---------

/** A run of contiguous markdown entries, concatenated in position order. */
export interface TextRun {
  type: 'text'
  content: string
  /** Position of the run's first entry. */
  position: number
}

/** One thinking/tool entry inside a process run. */
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
 * One run of contiguous thinking/tool entries (D1) — rendered as ONE
 * collapsed-by-default process fold at the run's actual position between
 * the turn's text segments. `position` (the run's first entry) is the
 * run's stable identity: streaming appends items but never moves the first
 * entry, so fold ids — and with them the open-state Map and DOM keys —
 * survive every full regrouping (D2).
 */
export interface ProcessRun {
  type: 'process'
  items: ProcessItem[]
  /** Position of the run's FIRST entry — the run/fold id (D2). */
  position: number
}

export type AgentRun = TextRun | ProcessRun

/** The operator's submission — its own turn. */
export interface OperatorUnit {
  kind: 'operator'
  entry: ConversationEntryView
}

/** One agent turn: everything the agent produced, as alternating runs (D1). */
export interface AgentUnit {
  kind: 'agent'
  runs: AgentRun[]
  /** Position of the turn's first entry. */
  position: number
  /** Timestamp of the turn's first entry (author label row). */
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

/** 过程条目（D1）：进入过程 run 的 element_type 词表。 */
function isProcessEntry(e: ConversationEntryView): boolean {
  return e.element_type === 'thinking' || e.element_type === 'tool'
}

/**
 * Split ONE agent turn's entry sequence into alternating runs（2.1，D1 —
 * 纯函数）: contiguous markdown entries concatenate into a text run,
 * contiguous thinking/tool entries accumulate into a process run. A kind
 * change opens the next run — that is what keeps each process fold at the
 * position where its entries actually happened, between the text segments.
 * Error entries never reach this function: `groupConversation` routes them
 * into standalone counted bubbles upstream (D5). Callers pre-filter empty
 * entries (they carry nothing to display and do not break a run).
 */
export function splitAgentRuns(entries: ConversationEntryView[]): AgentRun[] {
  const runs: AgentRun[] = []
  for (const e of entries) {
    const item: ProcessItem = {
      elementType: e.element_type,
      content: e.content,
      title: e.title ?? null,
      position: e.position,
    }
    const last = runs[runs.length - 1]
    if (isProcessEntry(e)) {
      if (last?.type === 'process') {
        last.items.push(item)
        continue
      }
      runs.push({ type: 'process', items: [item], position: e.position })
      continue
    }
    if (last?.type === 'text') {
      last.content += e.content
      continue
    }
    runs.push({ type: 'text', content: e.content, position: e.position })
  }
  return runs
}

/**
 * Group the ordered entry sequence into turn units (D3): each prompt opens
 * an operator turn; the entries until the next prompt belong to the
 * following agent turn; error entries render as standalone counted bubbles
 * and split the surrounding agent turns (D5 — they never join a run).
 * Empty-content entries are skipped (they carry nothing to display).
 */
export function groupConversation(entries: ErrorCountedView[]): TurnUnit[] {
  const units: TurnUnit[] = []
  // 当前 agent 回合积攒的条目：error/prompt 都会终结它——前者独立成错误
  // 气泡（D5），后者是操作者回合。收尾时一次性切 run（D1）。
  let pending: ConversationEntryView[] = []
  const flush = (): void => {
    if (pending.length === 0) return
    const first = pending[0]
    units.push({
      kind: 'agent',
      runs: splitAgentRuns(pending),
      position: first.position,
      startedAt: first.created_at_unix,
      maxTs: pending.reduce((m, e) => Math.max(m, e.created_at_unix || 0), 0),
    })
    pending = []
  }
  for (const e of entries) {
    if (!e.content) continue
    if (e.element_type === 'error') {
      flush()
      units.push({ kind: 'error', entry: e })
      continue
    }
    if (e.kind === 'prompt') {
      flush()
      units.push({ kind: 'operator', entry: e })
      continue
    }
    pending.push(e)
  }
  flush()
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

/**
 * 过程折叠的 summary 行标签（D3）: the run's LAST entry is the one currently
 * streaming, so its structured title (generic label fallback) IS the
 * "running tool" — as refetched entries land, the run's tail moves and the
 * summary tracks it live, no extra event needed. Callers pair the label
 * with the entry count.
 */
export function processRunSummary(run: ProcessRun): { label: string; full: string | null } {
  const last = run.items[run.items.length - 1]
  if (!last) return { label: 'process', full: null }
  return processItemLabel(last)
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

  /**
   * （D2）过程折叠的展开状态：以 run id（run 首条目 position）为键记在本
   * 视图。流式 refetch 每次全量重分组，重渲染后按 id 恢复——已展开的折叠
   * 不收起；Map 未命中即默认收起，过程折叠在流式期间永不自动展开（D3）。
   * id 只在会话内有意义，换会话清空。
   */
  private foldOpen = new Map<string, boolean>()

  /**
   * （workbench-live-conversation-flow 2.2）流式增量的就地缓冲：turn.append
   * 帧的条目先落这里（position 去重），与 `entries` 属性合并进渲染管线。
   * 快照重取（entries 属性）是收敛基准——重取带来的条目 position 覆盖缓冲
   * 后，缓冲里被覆盖的部分即被裁掉。
   */
  private streamEntries: ConversationEntryView[] = []
  /**
   * （6.1）流式期间已标记已读的可见回复段数（markdown/error）：快照的
   * `msg_count` 尚未赶上流式进度时，把这段增量补进段数锚，rail 徽标与
   * seam 才不闪现。msg_count 属性每次到达（快照收敛）即清零。
   */
  private streamMsgBonus = 0
  /** turn.append 订阅的退订句柄（connectedCallback 挂，disconnected 摘）。 */
  private unsubscribeTurn: (() => void) | null = null
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
    /* ── 自然对话流 ──
       26px 头像圆保留（assistant = accent 渐变底，user = accent-soft 底）。
       内容侧去卡片（D4）：agent 回合不再有气泡壳——.flow 裸排（作者标签
       小字行 + 正文段/过程折叠按 run 序直接铺）；用户侧 .msg-block 收敛为
       轻底色块（tinted 背景、无边框阴影）。最宽 min(680px, 100% - 60px)；
       时间戳在 meta 行（作者名 weight 600 淡色 + 时间右对齐
       tabular-nums）。错误气泡保留计数卡片形态（D5）。 */
    .turn-block {
      display: flex;
      gap: 10px;
      align-items: flex-start;
      max-width: 100%;
    }
    .turn-block.is-user {
      flex-direction: row-reverse;
    }
    .turn-block .flow,
    .turn-block .msg-block {
      flex: 1;
      min-width: 0;
      max-width: min(680px, calc(100% - 60px));
    }
    .turn-block .msg-block {
      padding: 9px 14px;
      background: var(--sebas-accent-soft);
      color: var(--sebas-text);
      border-radius: 12px;
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
    /* 过程折叠（一 run 一折，2.1/D1）：内容侧没有气泡壳后折叠自成一格——
       surface-2 浅底 + 细边圆角（贴地分组条，非卡片：无阴影）。summary 是
       原生 details/summary，可键盘展开（2.5）；行内实时渲染「进行中条目
       title + 条目计数」（D3）。展开后为二级逐条折叠（.process-item）。 */
    .turn-block details.fold {
      margin: var(--sebas-space-2) 0;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      background: var(--sebas-surface-2);
    }
    .turn-block details.fold summary {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 7px 12px;
      list-style: none;
      cursor: pointer;
      color: var(--sebas-text-dim);
      font-size: 0.78rem;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      user-select: none;
      border-radius: var(--sebas-radius-md);
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
    /* 进行中条目的 title（结构化 title 或通用标签）：mono 小字、不吃
       summary 的大小写（路径参数被 uppercase 会变形），超长省略号收干。 */
    .turn-block details.fold summary .running {
      flex: 1;
      min-width: 0;
      text-align: left;
      font-family: var(--sebas-font-mono);
      font-size: 0.76rem;
      text-transform: none;
      letter-spacing: 0;
      color: var(--sebas-text-faint);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .turn-block details.fold summary .fold-count {
      color: var(--sebas-accent);
      font-variant-numeric: tabular-nums;
    }
    /* 展开内容：work-block-body 同款（0.82rem/1.6 + 虚线顶边）。挂 .body
       复用 markdown 排版规则（后写的字号覆盖之）。 */
    .turn-block .fold-body {
      padding: 8px 12px 12px;
      font-size: 0.82rem;
      line-height: 1.6;
      border-top: 1px dashed var(--sebas-border);
    }
    /* 过程折叠的二级折叠（2.2）：thinking 段 / 工具条目各一折，默认收起。
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
    // 流式订阅（workbench-live-conversation-flow 2.2）：按聚焦会话过滤，
    // position 去重后并入渲染管线。
    this.unsubscribeTurn = sharedWs.subscribe((event) => {
      if (event.type !== 'turn.append') return
      this.onTurnAppend(event.session_id, event.entries)
    })
  }

  disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('resize', this.boundOnResize)
    this.unsubscribeTurn?.()
    this.unsubscribeTurn = null
    if (this.markSeenTimer !== null) {
      clearTimeout(this.markSeenTimer)
      this.markSeenTimer = null
    }
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    if (changed.has('sessionKey')) {
      // 换会话：上一个会话的流式残留全部作废（快照是新会话的真源），
      // 折叠展开状态同样作废（run id 只在会话内有意义，D2）。
      this.streamEntries = []
      this.streamMsgBonus = 0
      this.foldOpen.clear()
    }
    if (changed.has('msgCount')) {
      // 快照的段数到了（含流式期间产生的段）：增量补丁清零。
      this.streamMsgBonus = 0
    }
    if (changed.has('entries') || changed.has('sessionKey')) {
      // 快照收敛：position 已被属性覆盖的流式条目裁掉，只留快照还没
      // 追上的尾巴（乱序竞态下绝不回退视图）。
      const snapMax = this.entries.reduce((m, e) => Math.max(m, e.position), 0)
      this.streamEntries = this.streamEntries.filter((e) => e.position > snapMax)
      this.rebuildUnits()
      this.recomputeSeam()
    }
  }

  /** 渲染管线输入：快照条目 + 流式尾巴 → 错误合并 → 回合分组。 */
  private rebuildUnits(): void {
    const merged = [...this.entries, ...this.streamEntries]
    this.turnUnits = groupConversation(mergeSpawnErrors(merged))
  }

  /**
   * turn.append 到达（workbench-live-conversation-flow 2.2 / 6.1）：position
   * 去重后并入渲染管线。聚焦且贴底（sticky）时随渲染推进读锚——角标不闪、
   * 已读缝不出现；未贴底不写锚，照常计未读（session-unread-badge 语义）。
   * 思考/工具条目不进段数锚增量（与 msg_count 只数可见回复段的口径一致）。
   */
  private onTurnAppend(sessionId: string, incoming: ConversationEntryView[]): void {
    if (sessionId !== this.sessionKey || incoming.length === 0) return
    const maxKnown = Math.max(
      this.entries.reduce((m, e) => Math.max(m, e.position), 0),
      this.streamEntries.reduce((m, e) => Math.max(m, e.position), 0),
    )
    const fresh = incoming.filter((e) => e.position > maxKnown)
    if (fresh.length === 0) return
    this.streamEntries.push(...fresh)
    this.rebuildUnits()
    this.recomputeSeam()
    if (this.sticky) {
      if (this.msgCount != null) {
        this.streamMsgBonus += fresh.filter(
          (e) => e.element_type === 'markdown' || e.element_type === 'error',
        ).length
      }
      this.scheduleMarkSeen()
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
    // 流式期间快照还没追上的可见段（6.1）以增量补丁一并计入，rail 徽标
    // 与 seam 才不会在流式会话上闪现。
    const anchor =
      this.msgCount != null ? this.msgCount + this.streamMsgBonus : undefined
    writeCursor(this.sessionKey, value, anchor)
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

  /**
   * 2.3（D4）：operator 回合 = 轻底色块——tinted 背景保留，卡片边框/阴影
   * 去掉；已收到角标（3.2）语义与形态不变。
   */
  private renderOperatorUnit(u: OperatorUnit, receipt: boolean) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    return html`
      <div class="turn-block is-user">
        <div class="avatar user">你</div>
        <div class="msg-block">
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
   * 自然对话流（2.1，D4）：agent 回合不再套气泡卡片——作者标签行
   * （display → slug → assistant 回退链，3.1，D1；首字形头像语义不变）
   * 裸排小字行，正文段与过程折叠按 run 序直接铺在对话流里。
   */
  private renderAgentUnit(u: AgentUnit) {
    const iso = isoTime(u.startedAt)
    const ts = formatTime(u.startedAt)
    const label = resolveAgentDisplay(this.agentDisplay)
    const avatar = label === 'assistant' ? 'AI' : graphemes(label)[0]?.toUpperCase() ?? 'AI'
    return html`
      <div class="turn-block is-assistant" data-turn-position=${u.position}>
        <div class="avatar assistant">${avatar}</div>
        <div class="flow">
          <div class="meta">
            <span class="author">${label}</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          ${u.runs.map((r) => this.renderAgentRun(r))}
        </div>
      </div>
    `
  }

  private renderAgentRun(r: AgentRun) {
    if (r.type === 'text') {
      return html`<div class="body">${unsafeHTML(renderMarkdown(r.content))}</div>`
    }
    return this.renderProcessRun(r)
  }

  /**
   * 一 run 一折（2.1/D1）：过程 run 渲染为默认收起的折叠（原生 details/
   * summary，键盘可展开 2.5），嵌在正文段之间的真实发生位置。open 状态以
   * run id（首条目 position，D2）记在 {@link foldOpen}——流式全量重分组
   * 后按 id 恢复，未命中即收起（流式期间永不自动展开，D3）。summary 行
   * 实时渲染「进行中条目的结构化 title（缺省通用标签）+ 条目计数」——
   * 数据源即增量 refetch 后的 run 内容，无需新事件。
   */
  private renderProcessRun(r: ProcessRun) {
    const id = String(r.position)
    const { label, full } = processRunSummary(r)
    return html`
      <details
        class="fold process-fold"
        data-process-id=${id}
        data-process-count=${r.items.length}
        .open=${this.foldOpen.get(id) === true}
      >
        <summary @click=${this.foldToggle(id)} title=${full ?? nothing}>
          <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
          <span class="label">process</span>
          <span class="running">${label}</span>
          <span class="fold-count">${r.items.length}</span>
        </summary>
        <div class="body fold-body">
          ${repeat(r.items, (it) => it.position, (it) => this.renderProcessItem(it))}
        </div>
      </details>
    `
  }

  /**
   * 折叠开合写入 {@link foldOpen}（D2）。click 监听挂在 summary 上（挂在
   * details 上会连折叠体内二级折叠的点击一起收到），而 click 先于 details
   * 的默认翻转（activation behavior 在事件派发后才生效）——此刻 `open`
   * 仍是旧值，翻转后的状态即取反。
   */
  private foldToggle(id: string): (ev: Event) => void {
    return (ev) => {
      const summary = ev.currentTarget as HTMLElement
      const details = summary.parentElement as HTMLDetailsElement
      if (details.tagName !== 'DETAILS') return
      this.foldOpen.set(id, !details.open)
    }
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
