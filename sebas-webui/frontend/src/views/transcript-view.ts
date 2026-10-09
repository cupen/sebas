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
 *   - notice entries (zero-output turn synthetic notices,
 *     close-acceptance-blind-spots 4.2) likewise stay standalone and render
 *     as neutral info bars — same block shape as the error bubble, no
 *     failure semantics (no red, no count);
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
import {
  clearOpeningSeam,
  peekOpeningSeam,
  readAnchorCount,
  writeSeen as writeCursor,
} from './unread-cursor.js'
import { sharedWs } from '../api/shared-ws.js'
// （fix-webui-qa-findings D2）mode 契约条目的控制面词 → 人读标签。
import { modeBadgeLabel } from './mode-vocabulary.js'
// 4.5：「查看全部」隔离弹层（独立于会话滚动容器的 wa-dialog）。
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'

/** Bottom-scroll threshold for "mark-as-seen" detection. */
const NEAR_BOTTOM_PX = 80
/** Debounce window for mark-as-seen writes. */
const MARK_SEEN_DEBOUNCE_MS = 250
/**
 * （fix-webui-qa-round2 2.5，D-B13）大批量分片阈值：单帧到达条数超过它的
 * 部分进入 rAF 分片摄入（每片 ≤FLOOD_CHUNK 条），避免一次 append 上千条
 * 造成的秒级主线程冻结；小批量（drip 首包）仍直渲染。
 */
const FLOOD_CHUNK = 50

/**
 * 曾以「空流」（0 回合）渲染过的会话 key（polish-workbench-walkthrough-ux 3.1）：
 * 这些会话的首个回合若在聚焦 + 可见 + 贴底时到达，算「亲眼看着到达」的首交换，
 * 锚从空流建立。模块级而非实例字段——秒回场景下 dashboard 可能重建组件实例。
 * 一旦某 key 以非空回合渲染过即出表（一次性消费），所以表里只剩仍为空的会话。
 */
const emptyStreamSessions = new Set<string>()

/**
 * 空流登记的外部入口（fix-webui-qa-defects-round3 6.1）：真实应用里聚焦
 * 会话为空时 dashboard 渲染的是自己的空态占位（`empty-stream` div）而非
 * transcript 组件——组件内的 `settleEmptyStreamAnchor` 登记分支永远不跑，
 * 占位会话的首交换只能靠巧合推进锚（QA round5：新建即聚焦会话的首个交换
 * 冒出徽标 + 「~1 new」缝且不消）。dashboard 在渲染空态时经此登记，之后
 * 首回合到达仍按「看着到达」消费（聚焦 + 可见 + 贴底 guard 不变）。幂等。
 */
export function registerEmptyStreamSession(sessionKey: string): void {
  if (sessionKey) emptyStreamSessions.add(sessionKey)
}

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
  /**
   * （fold-tool-calls-into-process-tree 4.1）上游 tool-use call id：前端按
   * 它把一次调用的 📖 条目与 ✓ 结果条目精确配对成一块（并行同名工具不靠
   * 位置猜）。`null`/缺省 = 旧持久化条目无 id（退化为未配对自成一块）。
   */
  toolUseId?: string | null
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

/**
 * （fold-tool-calls-into-process-tree 4.1，D1/D2）一次工具调用的合并块：
 * 📖 调用条目与 ✓ 结果条目按 `toolUseId` 精确配对后合并为一个过程折叠的
 * 子折叠——收起标题 = 调用态结构化标题 + ✓/✗ 结果章，展开体 = 参数段 +
 * 结果段。`result === null` = 结果未到（回合中断 / 转录截断 / 并行在跑）
 * ——调用自成一块（调用态标题，无章），不丢参数。`position`（调用条目
 * position）是块的 DOM 身份。
 */
export interface ToolCallBlock {
  type: 'tool_call'
  /** The 📖 invocation entry (or a lone ✓/✗ entry when the invocation is missing). */
  invocation: ProcessItem
  /** The paired ✓/✗ result entry, when it has landed. */
  result: ProcessItem | null
  /** The invocation entry's transcript position — the block's DOM identity. */
  position: number
}

/**
 * 过程折叠体的一个节点（fold-tool-calls-into-process-tree 4.2）：thinking
 * 条目保持二级条目形态，工具条目一律合并为 {@link ToolCallBlock}（单棵
 * 过程树——不再有顶层 tool_result 块）。
 */
export type ProcessNode = ProcessItem | ToolCallBlock

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

/**
 * 零输出回合的中性提示条（close-acceptance-blind-spots 4.2，design D3）：
 * core 在空回合收尾时追加的合成 `notice` 条目，独立成单元——与错误气泡
 * 同构但走中性信息条渲染分支（非错误红泡）。
 */
export interface NoticeUnit {
  kind: 'notice'
  entry: ConversationEntryView
}

/**
 * （fix-webui-qa-findings D2）模式切换契约条目单元：`permission_mode_result`
 * 是一等条目类型（权限 spec「first-class entry, not folded into generic
 * markdown」），content 是 JSON 载荷 `{request_id, ok, mode, detail}`。
 * 渲染为模式标签条：成功显示生效模式，失败显示失败成因（可辨识）。
 */
export interface ModeResultUnit {
  kind: 'mode_result'
  entry: ConversationEntryView
}

/**
 * （fix-webui-qa-round8 1.2）升级降级契约条目单元：`escalate_downgrade`
 * 是一等条目类型，content 是 JSON 载荷 `{request_id, tool, reason, detail}`。
 * 渲染为中性系统条目（含原因文本）——ACP 边界的升级降级不再无痕。
 */
export interface EscalateDowngradeUnit {
  kind: 'escalate_downgrade'
  entry: ConversationEntryView
}

/**
 * （fix-webui-qa-round8 5.2）模型切换留痕条目单元：`model_change` 是一等
 * 条目类型，content 是 JSON 载荷 `{from, to}`。渲染为中性系统条目（含新旧
 * 模型名）。
 */
export interface ModelChangeUnit {
  kind: 'model_change'
  entry: ConversationEntryView
}

export type TurnUnit =
  | OperatorUnit
  | AgentUnit
  | ErrorUnit
  | NoticeUnit
  | ModeResultUnit
  | EscalateDowngradeUnit
  | ModelChangeUnit

/** 过程条目（D1）：进入过程 run 的 element_type 词表。 */
function isProcessEntry(e: ConversationEntryView): boolean {
  return e.element_type === 'thinking' || e.element_type === 'tool'
}

/**
 * （fix-webui-qa-round2 1.3，D-C3a）工具结果条目的已决判定（纯函数）：仅
 * `tool` 条目参与——完成态标题带 ✓/✗ 前缀（引擎 ToolEnd 的稳定约定），或
 * 内容携带显式拒绝记号。thinking 条目绝不参与（正文提及「denied」不属于
 * 结果）。未完成工具（仅 ToolStart）不满足任一条件，留在过程折叠内。
 */
export function isDecidedToolResult(e: {
  element_type?: string
  content: string
  title?: string | null
}): boolean {
  if (e.element_type !== undefined && e.element_type !== 'tool') return false
  if (toolResultDenied(e.content, e.title)) return true
  return typeof e.title === 'string' && /^[\u2713\u2717]/.test(e.title.trim())
}

/**
 * Split ONE agent turn's entry sequence into alternating runs（2.1，D1 —
 * 纯函数）: contiguous markdown entries concatenate into a text run,
 * contiguous thinking/tool entries accumulate into a process run. A kind
 * change opens the next run — that is what keeps each process fold at the
 * position where its entries actually happened, between the text segments.
 * （fold-tool-calls-into-process-tree 4.2）工具条目**全部留在过程 run 内**
 * （原 D-C3a 的顶层 lift 退役）：过程折叠是单棵树，调用与结果在折叠体内
 * 按 id 配对合并（见 {@link mergeToolCalls}）。Error entries never reach
 * this function: `groupConversation` routes them into standalone counted
 * bubbles upstream (D5). Callers pre-filter empty entries (they carry
 * nothing to display and do not break a run).
 */
