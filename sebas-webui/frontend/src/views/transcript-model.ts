/**
 * Framework-agnostic transcript model (2.5). Pure conversions from the
 * server's ordered entry sequence to the turn/run/unit shapes the UI renders.
 * Deliberately free of Lit, DOM and network imports so both the existing
 * `<sebas-transcript-view>` and the React workbench can share ONE derivation.
 *
 * `transcript-view.ts` re-exports every symbol here, so existing imports and
 * tests keep working unchanged.
 */

import type { ConversationEntryView } from '../api/client.js'

/**
 * 曾以「空流」（0 回合）渲染过的会话 key（polish-workbench-walkthrough-ux 3.1）：
 * 这些会话的首个回合若在聚焦 + 可见 + 贴底时到达，算「亲眼看着到达」的首交换，
 * 锚从空流建立。模块级而非实例字段——秒回场景下 dashboard 可能重建组件实例。
 * 一旦某 key 以非空回合渲染过即出表（一次性消费），所以表里只剩仍为空的会话。
 */
export const emptyStreamSessions = new Set<string>()

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
export function graphemes(s: string): string[] {
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
