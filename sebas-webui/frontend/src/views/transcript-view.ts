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
 *     updates live as streamed frames land (D3). Fold open state is
 *     tracked per run id (the run's first entry position) in a local Map so
 *     an expanded fold survives full regroupings and streamed entries append
 *     in place (D2). The collapsed affordance of every fold — process run
 *     and second-level alike — is a lightweight inline link (glyph + title
 *     + count), never a button block or card chrome (fix-webui-streaming-
 *     liveness 4.4); an expanded second-level body over the truncation
 *     threshold renders a preview + explicit omission notice + a "view all"
 *     escape that opens an isolated wa-dialog outside the conversation's
 *     scroll container (4.5);
 *   - markdown is incremental (4.3): the live tail — the last text run of
 *     the last agent turn while `turnLive` (the engine's turn_engaged fact)
 *     — renders as plain text; once the turn settles it is rendered with
 *     the full markdown pipeline exactly once. Historical runs ride Lit's
 *     value-cached unsafeHTML, so streaming frames no longer re-parse
 *     unchanged content;
 *   - error entries (spawn failures) do not join runs — they render as
 *     their own counted error bubbles, positioned in sequence (D5);
 *   - the operator's newest submission shows a low-key "已收到" receipt
 *     badge while no agent output has arrived (pure entry-sequence derived
 *     state — no sender-side state machine);
 *   - the assistant author label resolves via `agentDisplay`
 *     (display → slug → assistant, D1) with a first-grapheme text avatar.
 *
 * The seen-boundary seam counts TURNS, never entries: it sits above the
 * first turn whose visible content extends beyond the stored read anchor and
 * never splits a turn (D5). The anchor is the SAME per-browser segment count
 * the rail's unread badge uses (unread-cursor, single `{anchor_count}`
 * field — session-parallel-liveness-and-unread-polish 2.3, design D3): the
 * seam is derived from the cumulative visible-segment total at each turn, so
 * the seam and the badge can never disagree about what has been read.
 *
 * Scroll behaviour (fix-webui-streaming-liveness 4.1, rewritten):
 *   - `sticky` means "the reader never deliberately scrolled up": any
 *     scroll within NEAR_BOTTOM_PX of the bottom (re)engages it, anything
 *     further disengages it. No seam-relative judgment — the former
 *     scrollTop≈seamTop comparison misread the programmatic seam-centering
 *     scroll as a deliberate scroll-up and froze auto-follow;
 *   - while `sticky`, every update commits an INSTANT pin to the bottom
 *     (no smooth scrolling — animated commits never catch up with
 *     per-frame streaming);
 *   - a near-bottom scroll marks turns as seen (250ms debounce, monotonic
 *     — only ever advances the boundary)
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { repeat } from 'lit/directives/repeat.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import type { ConversationEntryView } from '../api/client.js'
import { icon } from '../components/icons.js'
import { renderMarkdown } from '../components/markdown.js'
import { readAnchorCount, writeSeen as writeCursor } from './unread-cursor.js'
import { sharedWs } from '../api/shared-ws.js'
// 4.5：「查看全部」隔离弹层（独立于会话滚动容器的 wa-dialog）。
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'

/** Bottom-scroll threshold for "mark-as-seen" detection. */
const NEAR_BOTTOM_PX = 80
/** Debounce window for mark-as-seen writes. */
const MARK_SEEN_DEBOUNCE_MS = 250

/**
 * 曾以「空流」（0 回合）渲染过的会话 key（polish-workbench-walkthrough-ux 3.1）：
 * 这些会话的首个回合若在聚焦 + 可见 + 贴底时到达，算「亲眼看着到达」的首交换，
 * 锚从空流建立。模块级而非实例字段——秒回场景下 dashboard 可能重建组件实例。
 * 一旦某 key 以非空回合渲染过即出表（一次性消费），所以表里只剩仍为空的会话。
 */
const emptyStreamSessions = new Set<string>()

/**
 * 展开条目的截断阈值（fix-webui-streaming-liveness 4.5，D5.5）：行数或
 * 字符数任一超限即截断显示（取先到）。实现侧常量，后续可调。
 */
export const TRUNCATE_LINES = 40
export const TRUNCATE_CHARS = 8_000

/** `truncateHtml` 的结果：预览段 + 截断判定 + 如实的省略量。 */
export interface TruncateResult {
  preview: string
  truncated: boolean
  /** 预览之后省略的行数（按换行切分）。 */
  omittedLines: number
  /** 预览之后省略的字符数。 */
  omittedChars: number
}

/**
 * 展开条目的截断视图（4.5，D5.5，纯函数）：内容超过 {@link TRUNCATE_LINES}
 * 行或 {@link TRUNCATE_CHARS} 字符（任一先到即触发）时只保留预览段，并
 * 如实报告省略的行数与字符数——正文渲染方据此展示「已截断」明示与
 * 「查看全部」出口。未超限时 preview 即原文、truncated 为 false。
 */
export function truncateHtml(entry: { content: string }): TruncateResult {
  const content = entry.content
  const totalChars = content.length
  const lines = content.split('\n')
  let cut = totalChars
  if (lines.length > TRUNCATE_LINES) {
    cut = Math.min(cut, lines.slice(0, TRUNCATE_LINES).join('\n').length)
  }
  if (totalChars > TRUNCATE_CHARS) {
    cut = Math.min(cut, TRUNCATE_CHARS)
  }
  const preview = content.slice(0, cut)
  const omittedLines = Math.max(0, lines.length - preview.split('\n').length)
  return {
    preview,
    truncated: cut < totalChars,
    omittedLines,
    omittedChars: totalChars - preview.length,
  }
}

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

/**
 * One turn's visible-segment contribution（2.3，与后端 `count_chat_messages`
 * 同口径）：agent 回合里每个 text run（相邻 markdown 合并）计 1；error 气泡
 * 按合并计数计（每条 error 后端各计 1）；operator 提交与 process 条目不计。
 * 累加即得「读到此回合为止已见的段数」，seam 与徽标共用同一份段锚。
 */
export function unitSegmentCount(unit: TurnUnit): number {
  if (unit.kind === 'operator') return 0
  if (unit.kind === 'error') return unit.entry.count ?? 1
  return unit.runs.filter((r) => r.type === 'text').length
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
 * 工具条目被拒绝/失败的判定（polish-workbench-walkthrough-ux 5.6，纯函数）：
 * 审批被拒（fake-claude 的 deny → is_error 结果）与明确的拒绝措辞都不该
 * 再挂 ✓ 成功标记。词表按内容/标题里的显式拒绝记号判定（中文「已拒绝」、
 * ❌、英文 denied / rejected），不猜工具语义。
 */
export function toolResultDenied(content: string, title?: string | null): boolean {
  const marker = /已拒绝|denied|rejected|❌/i
  return marker.test(content) || (typeof title === 'string' && marker.test(title))
}

/** 拒绝态标签：去掉成功 ✓ 前缀，改 ✗（5.6「改 ✗ 或已拒绝」）。 */
export function deniedLabel(label: string): string {
  return '✗ ' + label.replace(/^✓\s*/, '')
}

/**
 * 被拒工具条目的展开体正文（fix-webui-qa-defects 5.3，纯函数）：core 写入
 * 的 ToolEnd 内容以 `✓ **tool**` 开头，被拒条目的折叠标题已挂 ✗（deniedLabel），
 * 展开详情的首行却仍是 ✓——与折叠态自相矛盾。这里把首行的成功前缀改写为
 * ✗（无前缀的形态原样返回，由调用方决定是否另加角标）。
 */
export function deniedDetailContent(content: string, title?: string | null): string {
  if (!toolResultDenied(content, title)) return content
  return content.replace(/^✓\s*/, '✗ ')
}

/**
 * 错误气泡的分类标签（fix-webui-qa-defects 5.2，design D5，纯函数）：按
 * `failure_class` 如实渲染——spawn → 「spawn failed」、stall → 「回合停滞」；
 * generic 或旧条目（无分类）→ 中性「错误」。写死的「spawn failed」退役。
 */
export function errorEntryLabel(entry: {
  failure_class?: string | null
  content?: string
}): string {
  switch (entry.failure_class) {
    case 'spawn':
      return 'spawn failed'
    case 'stall':
      return '回合停滞'
    default:
      return '错误'
  }
}

/**
 * Second-level fold label fallback（2.2）: entries without the structured
 * `title` (legacy persisted data) show a generic stable label derived from
 * the element type instead. （5.6）被拒条目去 ✓ 挂 ✗。
 */
export function processItemLabel(item: ProcessItem): { label: string; full: string | null } {
  const title = item.title?.trim() ? item.title : null
  const denied = toolResultDenied(item.content, title)
  if (title) {
    const full = denied ? title.replace(/^✓\s*/, '') : title
    return { label: denied ? deniedLabel(middleTruncate(title)) : middleTruncate(title), full }
  }
  return { label: denied ? deniedLabel(item.elementType) : item.elementType, full: null }
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

/**
 * （5.6）摘要行的拒绝角标：run 内任一条目被拒即整行不显示成功语义——
 * 摘要行是「最近条目」的标签，挂 ✓ 的拒绝结果属于误报。
 */
export function processRunDenied(run: ProcessRun): boolean {
  return run.items.some((it) => toolResultDenied(it.content, it.title))
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
   * rail-declutter-unread D3 / （2.3）：会话当前的服务端可见回复段数
   * （detail/summary payload 的 `msg_count`）。标记已读时写入共享游标的
   * `anchor_count` = max(服务端段数, 本地已渲染段数)，让 rail 徽标与 seam
   * 同步清零；`null`（旧 payload / 测试）= 只用本地渲染段数推进锚。
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
   * （4.3，D5.3）回合在飞：engine 的 `turn_engaged` 事实（dashboard 从
   * detail/summary 读出后下传）。true 时对话末尾的流式文本条目以纯文本
   * 呈现（跳过 markdown 解析——渲染成本不随帧线性增长）；回合定稿（属性
   * 翻 false，随状态刷新到达）后该条目一次性换 markdown 渲染。缺省 false
   * = 历史查看姿态，全部条目走 markdown。
   */
  @property({ attribute: false }) turnLive = false
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
   * （4.5）「查看全部」弹层正在展示的条目；null = 弹层关闭。弹层渲染在
   * 会话滚动容器之外（.scroll 的兄弟节点），置 null 即整棵卸载 DOM——
   * 关闭不在对话滚动面留下任何节点。
   */
  @state() private viewAllEntry: { title: string; content: string } | null = null

  /**
   * （workbench-live-conversation-flow 2.2）流式增量的就地缓冲：turn.append
   * 帧的条目先落这里（position 去重），与 `entries` 属性合并进渲染管线。
   * 快照重取（entries 属性）是收敛基准——重取带来的条目 position 覆盖缓冲
   * 后，缓冲里被覆盖的部分即被裁掉。
   */
  private streamEntries: ConversationEntryView[] = []
  /** turn.append 订阅的退订句柄（connectedCallback 挂，disconnected 摘）。 */
  private unsubscribeTurn: (() => void) | null = null
  /** 文档可见性翻转监听（3.2）：hidden 期间不推进锚，翻回 visible 恢复。 */
  private boundOnVisibility = (): void => this.onVisibilityChange()
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
      /* 4.1（D5.1）：滚动提交是瞬时贴底——smooth 动画在逐帧流式下永远追
         不上目标（下一帧又抬高 scrollHeight，动画互相打断），去掉。 */
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
    .turn-block .body + .process-fold,
    .turn-block .process-fold + .body,
    .turn-block .process-fold + .process-fold {
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
    /* 过程折叠（4.4，D5.4）：collapsed affordance 收敛为**单行行内 link**
       （glyph + 标签 + 进行中标题 + 计数）——无按钮块、无卡框、无大面积
       容器样式。展开体保留轻量分区（虚线顶边），open 状态由组件托管
       （foldOpen Map），显隐即条件渲染。 */
    .turn-block .process-fold {
      margin: var(--sebas-space-2) 0;
      min-width: 0;
    }
    .turn-block .fold-link {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      max-width: 100%;
      padding: 0;
      background: none;
      border: none;
      cursor: pointer;
      font: inherit;
      font-size: 0.78rem;
      color: var(--sebas-text-dim);
      text-align: left;
      transition: color var(--sebas-dur) var(--sebas-ease);
    }
    .turn-block .fold-link:hover {
      color: var(--sebas-accent);
    }
    .turn-block .fold-link:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .turn-block .fold-link .kind-icon {
      display: grid;
      place-items: center;
      width: 18px;
      height: 18px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .turn-block .fold-link .label {
      text-transform: uppercase;
      letter-spacing: 0.08em;
    }
    /* 进行中条目的 title（结构化 title 或通用标签）：mono 小字、不吃
       标签的大小写（路径参数被 uppercase 会变形），超长省略号收干。 */
    .turn-block .fold-link .running {
      min-width: 0;
      font-family: var(--sebas-font-mono);
      font-size: 0.76rem;
      color: var(--sebas-text-faint);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .turn-block .fold-link:hover .running {
      color: var(--sebas-accent);
    }
    .turn-block .fold-link .fold-count {
      color: var(--sebas-accent);
      font-variant-numeric: tabular-nums;
    }
    /* 展开内容：work-block-body 同款（0.82rem/1.6 + 虚线顶边）。挂 .body
       复用 markdown 排版规则（后写的字号覆盖之）。 */
    .turn-block .fold-body {
      padding: 8px 12px 12px;
      font-size: 0.82rem;
      line-height: 1.6;
      margin-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
    }
    /* 过程折叠的二级折叠（4.4/4.5）：collapsed 同为单行行内 link；相邻
       条目以虚线分隔，沿用同一视觉语言。 */
    .turn-block .process-item + .process-item {
      margin-top: var(--sebas-space-2);
      padding-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
    }
    .turn-block .process-item .item-link {
      font-family: var(--sebas-font-mono);
      font-size: 0.76rem;
    }
    .turn-block .process-item .item-title {
      min-width: 0;
      overflow-wrap: anywhere;
    }
    .turn-block .process-item .item-body {
      padding-top: var(--sebas-space-2);
      font-size: 0.8rem;
      line-height: 1.6;
    }
    /* （4.5）展开条目的截断：明示省略量 + 「查看全部」行内出口。 */
    .turn-block .truncation-note {
      margin: var(--sebas-space-2) 0 0;
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
    }
    .turn-block .truncation-note .view-all {
      padding: 0;
      background: none;
      border: none;
      cursor: pointer;
      font: inherit;
      color: var(--sebas-accent);
      text-decoration: underline;
      text-underline-offset: 3px;
    }
    .turn-block .truncation-note .view-all:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    /* （4.3）流式中的当前条目：纯文本呈现保住换行语义（未经 markdown
       解析），定稿后换 markdown 渲染。 */
    .turn-block .body.text-live {
      white-space: pre-wrap;
      overflow-wrap: break-word;
    }
    /* （4.5）「查看全部」弹层：会话滚动容器之外的独立 wa-dialog。 */
    .view-all-dialog {
      --wa-panel-width: min(720px, 90vw);
    }
    .view-all-body {
      min-width: 0;
      font-size: 0.85rem;
      line-height: 1.6;
      max-height: min(70vh, 640px);
      overflow-y: auto;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    this.recomputeSeam()
    window.addEventListener('resize', this.boundOnResize)
    // （3.2）后台 tab 到达不推进锚：可见性翻转只影响后续 arrivals 的判定，
    // 翻回 visible 不追溯清账（隐藏期间的到达保持 unseen，回到页面后由
    // seam/徽章如实呈现）。
    document.addEventListener('visibilitychange', this.boundOnVisibility)
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
    document.removeEventListener('visibilitychange', this.boundOnVisibility)
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
      this.foldOpen.clear()
    }
    if (changed.has('entries') || changed.has('sessionKey')) {
      // 快照收敛：position 已被属性覆盖的流式条目裁掉，只留快照还没
      // 追上的尾巴（乱序竞态下绝不回退视图）。
      const snapMax = this.entries.reduce((m, e) => Math.max(m, e.position), 0)
      this.streamEntries = this.streamEntries.filter((e) => e.position > snapMax)
      this.rebuildUnits()
      this.recomputeSeam()
      // （3.1）空流建立锚点：占位会话从 0 回合转出首回合、且操作员聚焦、
      // 文档可见、贴底时，这是「亲眼看着到达」的首交换——锚从空流状态建立，
      // 不画 seam、不闪徽章（session-unread-badge「first focused exchange of a
      // fresh placeholder」）。秒回场景下回复经快照而非 turn.append 到达，
      // 且 dashboard 可能连同 sessionKey 一起重渲染（组件实例被重建），
      // 所以判据用模块级「该会话曾以空流渲染过」而非实例内的回合数差值。
      // 打开既有会话不在此列：那种会话从没以空流出现过，seam 必须保留。
      this.settleEmptyStreamAnchor()
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
    // （3.1/3.2）聚焦 + 文档可见 + 贴底：到达即推进共享游标（亲眼看着到的
    // 内容不再挂未读）；文档隐藏（后台 tab）时不推进——回来看 seam/徽章。
    if (this.sticky && this.docVisible()) {
      // 贴底读流：段锚随渲染推进（写的是与徽标同一个 anchor_count 字段，
      // 2.3）——本地已渲染段数已含流式尾巴，无需另设增量补丁。
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
    if (
      changed.has('entries') ||
      changed.has('sessionKey') ||
      changed.has('seamIndex') ||
      // 4.1（D5.1）：流式帧经 turn.append → rebuildUnits 只改 turnUnits
      // （entries/seamIndex 都可能没变）——不监听它，纯增量就不触发滚动。
      // turnLive 翻转（定稿换 markdown）同样改变渲染高度，一并跟随。
      changed.has('turnUnits') ||
      changed.has('turnLive')
    ) {
      // Wait one frame for layout to settle, then scroll. Without the
      // rAF, scrollHeight can lag the freshly-inserted entries.
      requestAnimationFrame(() => this.applyAutoScroll())
    }
  }

  /** 文档可见性（3.2）：后台 tab 中的到达不算「看着到达」。 */
  private docVisible(): boolean {
    return document.visibilityState === 'visible'
  }

  /**
   * 空流锚点（3.1）：一个会话若曾以 0 回合在本浏览器渲染过（新建占位），
   * 它随后的首个回合到达且操作员聚焦、可见、贴底时，就是「亲眼看着到达」
   * 的首交换——锚从空流状态建立，seam 与 rail 徽章都不该出现
   * （session-unread-badge「first focused exchange of a fresh placeholder」）。
   *
   * 登记与消费用模块级表而非实例字段：秒回场景下回复经快照到达，dashboard
   * 可能连同 sessionKey 一起重渲染（组件实例重建、实例内的回合数差值作废），
   * 模块表跨实例仍有效。消费是一次性的——某个 key 一旦以非空回合渲染过，
   * 就从此出表，所以表里残留的必然是「仍为空」的会话，切回一个既有会话
   * 不会误推进它的锚（那种 seam 必须保留）。
   */
  private settleEmptyStreamAnchor(): void {
    const key = this.sessionKey
    if (!key) return
    if (this.turnUnits.length === 0) {
      emptyStreamSessions.add(key)
      return
    }
    if (!emptyStreamSessions.delete(key)) return
    if (!this.sticky || !this.docVisible()) return
    // （2.3，D3）锚已统一为段计数：写锚 = 当前已渲染段数（含流式尾巴），
    // 与 seam/rail 徽标同一条锚线；游标模块保证单调不回退，已在这里看到
    // 的首交换不再挂未读。
    this.writeSeen()
    // 同一更新周期内重算：seam 不闪现（写锚后紧接着 render）。
    this.recomputeSeam()
  }

  private onVisibilityChange(): void {
    // 翻回 visible 不追溯标记已读（spec：隐藏期到达保持 unseen，回去后被
    // 如实标记）；仅当翻离时取消待写的标记——那一刻之后的 arrival 不该被
    // 旧的贴底状态误清。
    if (!this.docVisible() && this.markSeenTimer !== null) {
      clearTimeout(this.markSeenTimer)
      this.markSeenTimer = null
    }
  }

  /** The current scroll container, if any. */
  private scrollEl: HTMLElement | null = null

  // ---- localStorage helpers --------------------------------------------

  // （2.3，D3）锚统一为段计数单字段：读写都走共享游标模块 unread-cursor
  // （`sebas:seen:<key>` 键不变；含 seen_ts 的旧 JSON 读为无锚 = fully
  // read，下次写入被纯 {anchor_count} 覆写）。seam 与徽标共用同一水位。

  private readSeen(): number | null {
    return readAnchorCount(this.sessionKey)
  }

  /**
   * 已渲染回合序列的可见段数累计（`unitSegmentCount` 前缀和）：第 i 项 =
   * 读到第 i 个回合为止已见的段数，seam 判定与 mark-seen 写入共用。
   */
  private segmentTotals(): number[] {
    const totals: number[] = []
    let acc = 0
    for (const unit of this.turnUnits) {
      acc += unitSegmentCount(unit)
      totals.push(acc)
    }
    return totals
  }

  private writeSeen(): void {
    // 写入锚 = max(服务端段数, 本地已渲染段数)——流式期间快照未追上的
    // 可见段（6.1）已含在本地渲染里，rail 徽标与 seam 不闪现；单调 max
    // 由游标模块保证。
    const totals = this.segmentTotals()
    const local = totals.length > 0 ? totals[totals.length - 1]! : 0
    const anchor = this.msgCount != null ? Math.max(this.msgCount, local) : local
    writeCursor(this.sessionKey, anchor)
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
    const anchor = this.readSeen()
    if (anchor === null || this.turnUnits.length === 0) {
      // 无锚 = fully read（首次访问 / 清缓存 / 旧 seen_ts 数据）：历史不
      // 因换浏览器集体冒缝（2.3，spec「first visit shows no unread」）。
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    // seam 落在第一个「可见段累计超过锚」的回合上方——按回合计数、绝不
    // 切开回合（D5），且与徽标读的是同一条段锚。
    const totals = this.segmentTotals()
    const idx = totals.findIndex((total) => total > anchor)
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
    this.writeSeen()
    this.seamIndex = null
    this.unseenCount = 0
  }

  // ---- scroll handling --------------------------------------------------

  private onScroll(): void {
    const el = this.scrollEl
    if (!el) return
    // 4.1（D5.1）：贴底跟随判定只看几何——距底 ≤ 阈值即（重新）贴底跟随，
    // 更远即用户主动上滚（停止跟随）。不再用 seam 相对位移：旧判定把
    // 「自动滚到 seam 中心」的编程滚动误读成用户上滚，sticky 被翻 false
    // 后自动滚动整体停摆——未读缝卡死流式跟随的根源。
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
    if (distanceFromBottom <= NEAR_BOTTOM_PX) {
      if (!this.sticky) this.sticky = true
      if (this.docVisible()) this.scheduleMarkSeen()
    } else if (this.sticky) {
      this.sticky = false
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
    const totals = this.segmentTotals()
    const local = totals.length > 0 ? totals[totals.length - 1]! : 0
    const anchor = this.readSeen() ?? 0
    if (this.turnUnits.length > 0 && local > anchor) {
      this.writeSeen()
      // The seam may have moved or disappeared; update internal state
      // and re-render without scheduling another auto-scroll — the user
      // is already where they want to be.
      const prev = this.seamIndex
      this.recomputeSeam()
      if (prev !== this.seamIndex) this.requestUpdate('seamIndex')
    }
  }

  /**
   * Apply the scroll behaviour for the current frame（4.1，D5.1）: sticky
   * = 瞬时贴底（scrollHeight 提交，无 smooth 动画）。未读缝不再参与自动
   * 滚动定位——seam 只是标记与「mark all seen」出口，想读旧内容向上滚动
   * 即自然解除 sticky。
   */
  private applyAutoScroll(): void {
    const el = this.scrollEl
    if (!el || !this.sticky) return
    const previous = el.style.scrollBehavior
    el.style.scrollBehavior = 'auto'
    el.scrollTop = el.scrollHeight
    el.style.scrollBehavior = previous
  }

  // ---- render -----------------------------------------------------------

  render() {
    const showSeam = this.unseenCount > 0
    // 3.2（D4）：角标是派生态——纯 entry 序语义：最后一条渲染单元仍是
    // 操作者提交即显示（排队窗非 working 也成立）。agent entry 一到（收尾
    // 不再是 prompt）条件即不成立，角标消失。
    const receipt = awaitingReceipt(this.turnUnits)
    // 4.2（D5.2）：seam 真假同一模板字面量——节点恒渲染，显隐只切 hidden
    // 属性。流式中的翻转不再在两个模板分支间切换，Lit 的既有条目 DOM 身份
    // 保留（不整洞重建），二级折叠展开态随之保留。
    const seam = html`
      <div class="seam" ?hidden=${!showSeam} data-count=${this.unseenCount} role="status">
        <span class="pill"
          ><span class="count">~${this.unseenCount} new</span> since you last viewed</span
        >
        <button type="button" class="link" @click=${this.markAllSeen}>
          mark all seen
        </button>
      </div>
    `
    return html`
      <div class="scroll" role="log" aria-label="Session conversation">
        ${this.seamIndex === null ? seam : nothing}
        ${this.turnUnits.map((u, i) =>
          // 4.2：map 项是**同一个**模板字面量——seam 有无是项内 child part
          // 的值翻转，不是项模板身份切换。seam 落点移动时其余回合的 DOM
          // 身份照旧保留（Lit 按索引复用节点）。
          html`${i === this.seamIndex ? seam : nothing}${this.renderUnit(
            u,
            receipt && i === this.turnUnits.length - 1,
          )}`,
        )}
      </div>
      ${this.renderViewAllDialog()}
    `
  }

  /**
   * （4.5）「查看全部」隔离弹层：渲染在会话滚动容器（.scroll）之外的
   * 独立 wa-dialog；关闭（wa-hide，含 Esc/背板/关闭钮）即置 null——
   * 条件渲染移除整棵子树，对话滚动面不残留任何节点。
   */
  private renderViewAllDialog() {
    const entry = this.viewAllEntry
    if (!entry) return nothing
    return html`
      <wa-dialog
        class="view-all-dialog"
        data-testid="view-all-dialog"
        label=${entry.title}
        .open=${true}
        @wa-hide=${this.closeViewAll}
      >
        <div class="body view-all-body">${unsafeHTML(renderMarkdown(entry.content))}</div>
      </wa-dialog>
    `
  }

  private closeViewAll = (): void => {
    this.viewAllEntry = null
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
            <span class="author error">${errorEntryLabel(e)}</span>
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
            <span class="author you">你</span>
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
          ${u.runs.map((r, i) => this.renderAgentRun(r, this.isLiveTextTail(u, i)))}
        </div>
      </div>
    `
  }

  /**
   * 该 run 是否为「流式中的当前文本条目」（4.3，D5.3）：末尾 agent 回合的
   * 最后一个 text run，且回合仍在飞（turnLive）。仅它以纯文本增量呈现；
   * 其余 run（历史文本、过程折叠）照常 markdown。
   */
  private isLiveTextTail(u: AgentUnit, runIndex: number): boolean {
    return (
      this.turnLive &&
      this.turnUnits[this.turnUnits.length - 1] === u &&
      runIndex === u.runs.length - 1 &&
      u.runs[runIndex].type === 'text'
    )
  }

  private renderAgentRun(r: AgentRun, liveText: boolean = false) {
    if (r.type === 'text') {
      if (liveText) {
        // 4.3（D5.3）：流式中的当前条目用纯文本呈现（Lit 文本绑定，零
        // markdown 解析、无 unsafeHTML 重解析）——渲染成本不随帧线性增长。
        // 回合定稿（turnLive 翻 false）后本方法走回 markdown 分支，一次性
        // 完成富文本渲染。
        return html`<div class="body text-live">${r.content}</div>`
      }
      return html`<div class="body">${unsafeHTML(renderMarkdown(r.content))}</div>`
    }
    return this.renderProcessRun(r)
  }

  /**
   * 一 run 一折（4.4，D5.4）：collapsed 呈现是**单行行内 link**——类型
   * glyph + `process` 标签 + 进行中条目 title（缺省通用标签）+ 条目计数，
   * 无按钮块、无边框卡框。点击切换展开/收起（状态由组件托管，见
   * {@link toggleFold}）；展开体按 run id 记在 {@link foldOpen}（D2）——
   * 流式全量重分组后按 id 恢复，未命中即收起（流式期间永不自动展开，
   * D3）。summary 行实时更新「进行中 title + 计数」，数据源即增量到达的
   * run 内容。
   */
  private renderProcessRun(r: ProcessRun) {
    const id = String(r.position)
    const open = this.foldOpen.get(id) === true
    const { label, full } = processRunSummary(r)
    return html`
      <div class="process-fold" data-process-id=${id} data-process-count=${r.items.length}>
        <button
          type="button"
          class="fold-link"
          data-testid="process-fold-link"
          aria-expanded=${open}
          title=${full ?? nothing}
          @click=${this.toggleFold(id)}
        >
          <span class="kind-icon" aria-hidden="true">${icon('zap', 11)}</span>
          <span class="label">process</span>
          <span class="running">${label}</span>
          <span class="fold-count">${r.items.length}</span>
        </button>
        ${open
          ? html`<div class="body fold-body">
              ${repeat(r.items, (it) => it.position, (it) => this.renderProcessItem(it))}
            </div>`
          : nothing}
      </div>
    `
  }

  /**
   * 折叠开合写入 {@link foldOpen}（D2/4.4）：link 化后开合完全由组件托管
   * （没有原生 details 的默认翻转），显式 requestUpdate 重渲染换显隐。
   */
  private toggleFold(id: string): (ev: Event) => void {
    return () => {
      this.foldOpen.set(id, !(this.foldOpen.get(id) === true))
      this.requestUpdate()
    }
  }

  /**
   * 二级折叠（4.4/4.5）：collapsed 同为单行行内 link（title 概要，
   * `title` 属性保全量；无 title 回退通用标签）；展开体超截断阈值时
   * 只渲预览 + 「已截断」明示 + 「查看全部」出口（弹层见
   * {@link renderViewAllDialog}）。
   */
  private renderProcessItem(it: ProcessItem) {
    const id = `item:${it.position}`
    const open = this.foldOpen.get(id) === true
    const { label, full } = processItemLabel(it)
    return html`
      <div class="process-item" data-position=${it.position} data-element-type=${it.elementType}>
        <button
          type="button"
          class="fold-link item-link"
          data-testid="process-item-link"
          aria-expanded=${open}
          title=${full ?? nothing}
          @click=${this.toggleFold(id)}
        >
          <span class="item-title">${label}</span>
        </button>
        ${open ? this.renderItemBody(it) : nothing}
      </div>
    `
  }

  /** （4.5）二级条目的展开体：超阈值截断 + 明示省略量 + 「查看全部」。
   *  （fix-webui-qa-defects 5.3）被拒条目的展开详情与折叠标题一致挂 ✗。 */
  private renderItemBody(it: ProcessItem) {
    const content = deniedDetailContent(it.content, it.title)
    const cut = truncateHtml({ ...it, content })
    if (!cut.truncated) {
      return html`<div class="body item-body">${unsafeHTML(renderMarkdown(content))}</div>`
    }
    return html`
      <div class="body item-body" data-truncated>
        ${unsafeHTML(renderMarkdown(cut.preview))}
        <p class="truncation-note" data-testid="truncation-note">
          已截断：省略 ${cut.omittedLines} 行 / ${cut.omittedChars} 字符
          <button type="button" class="view-all" @click=${() => this.openViewAll(it)}>
            查看全部
          </button>
        </p>
      </div>
    `
  }

  private openViewAll(it: ProcessItem): void {
    const { label } = processItemLabel(it)
    this.viewAllEntry = { title: label, content: it.content }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-transcript-view': SebasTranscriptView
  }
}
