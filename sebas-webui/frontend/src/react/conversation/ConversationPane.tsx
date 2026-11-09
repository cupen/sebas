import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { Alert, Button, Input, Skeleton, Tag, Typography } from 'antd'
import { api, errorText, type PermissionDecision } from '../../api/client.js'
import { sharedWs } from '../../api/shared-ws.js'
import {
  clearOpeningSeam,
  peekOpeningSeam,
  readAnchorCount,
  writeSeen,
} from '../../views/unread-cursor.js'
import {
  clampComposerHeight,
  COMPOSER_DEFAULT_PX,
  loadComposerHeight,
  saveComposerHeight,
} from '../../views/split-persist.js'
import type { ConversationStore } from '../state/conversation-store.js'
import { useStore } from '../state/use-store.js'
import { Composer } from './Composer.js'
import { TranscriptView } from './TranscriptView.js'
import { mirrorScrollSurface } from './pane-scroll.js'
import styles from './conversation.module.css'

export interface ConversationPaneProps {
  sessionKey: string
  conversation: ConversationStore
  reachability: { ok: boolean; cause?: string } | null
  /** Role capability: viewers get a read-only transcript, no composer (2.5). */
  canWrite?: boolean
}

interface Approval {
  request_id: string
  tool_name: string
  args: unknown
  reason?: string
}

const NEAR_BOTTOM_PX = 48

/**
 * Window in which a programmatic opening scroll (seam centering) is not read
 * as a read-to-bottom gesture. Lit uses the same guard (150ms) so a short
 * unread tail cannot self-clear the seam right after it is drawn.
 */
export const OPENING_SCROLL_GUARD_MS = 150

function mergeApprovals(
  incoming: readonly { request_id: string; tool_name: string; args: unknown; reason?: string }[],
  current: readonly Approval[],
): readonly Approval[] {
  const byId = new Map(current.map((item) => [item.request_id, item]))
  for (const item of incoming) {
    const existing = byId.get(item.request_id)
    byId.set(item.request_id, { ...existing, ...item })
  }
  return [...byId.values()]
}

/** Trailing ordinal of an agent-namespaced request id (`claude:tc-12` → 12). */
function requestOrdinal(id: string): number | null {
  const match = /(\d+)$/.exec(id)
  return match ? Number(match[1]) : null
}

/**
 * Deterministic pending-card order. Live frames arrive in tool-call order while
 * a reload rebuilds the list from the read model (a different source order);
 * sorting by the request-id ordinal keeps the same pending set in the same
 * display order across the reload instead of flipping without cause.
 */
function compareApprovals(left: Approval, right: Approval): number {
  const a = requestOrdinal(left.request_id)
  const b = requestOrdinal(right.request_id)
  if (a !== null && b !== null && a !== b) return a - b
  if ((a === null) !== (b === null)) return a === null ? 1 : -1
  return left.request_id.localeCompare(right.request_id)
}

function orderedApprovals(items: readonly Approval[]): readonly Approval[] {
  return [...items].sort(compareApprovals)
}

