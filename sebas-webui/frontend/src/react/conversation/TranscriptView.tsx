import { memo, useCallback, useMemo, useRef, useState } from 'react'
import { Button, Modal, Tag } from 'antd'
import type { ConversationEntryView } from '../../api/client.js'
import { renderMarkdown } from '../../components/markdown.js'
import {
  awaitingReceipt,
  deniedDetailContent,
  errorEntryLabel,
  groupConversation,
  mergeAdjacentThinking,
  mergeSpawnErrors,
  parseEscalateDowngradePayload,
  parseModelChangePayload,
  processItemLabel,
  processRunOutcome,
  processRunSummary,
  truncateHtml,
  turnObservedModels,
  unitSegmentCount,
  type AgentRun,
  type ProcessItem,
  type TurnUnit,
} from '../../views/transcript-model.js'
import styles from './transcript.module.css'

export interface TranscriptViewProps {
  entries: readonly ConversationEntryView[]
  /** Read anchor at focus time (segment count); null = no seam. */
  seamAnchor?: number | null
  emptyText?: string
  /**
   * Engine fact `turn_engaged`: the last agent turn's tail text renders as
   * plain text until the turn settles (streaming must never re-parse history
   * with the Markdown pipeline). Settlement renders it exactly once.
   */
  live?: boolean
  /** Perf harness hook: one call per rendered unit (memoized units skip). */
  onUnitRender?: (key: string) => void
  /** Perf harness hook: one call per Markdown parse. */
  onMarkdownRender?: () => void
}

const Markdown = memo(function Markdown({
  content,
  onRender,
}: {
  content: string
  onRender?: () => void
}) {
  onRender?.()
  return <div className={styles.markdown} dangerouslySetInnerHTML={{ __html: renderMarkdown(content) }} />
})

/** Full-content overlay target (truncated entry escape hatch). */
interface ViewAllEntry {
  title: string
  content: string
}

/**
 * Keep the array identity of historical units stable as the live tail grows.
 * Entry objects are append-only (the store merges by position), so a pure
 * prefix (same object identity for every previous entry) lets every unit
 * before the previous tail be reused verbatim — historical entries then skip
 * React render entirely on a delta, not merely skip Markdown.
 */
function useStableUnits(entries: readonly ConversationEntryView[]): TurnUnit[] {
  const cache = useRef<{ entries: readonly ConversationEntryView[]; units: TurnUnit[] } | null>(null)
  if (cache.current !== null && cache.current.entries === entries) return cache.current.units
  const previous = cache.current
  let units = groupConversation(mergeSpawnErrors([...entries]))
  if (previous !== null && entries.length > previous.entries.length && previous.units.length > 0) {
    let prefix = 0
    while (prefix < previous.entries.length && previous.entries[prefix] === entries[prefix]) prefix += 1
    if (prefix === previous.entries.length) {
      const oldByKey = new Map(previous.units.slice(0, -1).map((unit) => [unitKey(unit), unit]))
      units = units.map((unit) => oldByKey.get(unitKey(unit)) ?? unit)
    }
  }
  cache.current = { entries, units }
  return units
}

/** Grouped transcript with stable keys, tool folds and unread seam (4.2/4.3). */
export function TranscriptView({
  entries,
  seamAnchor = null,
  emptyText = '会话还没有消息',
  live = false,
  onUnitRender,
  onMarkdownRender,
}: TranscriptViewProps) {
  const [openFolds, setOpenFolds] = useState<ReadonlySet<string>>(new Set())
  const [viewAll, setViewAll] = useState<ViewAllEntry | null>(null)
  const units = useStableUnits(entries)
  // Observed-model timeline (fix-webui-qa-round10 2.4): the badge belongs to
  // the turn it started in; a later switch never rewrites history.
  const models = useMemo(() => turnObservedModels(units), [units])
  const onToggle = useCallback((id: string) => {
    setOpenFolds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])
  const onViewAll = useCallback((entry: ViewAllEntry) => setViewAll(entry), [])

  if (units.length === 0) return <p className={styles.empty}>{emptyText}</p>

  // Receipt phase is a pure entry-sequence fact: the last unit is still the
  // operator's submission (queued counts, not only `working`).
  const receipt = awaitingReceipt(units)

  let cumulative = 0
  const rows = units.map((unit, index) => {
    cumulative += unitSegmentCount(unit)
    // Anchor = read segment count: the seam belongs before the first unit that
    // pushes the cumulative total past it. An equal-total unit is fully read
    // and must not draw a seam.
    const showSeam = seamAnchor !== null && cumulative > seamAnchor
    return (
      <div key={`${unitKey(unit)}-${index}`} data-unit-key={unitKey(unit)}>
        {showSeam ? <Seam count={Math.max(0, cumulative - seamAnchor)} /> : null}
        <Unit
          unit={unit}
          model={unit.kind === 'agent' ? (models.get(unit.position) ?? '') : ''}
          receipt={receipt && index === units.length - 1}
          openFolds={openFolds}
          onToggle={onToggle}
          onViewAll={onViewAll}
          liveTail={live && index === units.length - 1}
          onUnitRender={onUnitRender}
          onMarkdownRender={onMarkdownRender}
        />
      </div>
    )
  })
  return (
    <div className={styles.transcript} data-testid="react-transcript">
      {rows}
      {viewAll !== null ? <ViewAllDialog entry={viewAll} onClose={() => setViewAll(null)} /> : null}
    </div>
  )
}

