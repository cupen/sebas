/**
 * Per-session read cursor shared by the rail's unread badge and the
 * transcript's seen-boundary seam (rail-declutter-unread D3;
 * session-parallel-liveness-and-unread-polish 2.3, design D3).
 *
 * localStorage 里的每会话游标**单字段** `{ anchor_count }`（u64）：
 *   - `anchor_count` = 「标记已读时的服务端可见回复段数」——rail 徽标 =
 *     `msg_count − anchor_count`，transcript 的已读缝也从同一个段计数推导，
 *     两条锚线合一（旧实现的 `seen_ts` 时间戳锚已整体退役）。
 *
 * 写入取单调 max（只进不退），多标签页同源共享同一份 localStorage。
 *
 * 旧数据兼容（D5b 同款「迁移是唯一消失点」语义）：存量 JSON 里含
 * `seen_ts`（或旧版裸时间戳数字）的对象在**读取时按「无 anchor」对待**
 * （读为 fully-read——换浏览器/清缓存后历史不集体冒红），不迁移字段；
 * 下一次写入（聚焦/读到底/流式贴底）即以纯 `{anchor_count}` 原地覆写。
 */

/** The wire shape written to localStorage: exactly one field. */
export interface StoredAnchor {
  anchor_count: number
}

function storageKey(sessionKey: string): string {
  return `sebas:seen:${sessionKey}`
}

/**
 * 锚点推进的窗口级广播（fix-webui-qa-defects-round3 6.1）：localStorage 不
 * 是响应式源——rail 徽标在锚被 transcript 贴底跟读/聚焦切换推进后，需要
 * 一次显式失效才会按新水位重渲染（否则徽标驻留到下一个无关状态变化，
 * 「读了但徽标不消」）。写侧广播、读侧（rail）按需失效；`detail.key` 是
 * 会话 encoded key，无订阅者时派发是无害 no-op。
 */
export const ANCHOR_ADVANCED_EVENT = 'sebas:anchor-advanced'

/**
 * Parse a stored value into the read anchor. 返回 `null` = 无锚（包括旧
 * `seen_ts` 形态——按「无 anchor」= fully-read 对待，不迁移）。
 */
function parse(raw: string | null): number | null {
  if (raw === null) return null
  try {
    const v = JSON.parse(raw) as unknown
    // （2.3）纯新格式：`{"anchor_count": n}`。任何含 seen_ts 的旧对象、
    // 裸数字、缺/非法 anchor_count 的对象一律视为无锚（fully-read）。
    if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
      const count = (v as { anchor_count?: unknown }).anchor_count
      if (typeof count === 'number' && Number.isFinite(count) && count >= 0) {
        return count
      }
    }
    return null
  } catch {
    return null
  }
}

/**
 * The stored read anchor (segment count) for a session. `null` = nothing
 * stored yet, or a legacy `seen_ts`-shaped value — both read as fully read.
 */
export function readAnchorCount(sessionKey: string): number | null {
  try {
    return parse(localStorage.getItem(storageKey(sessionKey)))
  } catch {
    return null
  }
}

function write(sessionKey: string, anchorCount: number): void {
  try {
    // wire 形状用 snake_case（与后端字段词表一致）；单字段——旧键上的
    // seen_ts 形态随本次写入被覆写消失。
    const stored: StoredAnchor = { anchor_count: anchorCount }
    localStorage.setItem(storageKey(sessionKey), JSON.stringify(stored))
    // （round3 6.1）写后广播：rail 等徽标面据此失效重渲染（详见事件常量注）。
    window.dispatchEvent(
      new CustomEvent(ANCHOR_ADVANCED_EVENT, { detail: { key: sessionKey } }),
    )
  } catch {
    /* storage may be disabled; degrade silently */
  }
}

/**
 * Advance the read anchor (transcript mark-as-seen path, streaming-while-
 * reading, manual read-to-bottom). 单调：更低的水位不回退已存的锚。所有
 * 写入调用点显式传当前可见回复段数（2.3：transcript 读到底、流式贴底推进
 * 与聚焦写锚三路写同一字段）。
 */
export function writeSeen(sessionKey: string, anchorCount: number): void {
  const prev = readAnchorCount(sessionKey)
  if (prev !== null && prev >= anchorCount) return
  write(sessionKey, anchorCount)
}

/**
 * Focus-time anchor write (rail switch success): everything up to the
 * session's current `msg_count` is read. 与 `writeSeen` 同一字段、同一单调
 * 语义（聚焦推进 = 流式推进 = 手动读到底，spec「anchors are independent
 * per browser」+「seam and badge share the anchor」）。
 */
export function writeFocusAnchor(sessionKey: string, msgCount: number): void {
  writeSeen(sessionKey, msgCount)
}

/**
 * The rail's unread number: `msg_count − anchor_count`, floored at 0. No
 * anchor（含旧 seen_ts 数据）= fully read，历史不冒未读。
 */
export function unreadCount(sessionKey: string, msgCount: number): number {
  const anchor = readAnchorCount(sessionKey)
  if (anchor === null) return 0
  return Math.max(0, msgCount - anchor)
}