/** Live transcript + composer for one focused session (4.1–4.3). */
export function ConversationPane({
  sessionKey,
  conversation,
  reachability,
  canWrite = true,
}: ConversationPaneProps) {
  const state = useStore(conversation.store, (value) => value)
  const [approvals, setApprovals] = useState<readonly Approval[]>([])
  const [approvalBusy, setApprovalBusy] = useState<ReadonlySet<string>>(new Set())
  const [approvalError, setApprovalError] = useState<string | null>(null)
  const [escalateReason, setEscalateReason] = useState('')
  const scroll = useRef<HTMLDivElement | null>(null)
  const pane = useRef<HTMLDivElement | null>(null)
  const [composerHeight, setComposerHeight] = useState(
    () => loadComposerHeight() ?? COMPOSER_DEFAULT_PX,
  )
  const composerHeightRef = useRef(composerHeight)
  const composerDrag = useRef<{ y: number; height: number } | null>(null)
  const follow = useRef(true)
  const openedFor = useRef<string | null>(null)
  /** The opening anchor settlement runs once per opened session. */
  const settledFor = useRef<string | null>(null)
  /** Programmatic opening scroll must not be read as a read-to-bottom gesture. */
  const openingScrollUntil = useRef(0)
  const [seam, setSeam] = useState<number | null>(() =>
    peekOpeningSeam(sessionKey) === undefined
      ? readAnchorCount(sessionKey)
      : (peekOpeningSeam(sessionKey) as number | null),
  )
  const [showJump, setShowJump] = useState(false)
  /**
   * Latest scroll-gesture handler. The mirror below is installed once (it
   * rebinds DOM properties), so it must reach the current closure instead of
   * freezing the first render's `state` (which had no detail yet).
   */
  const scrollGesture = useRef<() => void>(() => {})

  useEffect(() => {
    composerHeightRef.current = composerHeight
  }, [composerHeight])

  // The pane is the operator-addressed transcript surface, but the scrolling
  // box is the child the composer leaves room for. Mirror the metrics so
  // "scroll the pane to the bottom" means the transcript, not a no-op on a
  // flex wrapper.
  useEffect(() => {
    const outer = pane.current
    const inner = scroll.current
    if (outer === null || inner === null) return
    return mirrorScrollSurface(outer, inner, {
      // An explicit `pane.scrollTop = …` IS the gesture; the opening-centering
      // guard only exists to ignore scroll events that centering itself emits.
      onScrollTop: () => scrollGesture.current(),
    })
  }, [])

  function beginComposerResize(event: ReactPointerEvent<HTMLDivElement>): void {
    event.preventDefault()
    composerDrag.current = { y: event.clientY, height: composerHeightRef.current }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  function moveComposerResize(event: ReactPointerEvent<HTMLDivElement>): void {
    const origin = composerDrag.current
    const areaHeight = pane.current?.getBoundingClientRect().height ?? 0
    if (origin === null) return
    // Dragging the divider upward grows the composer.
    const next = clampComposerHeight(origin.height + origin.y - event.clientY, areaHeight)
    composerHeightRef.current = next
    setComposerHeight(next)
    saveComposerHeight(next, areaHeight)
  }

  function endComposerResize(event: ReactPointerEvent<HTMLDivElement>): void {
    composerDrag.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }

  // Opening boundary (Lit parity, round2 2.1/2.6): rail arms the pre-write
  // anchor before switching; a deep link without an arm falls back to the
  // stored anchor. Either way the boundary — not the post-write cursor — is
  // what decides where the seam is drawn.
  useEffect(() => {
    const armed = peekOpeningSeam(sessionKey)
    setSeam(armed === undefined ? readAnchorCount(sessionKey) : armed)
    follow.current = true
    openedFor.current = null
    settledFor.current = null
    openingScrollUntil.current = 0
    setShowJump(false)
    setApprovals([])
    setApprovalError(null)
    setEscalateReason('')
  }, [sessionKey])

  const waiting = state.detail?.status_slug === 'waiting'
  // 审批决策只在 waiting 相位可投递（与上方轮询 gate 同源）。其它相位里的
  // 滞留卡片——回合已取消/结束——操作一律禁用（视觉验收 DEF-2）；真正的
  // 迟到竞态仍会走 answer() 的错误 Alert 路径，404 如实呈现不受影响。
  const approvalsStale = !waiting
  useEffect(() => {
    if (!waiting) return
    let cancelled = false
    let timer: number | undefined
    let delay = 250
    const pull = async (): Promise<void> => {
      try {
        const response = await api.sessionApprovals(sessionKey)
        if (cancelled) return
        setApprovals((current) => mergeApprovals(response.approvals, current))
        // The gate may not be committed yet: keep backing off (250ms doubling
        // to 4s) while the phase says waiting but the read model is empty.
        if (response.approvals.length === 0) {
          timer = window.setTimeout(() => void pull(), delay)
          delay = Math.min(delay * 2, 4_000)
        }
      } catch {
        if (cancelled) return
        timer = window.setTimeout(() => void pull(), delay)
        delay = Math.min(delay * 2, 4_000)
      }
    }
    void pull()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [sessionKey, waiting])

  useEffect(() => {
    const off = sharedWs.subscribe((event) => {
      switch (event.type) {
        case 'turn.append':
          if (event.session_id === sessionKey) conversation.append(sessionKey, event.entries)
          break
        case 'session.updated':
          if (event.session_id === sessionKey) {
            conversation.applyPhase(sessionKey, {
              status_slug: event.status_slug,
              turn_engaged: event.turn_engaged,
              msg_count: event.msg_count,
              pending: event.pending,
              usage: event.usage,
            })
          }
          break
        case 'session.resync':
          conversation.resetCursor()
          void conversation.refresh()
          void api.sessionApprovals(sessionKey).then((response) => {
            setApprovals((current) => mergeApprovals(response.approvals, current))
          })
          break
        case 'permission.requested':
          if (event.session_id === sessionKey) {
            setApprovals((current) => mergeApprovals([
              {
                request_id: event.request_id,
                tool_name: event.tool_name,
                args: event.args,
                reason: event.reason,
              },
            ], current))
          }
          break
        default:
          break
      }
    })
    return off
  }, [sessionKey, conversation])

  useEffect(() => {
    const refetch = () => {
      // Reconnect / manual retry: the local cursor may have missed frames
      // while offline, so the detail snapshot is authoritative again.
      conversation.resetCursor()
      void conversation.refresh()
      void api
        .sessionApprovals(sessionKey)
        .then((response) => {
          setApprovals((current) => mergeApprovals(response.approvals, current))
        })
        .catch(() => {})
    }
    window.addEventListener('sebas:refetch', refetch)
    return () => window.removeEventListener('sebas:refetch', refetch)
  }, [sessionKey, conversation])

  useEffect(() => {
    const element = scroll.current
    if (element === null) return
    if (!follow.current) {
      setShowJump(true)
      return
    }
    element.scrollTop = element.scrollHeight
  }, [state.entries.length, state.detail?.status_slug])

  // Opening with an unseen boundary shows the seam instead of the bottom
  // (round8 3.1): follow stays detached so new arrivals raise the jump pill.
  // The programmatic centering itself is not a read gesture — without the
  // guard, a short unread tail lands within NEAR_BOTTOM_PX, onScroll clears
  // the seam it just drew, and the boundary never reaches the operator
  // (Lit's `openingScrollUntil` window has the same job).
  useEffect(() => {
    const element = scroll.current
    if (element === null || seam === null) return
    if (openedFor.current === sessionKey) return
    const boundary = element.querySelector('[role="separator"]')
    if (!(boundary instanceof HTMLElement)) return
    openedFor.current = sessionKey
    follow.current = false
    setShowJump(false)
    openingScrollUntil.current = Date.now() + OPENING_SCROLL_GUARD_MS
    boundary.scrollIntoView({ block: 'center', behavior: 'auto' })
  }, [sessionKey, seam, state.entries.length])

  /**
   * Opening anchor settlement (round8 5.1, Lit `settleOpeningAnchor`): an
   * opening that carries an unread boundary but whose whole content already
   * fits the viewport was read by the act of looking — write the anchor and
   * clear the boundary. Overflowing content keeps both: opening is not
   * reading, the badge and seam stay until a real read gesture. A fully-read
   * opening (no boundary) is owned by the follow settlement below.
   */
  useEffect(() => {
    const element = scroll.current
    const detail = state.detail
    if (element === null || detail === null || detail.encoded_key !== sessionKey) return
    if (settledFor.current === sessionKey) return
    if (seam === null) return
    if (document.visibilityState !== 'visible') return
    settledFor.current = sessionKey
    if (element.scrollHeight > 0 && element.scrollHeight <= element.clientHeight + 1) {
      writeSeen(sessionKey, detail.msg_count)
      follow.current = true
      dismissSeam()
    }
  }, [sessionKey, seam, state.entries.length, state.detail?.msg_count])

  // Seen-driven follow settlement (round8 5.1): with no seam, layout settling
  // at the bottom advances the shared cursor. An opening with a seam keeps it
  // on screen as the "last read here" boundary even when the content happens
  // to fit; only an explicit read gesture (scroll/jump) clears it.
  useEffect(() => {
    const element = scroll.current
    const detail = state.detail
    if (element === null || detail === null || detail.encoded_key !== sessionKey) return
    if (!follow.current || document.visibilityState !== 'visible') return
    // (DD3) an in-flight tail segment is not "seen" until the turn settles.
    if (detail.turn_engaged) return
    if (seam !== null) return
    const frame = requestAnimationFrame(() => {
      if (!follow.current) return
      const previous = readAnchorCount(sessionKey)
      if (previous === null ? detail.msg_count > 0 : detail.msg_count > previous) {
        writeSeen(sessionKey, detail.msg_count)
      }
    })
    return () => cancelAnimationFrame(frame)
  }, [sessionKey, seam, state.entries.length, state.detail?.msg_count])

  function jumpToLatest(): void {
    const element = scroll.current
    if (element === null) return
    follow.current = true
    setShowJump(false)
    element.scrollTop = element.scrollHeight
    if (state.detail !== null) {
      writeSeen(sessionKey, state.detail.msg_count)
      dismissSeam()
    }
  }

  function onScroll(): void {
    // The opening centering is a programmatic position, not a reading gesture:
    // ignoring it keeps the seam on screen until the operator really scrolls.
    if (Date.now() < openingScrollUntil.current) return
    settleScrollPosition()
  }

  /**
   * A read gesture at the current position: near the bottom means "read to the
   * end", so the shared anchor advances and any opening seam is dismissed.
   * Reached from the real scroll event and from a programmatic
   * `conversation-pane.scrollTop` (the mirror's gesture hook), because an
   * assignment onto an already-bottomed pane emits no scroll event.
   */
  function settleScrollPosition(): void {
    const element = scroll.current
    if (element === null) return
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight
    follow.current = distance <= NEAR_BOTTOM_PX
    setShowJump(!follow.current)
    if (follow.current && state.detail !== null) {
      writeSeen(sessionKey, state.detail.msg_count)
      dismissSeam()
    }
  }


  /** Seam cleared for good: the opening arm dies with it (no replay on remount). */
  function dismissSeam(): void {
    clearOpeningSeam(sessionKey)
    setSeam(null)
  }

  // Keep the mirror's gesture hook pointed at the current closure (state and
  // handlers change every render; the mirror DOM binding does not). It targets
  // the position settlement, not `onScroll`: the opening-scroll guard there
  // would swallow a deliberate programmatic assignment.
  useEffect(() => {
    scrollGesture.current = settleScrollPosition
  })

  async function answer(requestId: string, decision: PermissionDecision): Promise<void> {
    setApprovalBusy((current) => new Set(current).add(requestId))
    setApprovalError(null)
    try {
      await api.answerPermission(requestId, decision)
      setApprovals((current) => current.filter((item) => item.request_id !== requestId))
      await conversation.refresh()
    } catch (failure) {
      setApprovalError(errorText(failure))
    } finally {
      setApprovalBusy((current) => {
        const next = new Set(current)
        next.delete(requestId)
        return next
      })
    }
  }

  const detail = state.detail
  const entries = useMemo(() => state.entries, [state.entries])
  const writable = canWrite

  return (
    <div className={styles.pane} data-testid="conversation-pane" ref={pane}>
      <div className={styles.scroll} ref={scroll} onScroll={onScroll}>
        {state.loading && detail === null ? <Skeleton active paragraph={{ rows: 6 }} /> : null}
        {state.error !== null ? (
          <Alert
            type="error"
            showIcon
            title={`会话加载失败：${state.error}`}
            action={<Button size="small" onClick={() => void conversation.refresh()}>重试</Button>}
            role="alert"
          />
        ) : null}
        {detail !== null ? (
          <TranscriptView entries={entries} seamAnchor={seam} live={detail.turn_engaged} />
        ) : null}
      </div>

      {showJump ? (
        <Button
          size="small"
          className={styles.jump}
          aria-label="跳到最新"
          onClick={jumpToLatest}
        >
          跳到最新
        </Button>
      ) : null}

      {approvals.length > 0 ? (
        <div className={styles.approvals} aria-label="待审批工具调用">
          {/* 回合不再占用（已取消/已结束）时，滞留的审批请求已无处投递——
           * 决策会得到 404「no pending permission request」。操作保持可见但
           * 禁用，如实呈现「已失效」而不是误导可点（视觉验收 DEF-2）；真正
           * 的迟到竞态仍走 answer() 的错误 Alert 路径。 */}
          {orderedApprovals(approvals).map((approval) => (
            <div
              key={approval.request_id}
              className={styles.approval}
              role="alertdialog"
              aria-label={`待审批 ${approval.tool_name}`}
            >
              <div className={styles.approvalHead}>
                <Tag color="orange">待审批</Tag>
                <Typography.Text strong>{approval.tool_name}</Typography.Text>
              </div>
              {approval.reason ? (
                <Typography.Paragraph type="secondary" className={styles.approvalReason}>
                  {approval.reason}
                </Typography.Paragraph>
              ) : null}
              <pre className={styles.approvalArgs}>{JSON.stringify(approval.args, null, 2)}</pre>
              {approvalError !== null ? <Alert type="error" showIcon title={approvalError} /> : null}
              <div className={styles.approvalActions}>
                <Button
                  type="primary"
                  loading={approvalBusy.has(approval.request_id)}
                  disabled={approvalsStale}
                  onClick={() => void answer(approval.request_id, { decision: 'allow_once' })}
                >
                  允许一次
                </Button>
                <Button
                  loading={approvalBusy.has(approval.request_id)}
                  disabled={approvalsStale}
                  onClick={() => void answer(approval.request_id, { decision: 'allow_session' })}
                >
                  本会话允许
                </Button>
                <Button
                  danger
                  loading={approvalBusy.has(approval.request_id)}
                  disabled={approvalsStale}
                  onClick={() => void answer(approval.request_id, { decision: 'deny' })}
                >
                  拒绝
                </Button>
                <Input
                  size="small"
                  aria-label="原因"
                  placeholder="上抛原因"
                  value={escalateReason}
                  onChange={(event) => setEscalateReason(event.target.value)}
                  disabled={approvalsStale}
                  className={styles.escalateReason}
                />
                <Button
                  loading={approvalBusy.has(approval.request_id)}
                  disabled={approvalsStale || escalateReason.trim() === ''}
                  onClick={() =>
                    void answer(approval.request_id, {
                      decision: 'escalate',
                      reason: escalateReason.trim(),
                    })
                  }
                >
                  上抛
                </Button>
              </div>
            </div>
          ))}
        </div>
      ) : null}

      {detail !== null && writable ? (
        <>
          <div
            className={styles.vsplit}
            role="separator"
            aria-label="调整输入区高度"
            aria-orientation="horizontal"
            aria-valuemin={120}
            aria-valuenow={Math.round(composerHeight)}
            tabIndex={0}
            onPointerDown={beginComposerResize}
            onPointerMove={moveComposerResize}
            onPointerUp={endComposerResize}
            onPointerCancel={endComposerResize}
            onKeyDown={(event) => {
              const areaHeight = pane.current?.getBoundingClientRect().height ?? 0
              if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
                event.preventDefault()
                const delta = event.key === 'ArrowUp' ? 16 : -16
                const next = clampComposerHeight(composerHeightRef.current + delta, areaHeight)
                composerHeightRef.current = next
                setComposerHeight(next)
                saveComposerHeight(next, areaHeight)
              }
            }}
          />
          <div className={styles.composerArea} style={{ height: composerHeight }}>
            <Composer
              sessionKey={sessionKey}
              detail={detail}
              reachability={reachability}
              onRefresh={async () => { await conversation.refresh() }}
            />
          </div>
        </>
      ) : null}
      {detail !== null && !writable ? (
        // Viewer (sessions.write absent): the transcript stays readable, but a
        // submittable composer would be a phantom affordance — every write is
        // 403 server-side. State the read-only contract in its place.
        <Alert
          type="info"
          showIcon
          className={styles.readonlyNote}
          title="只读视图：当前角色没有会话写权限，不能发送消息。"
        />
      ) : null}
    </div>
  )
}
