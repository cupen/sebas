/**
 * Per-session read cursor shared by the rail's unread badge and the
 * transcript's seen-boundary seam (rail-declutter-unread D3).
 *
 * localStorage 里的每会话游标 `{ seen_ts, anchor_count }`：
 *   - `seen_ts` 是 transcript seen-boundary 的既有锚（unix 秒）——存储键与
 *     旧实现相同（`sebas:seen:<key>`），旧数据原地迁移，行为不变；
 *   - `anchor_count` 是「标记已读时的服务端可见回复段数」——rail 徽标 =
 *     `msg_count − anchor_count`。徽标（按段）与 seam（按轮）聚合粒度不同，
 *     属有意为之，但两者共用同一游标，永不互相矛盾。
 *
 * 无锚点（首次访问 / 清缓存 / 旧数据只有时间戳）= 全部已读：历史会话不因
 * 换浏览器而集体冒红点。写入取单调 max（seen 与 anchor 各自只进不退），
 * 多标签页同源共享同一份 localStorage。
 *
 * 旧格式迁移：旧实现把 seen 时间戳存成裸数字。读到裸数字 →
 * `{ seen_ts: n, anchor_count: null }`——无法重构当时的段数，按已读处理，
 * 下一次聚焦/标记已读时锚自然补全。
 */
/** One session's stored read anchor. `anchorCount: null` = legacy/no count yet. */
export interface SeenAnchor {
  seenTs: number
  anchorCount: number | null
}

function storageKey(sessionKey: string): string {
  return `sebas:seen:${sessionKey}`
}

function parse(raw: string | null): SeenAnchor | null {
  if (raw === null) return null
  try {
    const v = JSON.parse(raw) as unknown
    if (typeof v === 'number') {
      // 旧格式：裸 seen 时间戳。段数不可重构 → null（按已读，待补锚）。
      return Number.isFinite(v) ? { seenTs: v, anchorCount: null } : null
    }
    if (v !== null && typeof v === 'object') {
      const o = v as { seen_ts?: unknown; anchor_count?: unknown }
      const seenTs = typeof o.seen_ts === 'number' && Number.isFinite(o.seen_ts) ? o.seen_ts : 0
      const anchorCount =
        typeof o.anchor_count === 'number' && Number.isFinite(o.anchor_count)
          ? o.anchor_count
          : null
      return { seenTs, anchorCount }
    }
    return null
  } catch {
    return null
  }
}

/** The stored anchor for a session, or null when nothing has ever been written. */
export function readAnchor(sessionKey: string): SeenAnchor | null {
  try {
    return parse(localStorage.getItem(storageKey(sessionKey)))
  } catch {
    return null
  }
}

function write(sessionKey: string, anchor: SeenAnchor): void {
  try {
    // wire 形状用 snake_case（与后端字段词表一致）。
    localStorage.setItem(
      storageKey(sessionKey),
      JSON.stringify({ seen_ts: anchor.seenTs, anchor_count: anchor.anchorCount }),
    )
  } catch {
    /* storage may be disabled; degrade silently */
  }
}

/**
 * Advance the seen boundary (transcript mark-as-seen path). Both fields are
 * monotonic: a lower value never regresses a stored one. Passing
 * `anchorCount` (the session's current visible-segment count) also advances
 * the badge anchor so the rail stays in agreement with the seam; omit it to
 * touch only the timestamp.
 */
export function writeSeen(sessionKey: string, seenTs: number, anchorCount?: number): void {
  const prev = readAnchor(sessionKey)
  const next: SeenAnchor = {
    seenTs: Math.max(prev?.seenTs ?? 0, seenTs),
    anchorCount:
      anchorCount === undefined
        ? (prev?.anchorCount ?? null)
        : Math.max(prev?.anchorCount ?? 0, anchorCount),
  }
  write(sessionKey, next)
}

/**
 * Focus-time anchor write (rail switch success): everything up to the
 * session's current `msg_count` is read. `seenTs` moves to "now" so the
 * transcript's seam also treats older content as seen.
 */
export function writeFocusAnchor(sessionKey: string, msgCount: number): void {
  const prev = readAnchor(sessionKey)
  write(sessionKey, {
    seenTs: Math.max(prev?.seenTs ?? 0, Math.floor(Date.now() / 1000)),
    anchorCount: Math.max(prev?.anchorCount ?? 0, msgCount),
  })
}

/**
 * The rail's unread number: `msg_count − anchor_count`, floored at 0. No
 * anchor (or a legacy timestamp-only anchor) counts as fully read.
 */
export function unreadCount(sessionKey: string, msgCount: number): number {
  const anchor = readAnchor(sessionKey)
  if (!anchor || anchor.anchorCount === null) return 0
  return Math.max(0, msgCount - anchor.anchorCount)
}
