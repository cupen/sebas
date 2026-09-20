/**
 * 分级通知层的模块级状态 store（add-webui-tiered-notices D4）。
 *
 * 轻量订阅广播，同 `sebas:ws-state` / `sebas:refetch` 的事件风格——但不走
 * window CustomEvent，而是回调订阅（组件挂载即回放当前态，避免丢首帧）。
 * 三类内容：
 *
 * - `notify()` 瞬时 toast（info / warn / error）：栈上限 3 条瞬时条，超出挤
 *   掉最旧（显式 `duration=0` 的驻留条不参与挤占）；同一文案（或同一
 *   `dedupeKey`）在 8s 去重窗口内不重复弹出。error 默认 8s 自动消失
 *   （fix-webui-qa-defects-round5 4.2）。
 * - `setWsDown()` 持久槽位：`/ws` 断线 → 持续 warn 驻留横幅（重连即消）。
 * - `setFatal()` 持久槽位：core 不可达 → fatal 驻留横幅 + 锁定遮罩（恢复由
 *   调用方置 null，并自行弹「核心已恢复」info）。
 *
 * 消费方只有 `sebas-notice-layer`（订阅 + 渲染）；生产方是 `api/client.ts`
 * 拦截器、app-shell（可达性 / WS 状态）与视图显式上报。
 */

export type NoticeLevel = 'info' | 'warn' | 'error'

/** core 不可达的机器可读分档（与 `CoreReachabilityState.kind` 同词表）。 */
export type FatalKind = 'startup_failed' | 'auth_rejected' | 'disconnected'

export interface NoticeInput {
  level: NoticeLevel
  message: string
  /** 覆盖该级默认时长（ms）；0 = 驻留至手动关闭。省略按级别默认。 */
  duration?: number
  /** 去重键；省略用 message 本身。同一键在窗口内只弹第一条。 */
  dedupeKey?: string
}

export interface NoticeItem {
  id: number
  level: NoticeLevel
  message: string
  /** 0 = 驻留（须手动关闭）；>0 = 自动消失的瞬时条。 */
  duration: number
  dedupeKey: string
  /** 入栈时刻（Date.now()），去重窗口的锚点。 */
  at: number
}

/** fatal 槽位载荷（kind 缺失 = 退化为通用「核心不可达」文案）。 */
export interface FatalNotice {
  kind?: FatalKind
  cause?: string
}

export interface NoticeState {
  /** 瞬时 + 驻留 toast 条目（渲染为 wa-toast-item）。 */
  items: NoticeItem[]
  /** `/ws` 断线中（持续 warn 驻留横幅槽位）。 */
  wsDown: boolean
  /** core 不可达（fatal 驻留横幅 + 锁定遮罩槽位）；null = 可达/未知。 */
  fatal: FatalNotice | null
}

/**
 * （fix-webui-qa-defects-round5 4.2）失败类 toast 的自动消失时长（8s）——
 * 与成功类（info 5s）策略分开常量化。失败不再驻留：异常态下层层驻留 toast
 * 会盖住工作台，操作者重试后新反馈自然替换；持续型故障由 fatal 横幅槽位
 * （setFatal）承载，不依赖 error toast 驻留。
 */
export const ERROR_TOAST_DURATION_MS = 8_000

/** 各级默认时长（D2 映射表）：info 5s、warn 8s、error 8s（4.2 起自动消失）。 */
export const DEFAULT_DURATIONS: Record<NoticeLevel, number> = {
  info: 5_000,
  warn: 8_000,
  error: ERROR_TOAST_DURATION_MS,
}

/** 同文案去重窗口（design 共识值 8s）。 */
export const DEDUPE_WINDOW_MS = 8_000

/** 瞬时条栈上限（4.2 起 error 默认也是瞬时条，同样参与挤占）。 */
export const MAX_TRANSIENT_ITEMS = 3

let state: NoticeState = { items: [], wsDown: false, fatal: null }
const listeners = new Set<(state: NoticeState) => void>()
let nextId = 1

function emit(): void {
  for (const listener of listeners) listener(state)
}

/**
 * 订阅状态广播。订阅即回放当前态（挂载即对齐，无需另取 getter）；返回
 * 退订函数。
 */
export function subscribeNotices(listener: (state: NoticeState) => void): () => void {
  listeners.add(listener)
  listener(state)
  return () => {
    listeners.delete(listener)
  }
}

/**
 * 入栈一条通知。去重窗口内同文案（或同 dedupeKey）静默丢弃；瞬时条超上限
 * 挤掉最旧（显式 duration=0 的驻留条不参与挤占）。返回条目 id；被去重丢弃
 * 时返回 null。
 */
export function notify(input: NoticeInput): number | null {
  const dedupeKey = input.dedupeKey ?? input.message
  const now = Date.now()
  if (state.items.some((it) => it.dedupeKey === dedupeKey && now - it.at < DEDUPE_WINDOW_MS)) {
    return null
  }
  const item: NoticeItem = {
    id: nextId++,
    level: input.level,
    message: input.message,
    duration: input.duration ?? DEFAULT_DURATIONS[input.level],
    dedupeKey,
    at: now,
  }
  let items = [...state.items, item]
  if (item.duration > 0) {
    // 瞬时条（duration>0）挤占：超出上限移除最旧；显式驻留条（duration=0）
    // 永不被挤。
    const transient = items.filter((it) => it.duration > 0)
    while (transient.length > MAX_TRANSIENT_ITEMS) {
      const oldest = transient.shift()
      if (!oldest) break
      items = items.filter((it) => it.id !== oldest.id)
    }
  }
  state = { ...state, items }
  emit()
  return item.id
}

/** 移除一条通知（驻留条手动关闭 / 层内条目消失后的回写）。 */
export function dismiss(id: number): void {
  if (!state.items.some((it) => it.id === id)) return
  state = { ...state, items: state.items.filter((it) => it.id !== id) }
  emit()
}

/** `/ws` 断线槽位（持续 warn 驻留横幅；重连即消）。 */
export function setWsDown(down: boolean): void {
  if (state.wsDown === down) return
  state = { ...state, wsDown: down }
  emit()
}

/** fatal 槽位（core 不可达；null = 恢复/未知，横幅与锁定随之消失）。 */
export function setFatal(fatal: FatalNotice | null): void {
  state = { ...state, fatal: fatal ? { ...fatal } : null }
  emit()
}

/**
 * 仅测试用：清空全部状态与 id 计数（模块级 store 在 vitest 用例间需要隔离，
 * 且用例对 id 有确定性断言）。
 */
export function resetNotices(): void {
  state = { items: [], wsDown: false, fatal: null }
  nextId = 1
  emit()
}