/**
 * Full-content escape hatch (truncation 4.5). AntD portals the dialog to
 * <body>, so it lives outside the conversation scroll surface; closing
 * unmounts its body (`destroyOnHidden`) and leaves no node in the transcript.
 */
function ViewAllDialog({ entry, onClose }: { entry: ViewAllEntry; onClose: () => void }) {
  return (
    <Modal
      open
      title={entry.title}
      footer={null}
      width={720}
      destroyOnHidden
      onCancel={onClose}
      data-testid="view-all-dialog"
    >
      <div className={styles.viewAllBody}>
        <div
          className={styles.markdown}
          dangerouslySetInnerHTML={{ __html: renderMarkdown(entry.content) }}
        />
      </div>
    </Modal>
  )
}

function Seam({ count }: { count: number }) {
  return (
    <div className={styles.seam} role="separator" aria-label={`未读分界，${count} 段未读`}>
      <span>未读</span>
    </div>
  )
}

function unitKey(unit: TurnUnit): string {
  switch (unit.kind) {
    case 'operator':
      return `op-${unit.entry.position}`
    case 'agent':
      return `agent-${unit.position}`
    default:
      return `${unit.kind}-${unit.entry.position}`
  }
}

const Unit = memo(function Unit({
  unit,
  model,
  receipt,
  openFolds,
  onToggle,
  onViewAll,
  liveTail,
  onUnitRender,
  onMarkdownRender,
}: {
  unit: TurnUnit
  model: string
  receipt: boolean
  openFolds: ReadonlySet<string>
  onToggle: (id: string) => void
  onViewAll: (entry: ViewAllEntry) => void
  liveTail: boolean
  onUnitRender?: (key: string) => void
  onMarkdownRender?: () => void
}) {
  onUnitRender?.(unitKey(unit))
  if (unit.kind === 'operator') {
    return (
      <article className={`${styles.unit} ${styles.operator}`}>
        <div className={styles.roleRow}>
          <span className={styles.role}>你</span>
          {receipt ? (
            <span
              className={styles.receipt}
              data-receipt
              title="服务端已接受，等待 agent 开始输出"
            >
              已收到
            </span>
          ) : null}
        </div>
        <pre className={styles.plain}>{unit.entry.content}</pre>
      </article>
    )
  }
  if (unit.kind === 'error') {
    // failure_class drives the label (spawn → 启动失败, stall → 回合停滞,
    // generic/legacy → 错误); the hardwired "spawn failed" is retired.
    const count = unit.entry.count ?? 1
    return (
      <article className={`${styles.unit} ${styles.error}`} role="alert">
        <div className={styles.roleRow}>
          <span className={styles.role}>{errorEntryLabel(unit.entry)}</span>
          {count > 1 ? <span className={styles.count}>×{count}</span> : null}
        </div>
        <Markdown content={unit.entry.content} onRender={onMarkdownRender} />
      </article>
    )
  }
  if (unit.kind === 'notice') {
    return (
      <article className={`${styles.unit} ${styles.notice}`}>
        <div className={styles.role}>提示</div>
        <Markdown content={unit.entry.content} onRender={onMarkdownRender} />
      </article>
    )
  }
  if (unit.kind === 'escalate_downgrade') {
    // Lit parity (fix-webui-qa-round8 1.2): a first-class contract entry, not
    // an anonymous system line — the downgrade must be visible with the tool
    // name and the operator's escalation reason.
    const payload = parseEscalateDowngradePayload(unit.entry.content)
    return (
      <article
        className={`${styles.unit} ${styles.notice}`}
        data-testid="escalate-downgrade-entry"
      >
        <div className={styles.role}>审批降级</div>
        <p className={styles.plain}>
          {payload.detail || '该决策已按「仅放行一次」降级执行'}
          {payload.tool ? `（工具：${payload.tool}）` : ''}
        </p>
        {payload.reason ? <p className={styles.plain}>升级原因：{payload.reason}</p> : null}
      </article>
    )
  }
  if (unit.kind === 'model_change') {
    const payload = parseModelChangePayload(unit.entry.content)
    return (
      <article className={`${styles.unit} ${styles.notice}`} data-testid="model-change-entry">
        <div className={styles.role}>模型</div>
        <p className={styles.plain}>
          模型已切换：{payload.from ?? '（默认）'} → {payload.to}
        </p>
      </article>
    )
  }
  if (unit.kind !== 'agent') {
    return (
      <article className={`${styles.unit} ${styles.system}`}>
        <div className={styles.role}>{unit.kind === 'mode_result' ? '模式' : '系统'}</div>
        <pre className={styles.plain}>{unit.entry.content}</pre>
      </article>
    )
  }
  return (
    <article className={`${styles.unit} ${styles.agent}`}>
      <div className={styles.roleRow}>
        <span className={styles.role}>agent</span>
        {model ? (
          <span className={styles.modelChip} data-testid="turn-model" title="本回合生效模型">
            {model}
          </span>
        ) : null}
      </div>
      {unit.runs.map((run, index) => (
        <AgentRunView
          key={`${unit.position}-${run.position}-${run.type}`}
          unitPosition={unit.position}
          run={run}
          open={openFolds.has(`${unit.position}:${run.position}`)}
          onToggle={onToggle}
          onViewAll={onViewAll}
          liveTail={liveTail && index === unit.runs.length - 1}
          onMarkdownRender={onMarkdownRender}
        />
      ))}
    </article>
  )
})

