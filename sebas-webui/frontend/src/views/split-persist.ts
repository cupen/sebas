/**
 * Split-boundary persistence (workbench-interaction-polish 5.1/5.2, D1).
 *
 * Both draggable boundaries remember their pixel size in localStorage:
 * `sebas.rail-width` (rail|main, 180–480px) and `sebas.composer-height`
 * (stage|composer, min 120px, max half the main area). Every access is
 * defensive — a browser in privacy mode, an opaque-origin document, or any
 * throwing storage degrades to the defaults instead of breaking layout.
 * wa-split-panel positions are percentages while the boundaries are
 * pixel-clamped, so the caller converts: position = px / measured size × 100.
 */

/** 侧栏|主区分割的 localStorage 键（proposal D1 定名）。 */
export const RAIL_WIDTH_KEY = 'sebas.rail-width'
/** 会话流|输入框分割的 localStorage 键。 */
export const COMPOSER_HEIGHT_KEY = 'sebas.composer-height'

/** 侧栏宽度边界（px）。 */
export const RAIL_MIN_PX = 180
export const RAIL_MAX_PX = 480
/** 输入框最低高度（px）；上限是主区一半（按当时量得的高度算）。 */
export const COMPOSER_MIN_PX = 120

/** 侧栏默认宽度（既有 220px 定宽的延续）。 */
export const RAIL_DEFAULT_PX = 220
/** 输入框默认高度（px）。 */
export const COMPOSER_DEFAULT_PX = 220

let probedStorage: Storage | null | undefined

/**
 * The real storage when usable, `null` otherwise (privacy mode, jsdom's
 * opaque origin, access throwing). Probed once — the result is memoized.
 */
export function defaultStorage(): Storage | null {
  if (probedStorage !== undefined) return probedStorage
  try {
    const s = window.localStorage
    if (!s) {
      probedStorage = null
      return null
    }
    const probe = '__sebas_split_probe__'
    s.setItem(probe, probe)
    s.removeItem(probe)
    probedStorage = s
  } catch {
    probedStorage = null
  }
  return probedStorage
}

function rawGet(store: Storage | null, key: string): string | null {
  try {
    return store?.getItem(key) ?? null
  } catch {
    return null
  }
}

function rawSet(store: Storage | null, key: string, value: string): void {
  try {
    store?.setItem(key, value)
  } catch {
    // 写不进去就放弃——布局不受影响。
  }
}

/** Clamp a rail width into [180, 480]; non-finite falls back to the default. */
export function clampRailWidth(px: number): number {
  if (!Number.isFinite(px)) return RAIL_DEFAULT_PX
  return Math.min(RAIL_MAX_PX, Math.max(RAIL_MIN_PX, Math.round(px)))
}

/**
 * Clamp a composer height into [120, areaHeight/2]. `areaHeight` is the
 * measured stage+composer area at clamp time; a non-positive measurement
 * degrades to the 120px floor only.
 */
export function clampComposerHeight(px: number, areaHeight: number): number {
  if (!Number.isFinite(px)) return COMPOSER_MIN_PX
  const max = areaHeight > 0 ? Math.floor(areaHeight / 2) : COMPOSER_MIN_PX
  return Math.min(Math.max(COMPOSER_MIN_PX, max), Math.max(COMPOSER_MIN_PX, Math.round(px)))
}

function parseClamped(raw: string | null, clamp: (n: number) => number): number | null {
  if (raw === null) return null
  const px = Number(raw)
  if (!Number.isFinite(px)) return null
  return clamp(px)
}

/** Read the persisted rail width (clamped); `null` = nothing sane stored. */
export function loadRailWidth(store: Storage | null = defaultStorage()): number | null {
  return parseClamped(rawGet(store, RAIL_WIDTH_KEY), clampRailWidth)
}

/** Persist the rail width (clamped here; the drag handler clamps earlier too). */
export function saveRailWidth(px: number, store: Storage | null = defaultStorage()): void {
  rawSet(store, RAIL_WIDTH_KEY, String(clampRailWidth(px)))
}

/** Read the persisted composer height (floor-clamped); `null` = nothing sane stored. */
export function loadComposerHeight(store: Storage | null = defaultStorage()): number | null {
  return parseClamped(rawGet(store, COMPOSER_HEIGHT_KEY), (n) =>
    Math.max(COMPOSER_MIN_PX, Math.round(n)),
  )
}

/** Persist the composer height; `areaHeight` bounds it to half the area. */
export function saveComposerHeight(
  px: number,
  areaHeight: number,
  store: Storage | null = defaultStorage(),
): void {
  rawSet(store, COMPOSER_HEIGHT_KEY, String(clampComposerHeight(px, areaHeight)))
}

/**
 * 窄屏断点（proposal：<640px 退化为现状，分割线不可拖）。
 * 与 app-shell 既有 640px 媒体查询同一阈值。
 */
export const NARROW_BREAKPOINT_PX = 640

export function isNarrowViewport(): boolean {
  // jsdom 无 matchMedia：按桌面（可拖拽）处理。
  return window.matchMedia?.(`(max-width: ${NARROW_BREAKPOINT_PX}px)`).matches ?? false
}

/** Subscribe to narrow-viewport flips; returns an unsubscribe function. */
export function onNarrowChange(cb: (narrow: boolean) => void): () => void {
  const mq = window.matchMedia?.(`(max-width: ${NARROW_BREAKPOINT_PX}px)`)
  if (!mq) return () => {}
  const handler = (e: MediaQueryListEvent): void => cb(e.matches)
  mq.addEventListener('change', handler)
  return () => mq.removeEventListener('change', handler)
}