export function splitAgentRuns(entries: ConversationEntryView[]): AgentRun[] {
  const runs: AgentRun[] = []
  for (const e of entries) {
    const item: ProcessItem = {
      elementType: e.element_type,
      content: e.content,
      title: e.title ?? null,
      toolUseId: e.tool_use_id ?? null,
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
    // （fix-webui-qa-round8 7.2）拼接前逐条收口未闭合围栏：一个条目的坏
    // 围栏只影响它自己渲染成代码块，不再吞掉 run 里它之后的全部条目。
    if (last?.type === 'text') {
      last.content += closeUnclosedFence(e.content)
      continue
    }
    runs.push({ type: 'text', content: closeUnclosedFence(e.content), position: e.position })
  }
  return runs
}

/**
 * （fold-tool-calls-into-process-tree review F1，纯函数）跨 run 配对预通过：
 * native 载体对每次泊车/决策向转录落 markdown 条目（⏳ awaits approval /
 * 🛡 policy），会把同一调用的结果条目切进调用之后的另一个过程 run——
 * {@link mergeToolCalls} 只在单 run 内配对，被门控的调用于是呈两块（结果
 * 与调用分家、调用块与汇总行无章），恰是 spec 禁止的「依赖落账相邻性」。
 * 这里在整个 agent 回合范围内按 id 重聚：已决结果搬回其调用所在 run
 * （插在调用条目之后，重复结果不认领、留原地自成一块），搬空的过程 run
 * 丢弃；正文 run（⏳/🛡 停留处）原样保留。配对仍是纯派生：每次分组都从
 * 完整条目序列重算。
 */
export function foldCrossRunToolResults(runs: AgentRun[]): AgentRun[] {
  const shape = (it: ProcessItem) => ({
    element_type: it.elementType,
    content: it.content,
    title: it.title,
  })
  const home = new Map<string, { run: number; index: number }>()
  runs.forEach((r, ri) => {
    if (r.type !== 'process') return
    r.items.forEach((it, ii) => {
      if (
        it.elementType === 'tool' &&
        it.toolUseId &&
        !home.has(it.toolUseId) &&
        !isDecidedToolResult(shape(it))
      ) {
        home.set(it.toolUseId, { run: ri, index: ii })
      }
    })
  })
  if (home.size === 0) return runs
  const claimed = new Set<string>()
  const adopt = new Map<number, { after: number; item: ProcessItem }[]>()
  const next: AgentRun[] = runs.map((r, ri) => {
    if (r.type !== 'process') return r
    const stay: ProcessItem[] = []
    for (const it of r.items) {
      const id = it.toolUseId
      const spot = id ? home.get(id) : undefined
      if (
        it.elementType === 'tool' &&
        id &&
        spot &&
        spot.run < ri &&
        !claimed.has(id) &&
        isDecidedToolResult(shape(it))
      ) {
        claimed.add(id)
        const list = adopt.get(spot.run) ?? []
        list.push({ after: spot.index, item: it })
        adopt.set(spot.run, list)
        continue
      }
      stay.push(it)
    }
    return { ...r, items: stay }
  })
  if (adopt.size === 0) return runs
  return next
    .map((r, ri) => {
      if (r.type !== 'process' || !adopt.has(ri)) return r
      const items = [...r.items]
      for (const { after, item } of (adopt.get(ri) ?? []).sort((a, b) => b.after - a.after)) {
        items.splice(after + 1, 0, item)
      }
      return { ...r, items }
    })
    .filter((r) => r.type !== 'process' || r.items.length > 0)
}

/**
 * （fold-tool-calls-into-process-tree 4.1，纯函数）按 `tool_use_id` 把过程
 * run 里的工具条目配对合并为 {@link ToolCallBlock}：📖 调用开块并在
 * open 表登记（按 id）；✓/✗ 结果经 id 查表并入**自己的**调用块——并行
 * 同名工具（Read A + Read B）不靠位置、不靠工具名，绝不错配。配不上
 * （结果未到 / 转录截断 / 旧条目无 id）的条目自成一块，内容零丢失。配对
 * 是纯派生：每次渲染都从完整条目序列重算，刷新后自然一致。
 */
export function mergeToolCalls(items: ProcessItem[]): ProcessNode[] {
  const out: ProcessNode[] = []
  const open = new Map<string, ToolCallBlock>()
  for (const it of items) {
    if (it.elementType !== 'tool') {
      out.push(it)
      continue
    }
    if (!isDecidedToolResult({ element_type: it.elementType, content: it.content, title: it.title })) {
      const block: ToolCallBlock = {
        type: 'tool_call',
        invocation: it,
        result: null,
        position: it.position,
      }
      out.push(block)
      if (it.toolUseId) open.set(it.toolUseId, block)
      continue
    }
    const block = it.toolUseId ? open.get(it.toolUseId) : undefined
    if (block && !block.result) {
      block.result = it
    } else {
      // 未配对结果（调用缺失 / 旧条目无 id / 重复结果）：自成一块。
      out.push({
        type: 'tool_call',
        invocation: it,
        result: null,
        position: it.position,
      })
    }
  }
  return out
}

/**
 * （fold-tool-calls-into-process-tree 4.2，纯函数）合并块的执行结果章：
 * 已配对 → 结果条目定章（显式拒绝记号 = 已拒绝，否则已执行）；未配对但
 * 调用条目自带已决标记（截断转录里的孤立 ✓/✗）→ 从调用条目定章；调用
 * 仍在跑 / 未配对调用 → null（无章，调用态标题）。
 */
export function toolCallBlockOutcome(block: ToolCallBlock): 'ok' | 'denied' | null {
  const shape = (it: ProcessItem): { element_type?: string; content: string; title?: string | null } => ({
    element_type: it.elementType,
    content: it.content,
    title: it.title,
  })
  if (block.result) {
    return toolResultDenied(block.result.content, block.result.title) ? 'denied' : 'ok'
  }
  if (isDecidedToolResult(shape(block.invocation))) {
    return toolResultDenied(block.invocation.content, block.invocation.title) ? 'denied' : 'ok'
  }
  return null
}

/**
 * （fold-tool-calls-into-process-tree 4.2，纯函数）合并块的拒绝态：
 * 结果条目或调用条目任一携带显式拒绝记号即整块挂 ✗（被拒调用的收起
 * 行与 data-denied 数据源）。
 */
export function toolCallBlockDenied(block: ToolCallBlock): boolean {
  return (
    (block.result !== null && toolResultDenied(block.result.content, block.result.title)) ||
    toolResultDenied(block.invocation.content, block.invocation.title)
  )
}

/**
 * （fix-webui-qa-round8 7.1）相邻思考增量的聚合（纯函数）：过程折叠内的
 * **相邻** thinking 条目合并为一个连续段落（内容顺序拼接、position 取首条
 * ——折叠体的 DOM 身份不变）。逐帧落账的 thinking 增量展开后呈现为少量
 * 连续段落，不再是一屏碎片行；与工具条目相邻的 thinking 照旧分段。
 */
export function mergeAdjacentThinking(items: ProcessItem[]): ProcessItem[] {
  const out: ProcessItem[] = []
  for (const it of items) {
    const last = out[out.length - 1]
    if (it.elementType === 'thinking' && last?.elementType === 'thinking') {
      last.content += it.content
      continue
    }
    out.push({ ...it })
  }
  return out
}

/**
 * Group the ordered entry sequence into turn units (D3): each prompt opens
 * an operator turn; the entries until the next prompt belong to the
 * following agent turn; error entries render as standalone counted bubbles
 * and split the surrounding agent turns (D5 — they never join a run);
 * notice entries render as standalone neutral info bars and likewise split
 * the surrounding agent turns (close-acceptance-blind-spots 4.2).
 * Empty-content entries are skipped (they carry nothing to display).
 */
export function groupConversation(entries: ErrorCountedView[]): TurnUnit[] {
  const units: TurnUnit[] = []
  // 当前 agent 回合积攒的条目：error/notice/prompt 都会终结它——前两者
  // 独立成单元（错误气泡 D5 / 中性提示条），后者是操作者回合。收尾时
  // 一次性切 run（D1）。
  let pending: ConversationEntryView[] = []
  const flush = (): void => {
    if (pending.length === 0) return
    const first = pending[0]
    units.push({
      kind: 'agent',
      runs: foldCrossRunToolResults(splitAgentRuns(pending)),
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
    if (e.element_type === 'notice') {
      flush()
      units.push({ kind: 'notice', entry: e })
      continue
    }
    // （fix-webui-qa-findings D2）模式切换契约条目独立成单元（一等渲染）。
    if (e.element_type === 'permission_mode_result') {
      flush()
      units.push({ kind: 'mode_result', entry: e })
      continue
    }
    // （fix-webui-qa-round8 1.2 / 5.2）升级降级与模型切换的一等契约条目：
    // 各自独立成单元，绝不卷进相邻正文的文本 run（围栏/正文互不污染）。
    if (e.element_type === 'escalate_downgrade') {
      flush()
      units.push({ kind: 'escalate_downgrade', entry: e })
      continue
    }
    if (e.element_type === 'model_change') {
      flush()
      units.push({ kind: 'model_change', entry: e })
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

/**
 * 回合观察模型时间线（fix-webui-qa-round10 2.4，B-DEF-02，纯函数）：按单元
 * 序重放 `model_change` 留痕（`{from, to}`），得到每个 agent 回合**开始时刻**
 * 的生效模型。徽章只读这个观察值——绝不从会话当前模型回填，切换模型不改写
 * 历史回合的徽章（spec「Turn model badge is frozen at observation time」）。
 *
 * 观察不到就如实缺省：第一条 model_change 之前的回合（含整场零切换的会话）
 * 无观察值 → 空串（徽章整枚不渲染，不伪造）。model_change 单元自身与
 * 操作者/错误/提示单元不产生映射；mid-turn 切换会终结当前 agent 单元
 * （groupConversation 对 model_change flush），切换后的条目自成新单元、
 * 取新模型——与「切换从下一回合生效」的语义一致。
 */
export function turnObservedModels(units: TurnUnit[]): Map<number, string> {
  const out = new Map<number, string>()
  let effective: string | null = null
  for (const u of units) {
    if (u.kind === 'model_change') {
      effective = parseModelChangePayload(u.entry.content).to
      continue
    }
    if (u.kind === 'agent') out.set(u.position, effective ?? '')
  }
  return out
}

/** The newest entry timestamp of a turn — the turn's seam edge (D5). */
export function unitMaxTs(unit: TurnUnit): number {
  if (
    unit.kind === 'operator' ||
    unit.kind === 'error' ||
    unit.kind === 'notice' ||
    unit.kind === 'mode_result' ||
    unit.kind === 'escalate_downgrade' ||
    unit.kind === 'model_change'
  ) {
    return unit.entry.created_at_unix || 0
  }
  return unit.maxTs
}

/**
 * One turn's visible-segment contribution（2.3，与后端 `count_chat_messages`
 * 同口径）：agent 回合里每个 text run（相邻 markdown 合并）计 1；error 气泡
 * 按合并计数计（每条 error 后端各计 1）；operator 提交、process 条目与
 * notice 提示条不计（notice 不是回复段——零输出回合不虚增未读段数，
 * close-acceptance-blind-spots 4.2）。累加即得「读到此回合为止已见的段数」，
 * seam 与徽标共用同一份段锚。
 */
export function unitSegmentCount(unit: TurnUnit): number {
  if (
    unit.kind === 'operator' ||
    unit.kind === 'notice' ||
    unit.kind === 'mode_result' ||
    unit.kind === 'escalate_downgrade' ||
    unit.kind === 'model_change'
  )
    return 0
  if (unit.kind === 'error') return unit.entry.count ?? 1
  return unit.runs.filter((r) => r.type === 'text').length
}

/**
 * （fix-webui-qa-round8 7.2）未闭合代码围栏的条目级收口（纯函数）：条目内容
 * 里的 ``` 围栏标记逐行翻转开合，落账时仍处开态（计数为奇）就在内容末尾补
 * 一个闭合围栏——markdown 渲染的围栏作用域被限制在**单条目内**，相邻条目
 * 的正文不再被卷进上一条的未闭合代码块。已配平的内容原样返回。
 */
export function closeUnclosedFence(content: string): string {
  let open = false
  for (const line of content.split('\n')) {
    if (/^\s*```/.test(line)) open = !open
  }
  return open ? content + '\n```' : content
}

/**
 * （fix-webui-qa-findings D2）模式契约条目的载荷解析（纯函数）：content 是
 * `{request_id, ok, mode, detail}` JSON；解析失败按未知处理（mode 原文展示、
 * ok=false），绝不因坏载荷抛错炸掉整棵转录。
 */
export function parseModeResultPayload(content: string): {
  ok: boolean
  mode: string
  detail: string
} {
  try {
    const v = JSON.parse(content) as Record<string, unknown>
    return {
      ok: v['ok'] !== false,
      mode: typeof v['mode'] === 'string' ? v['mode'] : 'unknown',
      detail: typeof v['detail'] === 'string' ? v['detail'] : '',
    }
  } catch {
    return { ok: false, mode: 'unknown', detail: content }
  }
}

/**
 * （fix-webui-qa-round8 1.2）升级降级条目的载荷解析（纯函数）：content 是
 * `{request_id, tool, reason, detail}` JSON；解析失败按空载荷处理（detail
 * 回退原文本），绝不因坏载荷抛错炸掉整棵转录。
 */
export function parseEscalateDowngradePayload(content: string): {
  request_id: string
  tool: string
  reason: string
  detail: string
} {
  try {
    const v = JSON.parse(content) as Record<string, unknown>
    return {
      request_id: typeof v['request_id'] === 'string' ? v['request_id'] : '',
      tool: typeof v['tool'] === 'string' ? v['tool'] : '',
      reason: typeof v['reason'] === 'string' ? v['reason'] : '',
      detail: typeof v['detail'] === 'string' ? v['detail'] : '',
    }
  } catch {
    return { request_id: '', tool: '', reason: '', detail: content }
  }
}

/**
 * （fix-webui-qa-round8 5.2）模型切换条目的载荷解析（纯函数）：content 是
 * `{from, to}` JSON（from 可为 null = 切换前无已知模型）；解析失败按未知
 * 处理，绝不因坏载荷抛错。
 */
export function parseModelChangePayload(content: string): {
  from: string | null
  to: string
} {
  try {
    const v = JSON.parse(content) as Record<string, unknown>
    return {
      from: typeof v['from'] === 'string' ? v['from'] : null,
      to: typeof v['to'] === 'string' ? v['to'] : 'unknown',
    }
  } catch {
    return { from: null, to: content }
  }
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
      return '启动失败'
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
 * （fix-webui-qa-round3 D1）process run 的成员构成词（纯函数）：thinking 独占
 * = thinking、tool 独占 = tool、混合 = mixed。折叠收起行据此挑 glyph 与
 * data-kind——thinking 回合的折叠在缺省形态即标明 thinking（spec「thinking
 * fold is titled and distinguishable」），操作者不展开就能认出「此处是
 * thinking」，而不是一条与正文无异的裸文本。
 */
export function processRunKind(run: ProcessRun): 'thinking' | 'tool' | 'mixed' {
  const has = { thinking: false, tool: false }
  for (const it of run.items) {
    if (it.elementType === 'thinking') has.thinking = true
    else has.tool = true
  }
  if (has.thinking && has.tool) return 'mixed'
  return has.thinking ? 'thinking' : 'tool'
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
 * （fix-webui-qa-findings M1）工具 run 的顶层执行结果（纯函数）：
 * `ok` = 已执行（末条目带完成标记）、`denied` = 被拒绝（显式拒绝记号）、
 * `null` = 仍在跑（无完成标记）。折叠 summary 行据此直接呈现
 * 「已执行/已拒绝」章——决策结果不再只藏在两层折叠之内。
 */
export function processRunOutcome(run: ProcessRun): 'ok' | 'denied' | null {
  const last = run.items[run.items.length - 1]
  if (!last) return null
  if (toolResultDenied(last.content, last.title)) return 'denied'
  if (typeof last.title === 'string' && /^[\u2713\u2717]/.test(last.title.trim())) return 'ok'
  return null
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
 * （fix-webui-qa-defects-round4 2.2，design D1）同一事实的 entries 级同构
 * 判定：最新转录条目仍是操作员提交（`kind === 'prompt'`）即处于「已收到」
 * 接收回执相位。composer 的停止控件供数走这里——dashboard 手里是 detail
 * 的 entries（转录流推送即时到达），无需先经 transcript-view 的分组管线；
 * 与 [`awaitingReceipt`] 是同一事实的两张皮（分组前后），漂移由各自单测
 * 钉住。
 *
 * （fix-webui-qa-round2 1.2，D-B215 防御半边）空内容 prompt 条目不是提交
 * ——它没有可等待回复的文本（引擎侧激活拉起曾注入空 prompt 条目，令本
 * 判定恒真、composer 卡死在停止态）。content 字段缺席（旧调用方形状）按
 * 非空对待，不改变既有语义。
 */
export function entriesAwaitReceipt(
  entries: readonly { kind: string; content?: string }[],
): boolean {
  if (entries.length === 0) return false
  const last = entries[entries.length - 1]!
  if (last.kind !== 'prompt') return false
  return last.content === undefined || last.content.length > 0
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
   * （fix-webui-qa-round9 4.3 引入；fix-webui-qa-round10 2.4 退役为徽章
   * 真源）会话当前模型（dashboard 从 detail/summary 的 `current_model` 下传）。
   * 徽章改读回合观察时间线（`turnObservedModels`，model_change 留痕重放）后
   * 本属性不再参与回合徽章渲染——会话当前模型变化不得改写历史回合的徽章
   * （B-DEF-02）；保留属性仅为 dashboard 传参面兼容。
   */
  @property({ attribute: false }) currentModel: string | null = null
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
  @state() private seamIndex: number | null = 0  /**
   * （fix-webui-qa-round2 2.1/2.6，D5+D-R2A）开卷未读边界：聚焦进入会话时
   * 捕获的「推进前读锚」（null = 无未读边界，不画线）。聚焦写锚此后推进到
   * 服务端当前计数（spec「Focus anchors at the server's current count」），
   * 分界线呈现改由这份冻结边界驱动——锚到顶与分界线可见从此互不牵制。
   * 边界的清账：滚读到底、mark all seen、聚焦中看着到达（sticky 流式）——
   * 三条都是「边界以下内容已被看过」的可达路径。
   */
  @state() private seamBoundary: number | null = null
  /**
   * 渲染管线输入：错误合并 → 回合分组（seam/滚动/渲染都以此为准，
   * 分组后索引与回合一一对应）。
   */
  @state() private turnUnits: TurnUnit[] = []
  /**
   * （fix-webui-qa-round10 2.4，B-DEF-02）回合开始时刻的观察模型（agent 回
   * 单元 position → 模型名，空串 = 无观察）：徽章唯一真源，随 rebuildUnits
   * 同步重算。会话当前模型（currentModel）不再参与。
   */
  @state() private observedModels = new Map<number, string>()

  /**
   * （fix-webui-qa-round8 3.1）「跳到最新」浮标的可见态（纯派生）：未处于
   * 贴底跟随（sticky=false）且视口下方还有内容（新条目/未读尾段）时出现；
   * 点击回底并恢复跟随。几何判定见 {@link refreshJumpPill}。
   */
  @state() private jumpVisible = false

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
      /* （fix-webui-qa-round8 3.1）浮标的定位上下文。 */
      position: relative;
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
      /* （fix-webui-qa-round10 2.2，C-DEF-03）对话面永不横向滚动：overflow-y
         的 auto 会把缺省的 overflow-x: visible 计算成 auto——超宽内容的
         scrollWidth 曾把容器撑出数千 px 的隐藏横向溢出（QA 实测 5350px）。
         横向滚动收敛到条目内层滚动区（table/pre 各自 overflow-x:auto），
         容器宽度钉在布局宽。 */
      overflow-x: hidden;
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
    /* （fix-webui-qa-round8 3.1）「跳到最新」浮标：sticky=false 且视口下方
       有新内容时出现在滚动容器底部居中；点击回底并恢复跟随。 */
    .jump-latest {
      position: absolute;
      bottom: 14px;
      left: 50%;
      transform: translateX(-50%);
      z-index: 5;
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 4px 12px;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-full);
      background: var(--sebas-surface);
      color: var(--sebas-text-dim);
      font: inherit;
      font-size: 0.74rem;
      cursor: pointer;
      box-shadow: var(--sebas-shadow-1, 0 2px 8px rgba(0, 0, 0, 0.18));
      transition:
        color var(--sebas-dur) var(--sebas-ease),
        border-color var(--sebas-dur) var(--sebas-ease);
    }
    .jump-latest:hover {
      color: var(--sebas-accent);
      border-color: var(--sebas-accent-border, var(--sebas-accent));
    }
    .jump-latest:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .jump-latest[hidden] {
      display: none;
    }
    /* ── 自然对话流 ──
       26px 头像圆保留（assistant = accent 渐变底，user = accent-soft 底）。
       内容侧去卡片（D4）：agent 回合不再有气泡壳——.flow 裸排（作者标签
       小字行 + 正文段/过程折叠按 run 序直接铺）；用户侧 .msg-block 收敛为
       轻底色块（tinted 背景、无边框阴影）。最宽 min(680px, 100% - 60px)；
       时间戳在 meta 行（作者名 weight 600 淡色 + 时间紧随其后、
       tabular-nums；round7 4.3 起两侧统一为「紧随作者名」约定，不再右对齐）。
       错误气泡保留计数卡片形态（D5）。 */
    .turn-block {
      display: flex;
      gap: 10px;
      align-items: flex-start;
      max-width: 100%;
      /* （fix-webui-qa-round10 2.2）行级收缩守卫：回合块自身的最小宽度不参与
         内容固有宽度（超宽内容的 min-content 不得沿 flex 链上传撑破容器）。 */
      min-width: 0;
    }
    .turn-block.is-user {
      flex-direction: row-reverse;
    }
    .turn-block .flow,
    .turn-block .msg-block {
      flex: 1;
      min-width: 0;
      max-width: min(680px, calc(100% - 60px));
      /* （fix-webui-qa-round10 2.3，C-DEF-01/C-DEF-03）行内尺寸包含：内容
         固有宽度（宽表/长代码行）不再参与 .flow 的宽度计算——对话面宽度
         钉死在布局宽，超宽内容只能在条目内层滚动区横向滚动。这也是直播态
         空白绘制（QA 三次复现：DOM/aria 完好、console 零错误、reload 恢复）
         的对因修复——超宽内容撑出的数千 px 宽图层是合成层失效的病灶，
         宽度钉死后该图层不再存在。 */
      contain: inline-size;
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
    /* close-acceptance-blind-spots 4.2（design D3）：notice 中性信息条——
       surface-2 底 + 普通边框 + dim 文字，无失败语义色。颜色全走语义
       token，明暗两主题随 tokens.css 同源翻转（dark: 深灰蓝面；light:
       浅灰面），无需按主题各自调色。 */
    .turn-block .avatar.notice {
      background: var(--sebas-surface-2, #f0f2f7);
      color: var(--sebas-text-dim, #5f6a80);
      border-color: var(--sebas-border);
      font-weight: 700;
    }
    /* （fix-webui-qa-findings D2）mode 契约条目的模式词与失败态：
       模式词挂 mono 章（与头部 mode-tag 同视觉语言）；失败挂失败色。 */
    .turn-block .body .mode-mode {
      font-family: var(--sebas-font-mono);
      font-size: 0.78rem;
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-full);
      padding: 1px 8px;
      white-space: nowrap;
    }
    .turn-block .body .mode-failed {
      color: var(--sebas-status-failed, #b91c1c);
    }
    .turn-block .bubble.notice {
      background: var(--sebas-surface-2, #f0f2f7);
      border-color: var(--sebas-border);
      border-top-left-radius: 4px;
    }
    .turn-block .meta .author.notice {
      color: var(--sebas-text-dim);
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
    /* （fix-webui-qa-round7 1.2，D1）作者名收缩守卫：meta 行是 flex 弹性行，
       兄弟 .time/.receipt 均 nowrap；无 min-width:0 时长作者名把时间戳/回执
       芯片推出视口。守卫后作者名省略号收干，时间戳保持原位。 */
    .turn-block .meta .author {
      font-weight: 600;
      color: var(--sebas-text-dim);
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .turn-block .meta .author.you {
      color: var(--sebas-accent);
    }
    /* （fix-webui-qa-round9 4.3）回合块的生效模型芯片：低调 pill（faint 底
       + 淡字），与会话头模型标识同源同帧；模型不可知时整枚不渲染。 */
    .turn-block .meta .model-chip {
      flex: none;
      display: inline-flex;
      align-items: center;
      max-width: 16rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      padding: 0 var(--sebas-space-2);
      border-radius: 999px;
      background: var(--sebas-surface-raised, rgba(127, 127, 127, 0.14));
      color: var(--sebas-text-faint);
      font-size: 0.66rem;
      line-height: 1.5;
    }
    .turn-block .meta .time {
      /* （fix-webui-qa-round7 4.3）时间戳统一约定：紧随作者名（与回执芯片
         同行内联），两侧（用户气泡 / agent 行）一致。旧的「推到整行右缘」
         对齐把 agent 时间戳悬在半空、远离作者名，与用户气泡内位置不一致
         （QA round7 实锤）；该对齐写法不得回归。 */
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
      /* （fix-webui-qa-round11 4.1，B-4）pre-wrap：多行提交在气泡里保留
         换行——wire 往返本就带换行符，markdown 管线（breaks:false）把它们
         原样留在 p 元素内，此前容器按 normal 折叠成空格（三行变一行）。
         恢复后的转录走同一容器，同规则生效。子元素不受扰：pre 有 UA
         默认 white-space:pre（代码块语义不回退），块级标签之间的换行不
         产生可见空行；长 token 折行仍由下方 overflow-wrap:anywhere 承担
         （pre-wrap 与 anywhere 正交，spec「long single-token lines keep
         the existing overflow handling」）。 */
      white-space: pre-wrap;
      /* （fix-webui-qa-round7 1.2，D1）真折行：anywhere 参与 min-content
         收缩——无空格超长 token 不再撑破主面板。旧选择器只盖七种标签且用
         不参与固有宽度计算的 break-word，2100+ 字符连续 token 把滚动容器
         撑到万级 px。overflow-wrap 可继承，子元素（含 markdown 管线产出）
         一并生效；pre 保持 white-space:pre + overflow-x 滚动，语义不回退。 */
      overflow-wrap: anywhere;
    }
    /* 回合内多段文本（工具组切开的两段论述）：段间留一行呼吸，视觉上
       明确「中间发生过事」（design Risks：分段规则可读）。 */
    .turn-block .body + .body,
    .turn-block .body + .process-fold,
    .turn-block .process-fold + .body,
    .turn-block .process-fold + .process-fold {
      margin-top: var(--sebas-space-2);
    }
    /* （fix-webui-qa-round7 1.2）折行已上提至 .body（anywhere 可继承）——
       本规则退役；留位注释防回归者把 break-word 加回子标签。 */
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
    /* （fix-webui-qa-round7 1.2，D1）GFM 表格块级横向滚动：宽于容器的表格
       以自身滚动条呈现，转录容器本体不产生横向溢出（不撑破父容器）。 */
    .turn-block .body table {
      display: block;
      max-width: 100%;
      overflow-x: auto;
      /* 3c：.body 的 anywhere 会继承进单元格，把文字表格压到一字符宽，
         横滚永不触发——表格内恢复 normal，min-content 超宽时走上横滚。 */
      overflow-wrap: normal;
    }
    /* （fix-webui-qa-round10 2.2，C-DEF-03）条目内层滚动区的**可见滚动条**：
       表格/代码块横滚可用但滚动条不可见、右缘硬裁切无可供发现提示（QA
       实测）。滚动条样式在明暗主题下都取边框色阶 token——Firefox 走
       scrollbar-color/width，Chromium 系走 ::-webkit-scrollbar。 */
    .turn-block .body pre,
    .turn-block .body table,
    .view-all-body table {
      scrollbar-width: thin;
      scrollbar-color: var(--sebas-border-strong, rgba(127, 127, 127, 0.55)) transparent;
    }
    .turn-block .body pre::-webkit-scrollbar,
    .turn-block .body table::-webkit-scrollbar,
    .view-all-body table::-webkit-scrollbar {
      height: 8px;
      width: 8px;
    }
    .turn-block .body pre::-webkit-scrollbar-thumb,
    .turn-block .body table::-webkit-scrollbar-thumb,
    .view-all-body table::-webkit-scrollbar-thumb {
      background: var(--sebas-border-strong, rgba(127, 127, 127, 0.55));
      border-radius: 999px;
    }
    .turn-block .body pre::-webkit-scrollbar-track,
    .turn-block .body table::-webkit-scrollbar-track,
    .view-all-body table::-webkit-scrollbar-track {
      background: transparent;
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
       （foldOpen Map），显隐即条件渲染。
       （fix-webui-qa-round3 D1）收起行加一层 surface-2 底的**小胶囊**：
       仍是行内轻控件（无边框、无阴影、不换行成块），但与正文段落形态可
       区分——操作者无需展开即可从折叠行看出「此处有 thinking/工具过程」
       （spec「过程折叠的缺省呈现 SHALL 与正文可区分」；与 4.4「不得是
       大按钮/边框块/卡chrome」并存：胶囊无 border、无 box-shadow）。 */
    .turn-block .process-fold {
      margin: var(--sebas-space-2) 0;
      min-width: 0;
    }
    .turn-block .fold-link {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      max-width: 100%;
      padding: 1px 8px;
      background: var(--sebas-surface-2);
      border: none;
      border-radius: var(--sebas-radius-full);
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
    /* （fix-webui-qa-findings M1）决策结果的顶层章：已执行/已拒绝直接
       挂在折叠 summary 行上，不需要展开任何一层。 */
    .turn-block .fold-link .outcome {
      font-size: 0.68rem;
      font-weight: 600;
      border-radius: var(--sebas-radius-full);
      padding: 0 7px;
      white-space: nowrap;
    }
    .turn-block .fold-link .outcome-ok {
      color: var(--sebas-status-done, #15803d);
      background: var(--sebas-surface-2);
    }
    .turn-block .fold-link .outcome-denied {
      color: var(--sebas-status-failed, #b91c1c);
      background: var(--sebas-status-failed-bg, #fee2e2);
    }
    /* （fold-tool-calls-into-process-tree 4.2）工具调用合并块：过程折叠
       的子折叠——轻分区（虚线左边界 + 常驻决策章），默认收起，开合独立
       托管（原 D-C3a 的顶层默认展开块退役）。被拒块的拒绝色落在拒绝章
       （outcome-denied）上。 */
    .turn-block .tool-result {
      margin: var(--sebas-space-2) 0 0;
      padding-left: var(--sebas-space-3);
      border-left: 2px dashed var(--sebas-border);
      min-width: 0;
    }
    .turn-block .tool-result[data-denied='true'] > .fold-link {
      color: var(--sebas-status-failed, #b91c1c);
    }
    /* 合并块展开体：参数段在上、结果段在下，两段间虚线分隔（D4）。 */
    .turn-block .tool-call-body .call-result {
      margin-top: var(--sebas-space-2);
      padding-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
    }
    /* 展开内容：work-block-body 同款（0.82rem/1.6 + 虚线顶边）。挂 .body
       复用 markdown 排版规则（后写的字号覆盖之）。 */
    .turn-block .fold-body {
      padding: 8px 12px 12px;
      font-size: 0.82rem;
      line-height: 1.6;
      margin-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
      /* （fix-webui-qa-round13 1.1，B-2）展开态留白收敛：.body 的 pre-wrap
         （round11 4.1，为多行提交保留换行而设）经类名与继承进入展开面板，
         模板缩进换行与 markdown 块间换行在 pre-wrap 下逐个变成可见空行——
         「💭 thinking」标签与内容之间被撑出大段空带（QA b48/a54 两轮采样
         稳定复现）。展开体内恢复 normal，空白排版交还给块级元素的 margin；
         pre 代码块语义不回退（UA 默认 white-space:pre 直接命中 pre 元素，
         继承值不参与），长内容的 pre/table 横滚（overflow-x）不受影响。
         折叠态不经过本分支，形态不变。 */
      white-space: normal;
    }
    /* 内层条目体同样挂 .body（直接命中 pre-wrap），继承压不过直接规则——
       一并恢复 normal，思维内容自身携带的原始换行交回 markdown 段落语义。 */
    .turn-block .fold-body .body {
      white-space: normal;
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
       解析），定稿后换 markdown 渲染。（fix-webui-qa-round7 1.2）折行
       与 .body 同级 anywhere——流式里的无空格长 token 同样不撑破面板。 */
    .turn-block .body.text-live {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
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
      /* （fix-webui-qa-round7 1.2）弹层与转录同一排版约束：长 token 折行、
         表格块级横向滚动——「查看全部」不得成为布局破坏的旁路。 */
      overflow-wrap: anywhere;
    }
    .view-all-body table {
      display: block;
      max-width: 100%;
      overflow-x: auto;
      overflow-wrap: normal;
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
    this.cancelStreamDrain()
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
      this.streamQueue.length = 0
      this.cancelStreamDrain()
      // （fix-webui-qa-round2 2.1/2.6）开卷边界：优先消费 rail switch 在写
      // 锚前登记的边界（跨实例存活）；深链等未经 rail 的聚焦回退用当前
      // 读锚（此刻还没人推进过它）。turnLive 开卷（聚焦撞上在飞回合）不
      // 画线——在飞内容正被看着，旧边界没有呈现意义。placeholder 首交换
      // 规则不变：空会话边界即全读，无线可画。
      const armed = peekOpeningSeam(this.sessionKey)
      const pre = armed !== undefined ? armed : readAnchorCount(this.sessionKey)
      this.seamBoundary = this.turnLive ? null : pre
    }
    if (changed.has('entries') || changed.has('sessionKey')) {
      // 快照收敛：position 已被属性覆盖的流式条目裁掉，只留快照还没
      // 追上的尾巴（乱序竞速下绝不回退视图）。
      const snapMax = this.entries.reduce((m, e) => Math.max(m, e.position), 0)
      this.streamEntries = this.streamEntries.filter((e) => e.position > snapMax)
      this.rebuildUnits()
      this.recomputeSeam()
      // （round3 6.1）快照路径与 turn.append 路径同一贴底语义：秒回场景 /
      // 重拉收敛下新段经 entries 属性到达（不经 turn.append），此前只有
      // 滚动事件能触发标记（短对话不溢出 = 滚动事件永不来——首个交换的
      // 锚推进依赖巧合）。聚焦 + 可见 + 贴底时随快照增长推进共享锚（与
      // onTurnAppend 同一 guard），seam 与 rail 徽标不再驻留。后台 tab /
      // 用户上滚（sticky=false）不推进。**仅在会话未切换的更新里推进**：
      // 打开既有未读会话的首帧快照（sessionKey 同帧变更）是「开门」不是
      // 「看着到达」，锚原地不动、seam 照旧呈现（3.1 契约）。组件首次
      // 挂载同理（hasUpdated=false：首更 changed 不带未显式设置的属性，
      // 是装载不是到达）。
      //
      // （fix-webui-qa-round8 5.1）开卷推进不再在这里无条件写锚——已见
      // 驱动的开卷结算统一在 {@link settleOpeningAnchor}（渲染几何落定后，
      // 以「全部已读 / 内容全部在可视高度内」为前提）执行。
      if (
        this.hasUpdated &&
        !changed.has('sessionKey') &&
        this.sticky &&
        this.docVisible()
      ) {
        this.scheduleMarkSeen()
      }
      // （3.1）空流建立锚点：占位会话从 0 回合转出首回合、且操作员聚焦、
      // 文档可见、贴底时，这是「亲眼看着到达」的首交换——锚从空流状态建立，
      // 不画 seam、不闪徽章（session-unread-badge「first focused exchange of a
      // fresh placeholder」）。秒回场景下回复经快照而非 turn.append 到达，
      // 且 dashboard 可能连同 sessionKey 一起重渲染（组件实例被重建），
      // 所以判据用模块级「该会话曾以空流渲染过」而非实例内的回合数差值。
      // 打开既有会话不在此列：那种会话从没以空流出现过，seam 必须保留。
      this.settleEmptyStreamAnchor()
    }
    // （2.6，D-R2A）turnLive 竞速补写：末帧经 turn.append 收敛后 entries 可
    // 能不再变化，定稿（turnLive 翻 false）若不触发推进，锚就停在「扣除在
    // 飞尾段」的旧值——聚焦路径的确定性红。定稿瞬间补一次贴底推进。
    if (changed.has('turnLive') && this.hasUpdated && !this.turnLive) {
      if (this.sticky && this.docVisible()) this.scheduleMarkSeen()
    }
  }

  /** 渲染管线输入：快照条目 + 流式尾巴 → 错误合并 → 回合分组。 */
  private rebuildUnits(): void {
    const merged = [...this.entries, ...this.streamEntries]
    this.turnUnits = groupConversation(mergeSpawnErrors(merged))
    // （fix-webui-qa-round10 2.4，B-DEF-02）回合观察模型时间线与回合分组同
    // 源重算：徽章只读它，绝不读会话当前模型。
    this.observedModels = turnObservedModels(this.turnUnits)
  }

  /**
   * turn.append 到达（workbench-live-conversation-flow 2.2 / 6.1）：position
   * 去重后并入渲染管线。聚焦且贴底（sticky）时随渲染推进读锚——角标不闪、
   * 已读缝不出现；未贴底不写锚，照常计未读（session-unread-badge 语义）。
   * 思考/工具条目不进段数锚增量（与 msg_count 只数可见回复段的口径一致）。
   *
   * （fix-webui-qa-round2 2.5，D-B13）大批量路径分片摄入：单帧携带 >50 条
   * （flood 的 1200 chunks）先入队，逐 rAF 片段（每片 ≤50 条）重渲——
   * 一次性全量 append 曾造成 ~1.2s 主线程冻结。小批量（含 drip 首包）照旧
   * 直渲染，首包即时性不变。
   *
   * （fix-webui-qa-round2 2.1）聚焦 + 贴底的到达是「看着到达」：开卷边界
   * 之下没有未读内容了，冻结边界就地清账（seam 不再对看着到达的内容亮出）。
   */
  private onTurnAppend(sessionId: string, incoming: ConversationEntryView[]): void {
    if (sessionId !== this.sessionKey || incoming.length === 0) return
    const maxKnown = Math.max(
      this.entries.reduce((m, e) => Math.max(m, e.position), 0),
      this.streamEntries.reduce((m, e) => Math.max(m, e.position), 0),
    )
    const fresh = incoming.filter((e) => e.position > maxKnown)
    if (fresh.length === 0) return
    const watched = this.sticky && this.docVisible()
    if (watched && this.seamBoundary !== null) {
      // 看着到达：边界以下全部已见，分界线清账（登记一并作废）。
      this.seamBoundary = null
      clearOpeningSeam(this.sessionKey)
      this.seamIndex = null
      this.unseenCount = 0
    }
    if (fresh.length <= FLOOD_CHUNK) {
      this.streamEntries.push(...fresh)
      this.rebuildUnits()
      this.recomputeSeam()
    } else {
      this.streamQueue.push(...fresh)
      this.scheduleStreamDrain()
    }
    // （3.1）空流登记的在册会话：首交换的流式到达在此**同步**结算（无
    // 250ms 防抖窗——「never flashes the badge or the seam」）。未登记/
    // 已消费即 no-op，防抖路径照旧兜底非首交换的到达。
    this.settleEmptyStreamAnchor()
    // （3.1/3.2）聚焦 + 文档可见 + 贴底：到达即推进共享游标（亲眼看着到的
    // 内容不再挂未读）；文档隐藏（后台 tab）时不推进——回来看 seam/徽章。
    if (watched) {
      // 贴底读流：段锚随渲染推进（写的是与徽标同一个 anchor_count 字段，
      // 2.3）——本地已渲染段数已含流式尾巴，无需另设增量补丁。
      this.scheduleMarkSeen()
    }
  }

  // ---- 大批量分片摄入（fix-webui-qa-round2 2.5，D-B13）--------------------

  /** 待摄入的流式条目队列（>FLOOD_CHUNK 的到达先排队，逐片入渲）。 */
  private streamQueue: ConversationEntryView[] = []
  /** 下一片的 rAF 句柄；null = 无排片。 */
  private streamDrainHandle: number | null = null

  private scheduleStreamDrain(): void {
    if (this.streamDrainHandle !== null) return
    this.streamDrainHandle = requestAnimationFrame(() => {
      this.streamDrainHandle = null
      const batch = this.streamQueue.splice(0, FLOOD_CHUNK)
      if (batch.length === 0) return
      this.streamEntries.push(...batch)
      this.rebuildUnits()
      this.recomputeSeam()
      if (this.streamQueue.length > 0) this.scheduleStreamDrain()
    })
  }

  private cancelStreamDrain(): void {
    if (this.streamDrainHandle !== null) {
      cancelAnimationFrame(this.streamDrainHandle)
      this.streamDrainHandle = null
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
   * 它的首个回合到达且操作员聚焦、可见、贴底时，就是「亲眼看着到达」
   * 的首交换——锚从空流状态建立，seam 与 rail 徽章都不该出现
   * （session-unread-badge「first focused exchange of a fresh placeholder」）。
   *
   * 登记与消费用模块级表而非实例字段：秒回场景下回复经快照到达，dashboard
   * 可能连同 sessionKey 一起重渲染（组件实例重建、实例内的回合数差值作废），
   * 模块表跨实例仍有效。
   *
   * 消费纪律（fix-unread-fresh-exchange）：登记只在**锚真的向前推进**时才
   * 烧掉。首渲染常常只有操作者提交（prompt-only，0 可见段、msg_count 仍
   * 0）——此刻无可推进水位，**保留登记**；回复随后到达时（快照增长或
   * turn.append，经 onTurnAppend 也会走到这里）仍按「看着到达」即时结算。
   * 此前先删后算的顺序把这个中间态挂载变成了登记的唯一消费者：写锚 no-op
   * 而登记已不在，首交换的回复一旦经新实例挂载（hasUpdated=false 跳过快照
   * 推进）到达，三条推进路径同时落空，徽章 + 缝永久驻留（GUI 两次复现）。
   * 特权作废的边界不变：文档隐藏 / 操作员上滚离开贴底 = 不再「看着到达」，
   * 登记即刻作废、锚原地不动（spec：隐藏期到达保持 unseen，回来看
   * seam/徽章；上滚后的到达按未读计）。
   */
  private settleEmptyStreamAnchor(): void {
    const key = this.sessionKey
    if (!key) return
    if (this.turnUnits.length === 0) {
      emptyStreamSessions.add(key)
      return
    }
    if (!emptyStreamSessions.has(key)) return
    if (!this.sticky || !this.docVisible()) {
      emptyStreamSessions.delete(key)
      return
    }
    // （2.3，D3）锚已统一为段计数：写锚 = max(服务端段数, 本地已渲染段数)，
    // 与 seam/rail 徽标同一条锚线。锚不落后（含 msg_count 先行于本地渲染的
    // 一拍）= 无可推进水位 → 保留登记等回复真正到达。（DD3）turnLive 时
    // 在飞尾段不计入候选——首交换流式中锚停在 0，登记保留，定稿后结算。
    const wouldAnchor = this.anchorCandidate()
    const prev = readAnchorCount(key)
    if (prev !== null && prev >= wouldAnchor) return
    if (prev === null && wouldAnchor === 0) return
    emptyStreamSessions.delete(key)
    this.writeSeen()
    // （fix-webui-qa-round2 2.1）首交换是「看着到达」：开卷边界（若有）
    // 一并清账——边界若停在 0，锚推进后 recompute 会把看着长出来的内容
    // 误标成未读（seam 闪现）。清账 + 同一更新周期内重算：seam 不闪现。
    this.dismissSeam()
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
   *
   * `excludeLiveTail`（fix-webui-qa-findings DD3）：turnLive 时把在飞尾段
   * 从累计中扣除——seam 与锚都不把「只看到了开头」的未定稿段读作已读；
   * 回合定稿（turnLive 翻 false，随新快照到达）后该段计入，锚随贴底
   * mark-seen 一次性推进。
   */
  /**
   * （fix-webui-qa-findings DD3）在飞尾段的存在判定：turnLive 且最后一个
   * agent 回合的末尾是 text run。该段还在增长，操作者只「看着开始」而非
   * 「看着完整到达」——未定稿不计已读。
   */
  private hasLiveTextTail(): boolean {
    if (!this.turnLive) return false
    const last = this.turnUnits[this.turnUnits.length - 1]
    if (!last || last.kind !== 'agent') return false
    const tail = last.runs[last.runs.length - 1]
    return tail?.type === 'text'
  }
  private segmentTotals(excludeLiveTail = false): number[] {
    const totals: number[] = []
    let acc = 0
    for (const unit of this.turnUnits) {
      acc += unitSegmentCount(unit)
      totals.push(acc)
    }
    if (excludeLiveTail && this.hasLiveTextTail() && totals.length > 0) {
      // 尾段是序列的最后一段贡献：只有最后一个回合的累计含它。
      for (let i = this.turnUnits.length - 1; i < totals.length; i++) totals[i] -= 1
    }
    return totals
  }

  /**
   * （fix-webui-qa-findings DD3）锚写入的候选水位：turnLive 期间，最后一个
   * agent 回合的**末尾在飞 text run**不计入——它还在增长，操作者只「看着
   * 开始」而非「看着完整到达」。排除它使锚停在已完成段的水位；回合定稿
   * （turnLive 翻 false，随新快照到达）时该段计入，锚随贴底 mark-seen 推进。
   * 若操作者中途切走，transcript 卸载、锚停在离开时的水位——定稿后
   * msg_count 超过锚 → rail 徽标如实出现（DD3：开始看着、其余未看的长流式
   * 回复必须计未读；此前锚经 `max(msgCount, local)` 被在飞段的首个 delta
   * 一次性抬到顶，整段永远读作已读）。
   *
   * turnLive 但末尾不是 text run（流式工具/思考中、或末单元是操作者提交）
   * 时无可排除的在飞正文段：按既有口径取 max(服务端段数, 本地段数)。
   */
  private anchorCandidate(): number {
    const totals = this.segmentTotals()
    const local = totals.length > 0 ? totals[totals.length - 1]! : 0
    if (this.hasLiveTextTail()) {
      // 在飞尾 text run 尚未定稿（DD3）：从本地段数中扣除它的贡献，且不取
      // msgCount（服务端计数已把该段的首个 delta 算作一段——max 会把
      // 「只看到开头」的段整体抬进已读，正是 DD3 的根因）。
      return Math.max(0, local - 1)
    }
    return this.msgCount != null ? Math.max(this.msgCount, local) : local
  }

  private writeSeen(): void {
    // 写入锚 = max(服务端段数, 本地已渲染段数)——流式期间快照未追上的
    // 可见段（6.1）已含在本地渲染里，rail 徽标与 seam 不闪现；单调 max
    // 由游标模块保证。（DD3）turnLive 期间在飞尾段不计入（见
    // anchorCandidate），定稿后随 mark-seen 一次性推进。
    writeCursor(this.sessionKey, this.anchorCandidate())
  }

  // ---- seam logic -------------------------------------------------------

  /**
   * Recompute `seamIndex` and `unseenCount` from the current turns and the
   * open boundary（D5，按回合计数）.
   *
   * （fix-webui-qa-round2 2.1/2.6）边界源换成开卷冻结边界
   * {@link seamBoundary}（换会话时从开卷登记/读锚捕获）：聚焦写锚随开卷
   * 推进到服务端当前计数后，实时锚不再是「未读」的判据——分界线对开卷时
   * 尚未看过的内容呈现一次，滚读/看着到达/mark-all-seen 清账。`null` 边界
   * = 无线可画（无锚开卷、已读开卷、看着到达清账后）。 turnLive 时在飞
   * 尾段不参与未读判定（DD3 既有口径不变）。
   */
  private recomputeSeam(): void {
    const boundary = this.seamBoundary
    if (boundary === null || this.turnUnits.length === 0) {
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    // seam 落在第一个「可见段累计超过边界」的回合上方——按回合计数、绝不
    // 切开回合（D5）。开卷后到达且被看着的内容不延长边界（看着到达清账）。
    const totals = this.segmentTotals(this.turnLive)
    const idx = totals.findIndex((total) => total > boundary)
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
    this.dismissSeam()
  }

  /**
   * 分界线清账（fix-webui-qa-round2 2.1）：滚读到底 / mark all seen /
   * 看着到达共用——开卷登记一并作废（同会话重挂载不再按旧边界重绘）。
   */
  private dismissSeam(): void {
    this.seamBoundary = null
    this.seamIndex = null
    this.unseenCount = 0
    if (this.sessionKey) clearOpeningSeam(this.sessionKey)
  }

  // ---- scroll handling --------------------------------------------------

  private onScroll(): void {
    const el = this.scrollEl
    if (!el) return
    // （review round2-3）开卷定位自身的滚动事件不是阅读动作：整事件忽略
    // （含 sticky 判定与 mark-seen 排定——详见 openingScrollUntil 注）。
    if (Date.now() < this.openingScrollUntil) return
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
    // （fix-webui-qa-round8 3.1）浮标随几何刷新（脱离贴底且有内容在下方）。
    this.refreshJumpPill()
  }

  /**
   * （fix-webui-qa-round8 3.1）浮标可见性的几何刷新（纯派生）：脱离贴底
   * （sticky=false）且视口下方还有内容（scroll 余量超阈值 = 新条目/未读
   * 尾段在场）→ 出现；贴底跟随或已到底 → 消失。
   */
  private refreshJumpPill(): void {
    const el = this.scrollEl
    if (!el) {
      this.jumpVisible = false
      return
    }
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight
    this.jumpVisible = !this.sticky && distance > NEAR_BOTTOM_PX
  }

  /**
   * （fix-webui-qa-round8 3.1）点击浮标：回底 + 恢复自动跟随 + 按贴底语义
   * 推进已读（seam 随 commitMarkSeen 清账）。此后新条目自动跟随。
   */
  private jumpToLatest = (): void => {
    const el = this.scrollEl
    this.sticky = true
    this.jumpVisible = false
    if (el) {
      const previous = el.style.scrollBehavior
      el.style.scrollBehavior = 'auto'
      el.scrollTop = el.scrollHeight
      el.style.scrollBehavior = previous
    }
    if (this.docVisible()) this.scheduleMarkSeen()
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
    // （DD3）候选水位经 anchorCandidate——turnLive 时在飞尾段不计入：
    // 流式中贴底读不推进锚到未定稿段，定稿后本方法把它一次性覆盖。
    const candidate = this.anchorCandidate()
    const anchor = this.readSeen() ?? 0
    if (this.turnUnits.length > 0 && candidate > anchor) {
      this.writeSeen()
    }
    // 贴底 = 边界以下内容已被读过：分界线清账（边界可能随 mark-seen 推进，
    // 也可能本就为 null——dismiss 幂等）。
    if (this.seamBoundary !== null) this.dismissSeam()
  }

  /**
   * Apply the scroll behaviour for the current frame（4.1，D5.1）: sticky
   * = 瞬时贴底（scrollHeight 提交，无 smooth 动画）。未读缝不再参与自动
   * 滚动定位——seam 只是标记与「mark all seen」出口，想读旧内容向上滚动
   * 即自然解除 sticky。
   */
  /**
   * 会话首帧布局的记忆键：换会话后（或组件重挂载后）的第一次滚动定位走
   * 「开卷」语义（见 applyAutoScroll），后续更新保持既有贴底跟随。
   */
  private lastScrollKey: string | null = null

  /**
   * （review round2-3）开卷定位（seam 居中）自身引发的滚动事件窗口：这是
   * 程序化定位，不是「读到贴底」。不设窗的话，未读尾段很短（末回合矮）时
   * 居中落点距底 ≤ NEAR_BOTTOM_PX，onScroll 会把 sticky 重新贴底并排定
   * mark-seen——commitMarkSeen 的清账不问阅读意图，刚画出的分界线在
   * 250ms 内自灭，「重聚焦必现 seam」随内容高度偶发失败（QA D-B12 的
   * 残留形态）。真实操作员的下一次滚动在窗口之外，语义不变。
   */
  private openingScrollUntil = 0

  private applyAutoScroll(): void {
    const el = this.scrollEl
    if (!el) return
    // （fix-webui-qa-round8 3.1）浮标随每次布局落定刷新（内容增长不触发
    // scroll 事件，几何刷新在这里兜住）。
    this.refreshJumpPill()
    // （fix-webui-qa-findings M3）开卷定位：打开一个带未读缝的会话时，
    // 首帧把 seam 滚进视野（而不是贴底）——操作员第一眼看到「上次读到哪里」。
    //
    // （fix-webui-qa-round8 3.1/5.1，design D4 的两半）跟随语义分两种开卷：
    // - **全读开卷**（seam 不在场）：贴底跟随照常——开卷的程序化定位不再
    //   无条件把 sticky 翻 false（round8 的误伤面：定位本身不是操作者上滚，
    //   定位后自动滚动整体停摆的根因已除）；
    // - **未读缝开卷**：定位停在中部、sticky 保持脱离——后续新条目不把
    //   操作者拽离 seam，「跳到最新」浮标承接一键回底 + 恢复跟随（spec
    //   scenario「浮标出现 → 点击 → 底部 + 此后自动跟随」）。跟随只在
    //   操作者真正到达底部（滚到底 / 点浮标）时重新咬合，且此后仅操作者
    //   主动上滚才脱离——「定位完成即恢复」的形态在浮标回底这一步兑现。
    // （5.1）已见驱动的开卷结算随后执行。
    if (this.lastScrollKey !== this.sessionKey) {
      this.lastScrollKey = this.sessionKey
      const seam = el.querySelector<HTMLElement>('.seam:not([hidden])')
      if (seam && typeof seam.scrollIntoView === 'function') {
        this.sticky = false
        this.openingScrollUntil = Date.now() + 150
        seam.scrollIntoView({ block: 'center', behavior: 'auto' })
      }
      this.settleOpeningAnchor(el)
      if (seam) return
    }
    if (!this.sticky) return
    const previous = el.style.scrollBehavior
    el.style.scrollBehavior = 'auto'
    el.scrollTop = el.scrollHeight
    el.style.scrollBehavior = previous
  }

  /**
   * （fix-webui-qa-round8 5.1）已见驱动的开卷锚结算（渲染几何落定后执行，
   * 唯一前提是「打开时内容已真正可见」）：
   * - 全读开卷（无未读边界）：锚顶到服务端当前计数——没有未读内容的开卷
   *   是「全部已读」，基线就此确立（后续后台到达照常计未读）；
   * - 有未读边界且**全部内容都在可视高度内**（滚动余量为 0，无需滚动即已
   *   看完）：视为已见，写锚并清账（spec「打开会话并看到新内容 THEN 清
   *   除」在零滚动会话上的形态）；
   * - 有未读边界且内容溢出可视高度：**不写锚**——打开不等于看完，徽标与
   *   未读缝保持，直到滚动贴底 / mark all seen / 「跳到最新」/ 看着到达。
   * 后台 tab 的装载不算「看着」；空流在册的占位会话仍归
   * settleEmptyStreamAnchor 专管。
   */
  private settleOpeningAnchor(el: HTMLElement): void {
    if (!this.sessionKey || !this.docVisible()) return
    if (emptyStreamSessions.has(this.sessionKey)) return
    if (this.seamBoundary === null) {
      const prev = this.readSeen()
      const candidate = this.anchorCandidate()
      if (prev === null ? candidate > 0 : candidate > prev) this.writeSeen()
      return
    }
    if (
      el.scrollHeight > 0 &&
      el.scrollHeight <= el.clientHeight + 1
    ) {
      this.writeSeen()
      this.dismissSeam()
    }
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
          ><span class="count">~${this.unseenCount} 条新消息</span>（自你上次查看）</span
        >
        <button type="button" class="link" @click=${this.markAllSeen}>
          全部标为已读
        </button>
      </div>
    `
    return html`
      <div class="scroll" role="log" aria-label="会话对话">
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
      <button
        type="button"
        class="jump-latest"
        data-testid="jump-latest"
        ?hidden=${!this.jumpVisible}
        @click=${this.jumpToLatest}
      >
        ↓ 跳到最新
      </button>
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
    if (u.kind === 'notice') return this.renderNoticeUnit(u)
    if (u.kind === 'mode_result') return this.renderModeResultUnit(u)
    if (u.kind === 'escalate_downgrade') return this.renderEscalateDowngradeUnit(u)
    if (u.kind === 'model_change') return this.renderModelChangeUnit(u)
    if (u.kind === 'operator') return this.renderOperatorUnit(u, receipt)
    return this.renderAgentUnit(u)
  }

  /**
   * （fix-webui-qa-round8 1.2）升级降级契约条目的渲染：中性系统条目（i 头像
   * + 「审批降级」标签），正文说明「已按仅放行一次降级」并原文呈现操作者
   * 填写的升级原因——降级不再静默，原因去向可读。
   */
  private renderEscalateDowngradeUnit(u: EscalateDowngradeUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    const p = parseEscalateDowngradePayload(e.content)
    return html`
      <div class="turn-block is-notice" data-testid="escalate-downgrade-entry">
        <div class="avatar notice">i</div>
        <div class="bubble notice">
          <div class="meta">
            <span class="author notice">审批降级</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">
            <p>
              ${p.detail || '该决策已按「仅放行一次」降级执行'}${p.tool
                ? html`（工具：<span class="mode-mode">${p.tool}</span>）`
                : nothing}
            </p>
            ${p.reason
              ? html`<p>
                  升级原因：<span data-testid="escalate-downgrade-reason">${p.reason}</span>
                </p>`
              : nothing}
          </div>
        </div>
      </div>
    `
  }

  /**
   * （fix-webui-qa-round8 5.2）模型切换留痕条目的渲染：中性系统条目（i 头像
   * + 「模型」标签），正文「模型已切换：A → B」——两条执行体同一形态。
   */
  private renderModelChangeUnit(u: ModelChangeUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    const p = parseModelChangePayload(e.content)
    return html`
      <div class="turn-block is-notice" data-testid="model-change-entry">
        <div class="avatar notice">i</div>
        <div class="bubble notice">
          <div class="meta">
            <span class="author notice">模型</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">
            <p>
              模型已切换：<span class="mode-mode">${p.from ?? '（默认）'}</span>
              <span aria-hidden="true">→</span>
              <span class="mode-mode">${p.to}</span>
            </p>
          </div>
        </div>
      </div>
    `
  }

  /**
   * （fix-webui-qa-findings D2）模式切换契约条目的第一类渲染：中性模式章
   * （i 头像 + 生效模式词），成功显示「已切换 → {mode}」，失败挂失败色与
   * 成因——拒绝与成功一眼可辨（与权限 spec「denial distinguishable」一致）。
   */
  private renderModeResultUnit(u: ModeResultUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    const payload = parseModeResultPayload(e.content)
    return html`
      <div class="turn-block is-notice" data-testid="mode-result-entry" data-mode-ok=${payload.ok}>
        <div class="avatar notice">i</div>
        <div class="bubble notice">
          <div class="meta">
            <span class="author notice">权限模式</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">
            ${payload.ok
              ? html`<p>权限模式已切换：<span class="mode-mode">${modeBadgeLabel(payload.mode)}</span></p>`
              : html`<p class="mode-failed">模式切换失败：${payload.detail || '执行体未接受'}</p>`}
          </div>
        </div>
      </div>
    `
  }

  /**
   * 零输出回合的中性信息条（close-acceptance-blind-spots 4.2，design D3）：
   * `notice` 条目绝不走错误红泡——`i` 头像 + 中性标签「提示」+ surface-2
   * 底的浅信息条。颜色全部走语义 token（明暗两主题同源翻转，无需各自的
   * 硬编码色）。
   */
  private renderNoticeUnit(u: NoticeUnit) {
    const e = u.entry
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    return html`
      <div class="turn-block is-notice" data-testid="notice-entry">
        <div class="avatar notice">i</div>
        <div class="bubble notice">
          <div class="meta">
            <span class="author notice">提示</span>
            <time class="time" datetime=${iso || nothing}>${ts}</time>
          </div>
          <div class="body">${unsafeHTML(renderMarkdown(e.content))}</div>
        </div>
      </div>
    `
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
    // （fix-webui-qa-round10 2.4，B-DEF-02）回合徽章读**回合观察模型**（该
    // 回合开始时生效的模型，model_change 留痕重放）——不再读会话当前模型：
    // 切换模型只影响之后的回合，历史徽章绝不回溯改写。观察不到（该回合前
    // 无任何 model_change 留痕）如实无徽章，不伪造。
    const model = this.observedModels.get(u.position)?.trim() ?? ''
    return html`
      <div class="turn-block is-assistant" data-turn-position=${u.position}>
        <div class="avatar assistant">${avatar}</div>
        <div class="flow">
          <div class="meta">
            <span class="author">${label}</span>
            ${model
              ? html`<span class="model-chip" data-testid="turn-model" title="本回合生效模型">${model}</span>`
              : nothing}
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
   * （fold-tool-calls-into-process-tree 4.2）合并块渲染（原顶层
   * `renderToolResultRun` 并入过程树）：收起行 = 调用态结构化标题 +
   * ✓已执行/✗已拒绝章（已配对才带章；未配对调用保持调用态标题，D5），
   * 默认收起；展开体 = 参数段 + 结果段（各走既有截断 + 「查看全部」）。
   * 开合只写自己的 id（`item:<position>`），绝不触碰祖先折叠（D7）。
   */
  private renderToolCallBlock(block: ToolCallBlock) {
    const id = `item:${block.position}`
    const open = this.foldOpen.get(id) === true
    const denied = toolCallBlockDenied(block)
    const { label, full } = processItemLabel(block.invocation)
    const outcome = toolCallBlockOutcome(block)
    return html`
      <div
        class="tool-result"
        data-testid="tool-result-entry"
        data-position=${block.position}
        data-denied=${denied}
      >
        <button
          type="button"
          class="fold-link item-link"
          data-testid="tool-result-link"
          aria-expanded=${open}
          title=${full ?? nothing}
          @click=${this.toggleFold(id)}
        >
          <span class="item-title">${label}</span>
          ${outcome === 'ok'
            ? html`<span class="outcome outcome-ok" data-testid="tool-outcome">✓ 已执行</span>`
            : nothing}
          ${outcome === 'denied'
            ? html`<span class="outcome outcome-denied" data-testid="tool-outcome-denied"
                >✗ 已拒绝</span
              >`
            : nothing}
        </button>
        ${open ? this.renderToolCallBody(block) : nothing}
      </div>
    `
  }

  /**
   * 合并块展开体（fold-tool-calls-into-process-tree 4.3）：参数段（📖 调用
   * 条目的 markdown + json）+ 结果段（✓ 条目内容），两段各走既有
   * `truncateHtml` + 「查看全部」弹层；被拒结果的首行 ✓ 改写为 ✗（既有
   * 口径）。任一段超阈值即在容器上挂 `data-truncated`（与二级条目同一
   * 明示合同）。未配对块只有参数段（调用态）。
   */
  private renderToolCallBody(block: ToolCallBlock) {
    const cutArgs = truncateHtml({
      ...block.invocation,
      content: deniedDetailContent(block.invocation.content, block.invocation.title),
    })
    const cutResult = block.result
      ? truncateHtml({
          ...block.result,
          content: deniedDetailContent(block.result.content, block.result.title),
        })
      : null
    const truncated = cutArgs.truncated || (cutResult?.truncated ?? false)
    const body = html`
      <div class="call-args">
        ${cutArgs.truncated
          ? this.truncatedSection(block.invocation, cutArgs)
          : unsafeHTML(renderMarkdown(cutArgs.preview))}
      </div>
      ${block.result
        ? html`<div class="call-result">
            ${cutResult!.truncated
              ? this.truncatedSection(block.result!, cutResult!)
              : unsafeHTML(renderMarkdown(cutResult!.preview))}
          </div>`
        : nothing}
    `
    return truncated
      ? html`<div class="body item-body tool-call-body" data-testid="tool-call-body" data-truncated>
          ${body}
        </div>`
      : html`<div class="body item-body tool-call-body" data-testid="tool-call-body">${body}</div>`
  }

  /** 单段截断体：预览 + 明示省略量 + 「查看全部」出口（4.5）。 */
  private truncatedSection(it: ProcessItem, cut: TruncateResult) {
    return html`
      ${unsafeHTML(renderMarkdown(cut.preview))}
      <p class="truncation-note" data-testid="truncation-note">
        已截断：省略 ${cut.omittedLines} 行 / ${cut.omittedChars} 字符
        <button type="button" class="view-all" @click=${() => this.openViewAll(it)}>
          查看全部
        </button>
      </p>
    `
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
    // （fix-webui-qa-round3 D1）data-kind 标成员构成，glyph 按 kind 挑：
    // thinking 独占的 run 挂 thinking 专用 glyph——收起行不展开即可辨识
    // 「此处是 thinking/工具过程」，而非一条与正文无异的裸文本（spec
    // 「thinking fold is titled and distinguishable」）。PROCESS 标签、计数
    // 与摘要仍只出现在折叠行上；正文段（.body）永不携带（分派层保证：
    // text 条目永不入过程 run，scenario「text never wears a process chip」）。
    const kind = processRunKind(r)
    return html`
      <div class="process-fold" data-process-id=${id} data-process-count=${r.items.length} data-kind=${kind}>
        <button
          type="button"
          class="fold-link"
          data-testid="process-fold-link"
          aria-expanded=${open}
          title=${full ?? nothing}
          @click=${this.toggleFold(id)}
        >
          <span class="kind-icon" aria-hidden="true">${icon(kind === 'tool' ? 'zap' : 'thinking', 11)}</span>
          <span class="label">过程</span>
          <span class="running">${label}</span>
          <span class="fold-count">${r.items.length}</span>
          ${processRunOutcome(r) === 'ok'
            ? html`<span class="outcome outcome-ok" data-testid="tool-outcome">✓ 已执行</span>`
            : nothing}
          ${processRunOutcome(r) === 'denied'
            ? html`<span class="outcome outcome-denied" data-testid="tool-outcome-denied">✗ 已拒绝</span>`
            : nothing}
        </button>
        ${open
          ? html`<div class="body fold-body">
              ${repeat(
                // （fold-tool-calls-into-process-tree 4.2）单棵过程树：思考
                // 段保持二级条目，工具条目按 id 配对合并为合并块（各自默认
                // 收起）。键 = 合并块取调用 position（流式期间调用先落、
                // 结果后并入，块的 DOM 身份不随配对移动）。
                mergeToolCalls(mergeAdjacentThinking(r.items)) as ProcessNode[],
                (node) => ('type' in node ? `tc:${node.position}` : node.position),
                (node) =>
                  'type' in node
                    ? this.renderToolCallBlock(node)
                    : this.renderProcessItem(node),
              )}
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
    const { label } = processItemLabel(it)
    // （fix-webui-qa-round3 D1）thinking 条目的二级折叠标题标明 thinking：
    // glyph + data-element-type 已有词，收起行再加 thinking 专用 glyph——
    // 与工具合并块（title 词）在视觉上分开（spec「thinking 段的第二级折叠
    // 标题 SHALL 标明 thinking」）。
    // （fix-webui-qa-round6 3.1，design D4）thinking 条目按 element_type 分支
    // 补内容渲染：展开的过程折叠内**默认**显示条目携带的 thinking 文本（与
    // markdown 同层，不再要求对二级折叠的第二次点击）——占位词「thinking」
    // 只作为条目标签保留在标题行，绝不再顶替内容。
    // （fold-tool-calls-into-process-tree 4.2）工具条目不再以裸二级条目
    // 到达——一律经 mergeToolCalls 合并后走 renderToolCallBlock（单棵
    // 过程树）。
    return html`
      <div class="process-item" data-position=${it.position} data-element-type=${it.elementType}>
        <span class="item-link" data-testid="thinking-item-head">
          <span class="kind-icon item-kind-icon" aria-hidden="true">${icon('thinking', 10)}</span>
          <span class="item-title">${label}</span>
        </span>
        ${this.renderItemBody(it)}
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