function AgentRunView({
  unitPosition,
  run,
  open,
  onToggle,
  onViewAll,
  liveTail,
  onMarkdownRender,
}: {
  unitPosition: number
  run: AgentRun
  open: boolean
  onToggle: (id: string) => void
  onViewAll: (entry: ViewAllEntry) => void
  liveTail: boolean
  onMarkdownRender?: () => void
}) {
  if (run.type === 'text') {
    // Live tail is plain text: a delta must never invoke marked/DOMPurify.
    return liveTail ? (
      <pre className={styles.plain}>{run.content}</pre>
    ) : (
      <Markdown content={run.content} onRender={onMarkdownRender} />
    )
  }
  const summary = processRunSummary(run)
  const outcome = processRunOutcome(run)
  const id = `${unitPosition}:${run.position}`
  // Adjacent thinking increments collapse into one paragraph before the body
  // renders (streamed deltas must not become a screen of fragments).
  const items = mergeAdjacentThinking(run.items)
  return (
    <details
      className={styles.fold}
      open={open}
      onToggle={(event) => {
        const next = (event.currentTarget as HTMLDetailsElement).open
        if (next !== open) onToggle(id)
      }}
    >
      <summary className={styles.foldHead}>
        <Tag>{summary.label}</Tag>
        <span className={styles.foldFull}>{summary.full}</span>
        <span className={styles.foldCount}>{run.items.length} 项</span>
        {outcome === 'ok' ? (
          <span className={styles.outcomeOk} data-testid="tool-outcome">
            ✓ 已执行
          </span>
        ) : null}
        {outcome === 'denied' ? (
          <span className={styles.outcomeDenied} data-testid="tool-outcome-denied">
            ✗ 已拒绝
          </span>
        ) : null}
      </summary>
      <div className={styles.foldBody}>
        {items.map((item) => (
          <ProcessItemView key={`item-${item.position}`} item={item} onViewAll={onViewAll} />
        ))}
      </div>
    </details>
  )
}

/** Second-level process entry with its content inline (no nested fold). */
function ProcessItemView({
  item,
  onViewAll,
}: {
  item: ProcessItem
  onViewAll: (entry: ViewAllEntry) => void
}) {
  const { label, full } = processItemLabel(item)
  return (
    <div
      className={styles.processItem}
      data-position={item.position}
      data-element-type={item.elementType}
    >
      <div className={styles.processHead}>
        <span className={styles.processKind}>
          {item.elementType === 'thinking' ? '思考' : '工具'}
        </span>
        <span className={styles.processTitle} data-testid="thinking-item-head" title={full ?? undefined}>
          {label}
        </span>
      </div>
      <ItemBody item={item} onViewAll={onViewAll} />
    </div>
  )
}

/**
 * Entry body with the truncation contract (4.5): over the line/char threshold
 * only the preview renders, with an honest omission count and a view-all exit.
 * Denied tool results rewrite the leading ✓ to ✗ (fix-webui-qa-defects 5.3).
 */
function ItemBody({
  item,
  onViewAll,
}: {
  item: ProcessItem
  onViewAll: (entry: ViewAllEntry) => void
}) {
  const content = deniedDetailContent(item.content, item.title)
  const cut = truncateHtml({ content })
  if (!cut.truncated) {
    return <pre className={styles.plain}>{content}</pre>
  }
  return (
    <div data-truncated>
      <pre className={styles.plain}>{cut.preview}</pre>
      <p className={styles.truncationNote} data-testid="truncation-note">
        已截断：省略 {cut.omittedLines} 行 / {cut.omittedChars} 字符
        <Button
          type="link"
          size="small"
          className={styles.viewAll}
          onClick={() => onViewAll({ title: processItemLabel(item).label, content })}
        >
          查看全部
        </Button>
      </p>
    </div>
  )
}
