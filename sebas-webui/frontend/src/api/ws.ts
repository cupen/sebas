/**
 * Single-connection WebSocket client for `/ws`, implemented as a Lit
 * reactive controller so any view can subscribe to live events.
 *
 * Contract (add-ws-rpc-protocol — three-frame envelope, JSON codec first):
 * - every application-level frame is one of Request{id, method, params} /
 *   Response{id, result | error} / Notification{method, params} (snake_case
 *   fields, no other protocol-level fields); the codec seam below is the
 *   single place that touches the byte representation;
 * - server events arrive as Notifications whose `method` is the legacy
 *   dotted type (session.created / session.updated / session.removed /
 *   session.pending_dropped / session.turn_stalled / session.resync /
 *   config.updated / permission.requested / turn.append / core.reachability)
 *   and whose `params` is the legacy payload; subscribers receive the
 *   reconstituted `{type, ...payload}` event, so dispatch keys are unchanged;
 * - unknown methods are tolerated (ignored — forward compatibility);
 * - `request(method, params)` correlates the matching-id Response:
 *   10s timeout, immediate `not_connected` rejection when the socket is
 *   down (no queueing — reconnect convergence rides the existing refetch
 *   path), and every in-flight request is rejected on close;
 * - reconnect with exponential backoff after a drop, and refetch the
 *   visible view's data afterwards (the `onReconnect` hook).
 */

import type { ReactiveController, ReactiveControllerHost } from 'lit'
import type { PendingSubmission } from './client.js'

/**
 * （session-parallel-liveness-and-unread-polish 2.1/2.2，design D2）会话相位
 * 帧载荷：`session.created` 与 `session.updated` 同形，五键每次帧必带——
 * 旧 `status` 人读字符串已删除，无「只在 true 时上 wire」的兼容保留。rail
 * 圆点/徽标、composer 提交控件、待执行栈全部从这一份帧事实真读，不做任何
 * `status_slug === 'working'` 之类的字符串等值回退，也不等 HTTP 详情轮询。
 */
export interface SessionPhaseFrame {
  /** 七词相位 starting|queued|working|done|failed|waiting|dormant。 */
  status_slug: string
  /** 回合占用（WORKING ∨ 泊车 ∨ spawn 窗口）的引擎事实，每帧必带。 */
  turn_engaged: boolean
  /** 可见回复段数（rail 徽标数据源），每帧必带。 */
  msg_count: number
  /** 待生效提交全量（投递序），每帧必带。 */
  pending: PendingSubmission[]
}

export interface WsEvents {
  'session.created': SessionPhaseFrame & {
    type: 'session.created'
    session_id: string
  }
  'session.updated': SessionPhaseFrame & {
    type: 'session.updated'
    session_id: string
  }
  'session.removed': { type: 'session.removed'; session_id: string }
  /**
   * （workbench-turn-queue 5.2/7.3）会话终结时未执行的待生效提交，逐条
   * 列出。在 session.removed 帧之前到达；前端据此渲染一次性「未执行」
   * 提示。
   */
  'session.pending_dropped': {
    type: 'session.pending_dropped'
    session_id: string
    dropped: PendingSubmission[]
  }
  /**
   * （fix-pending-queue-liveness 2.2）某会话的停滞回合被看门狗强制收尾、
   * 队列已解除卡死。`released` = 释放的搁浅待执行提交条数。shell 据此弹
   * warn 分级通知（点名会话与条目数）。
   */
  'session.turn_stalled': {
    type: 'session.turn_stalled'
    session_id: string
    released: number
  }
  /**
   * （fix-webui-streaming-liveness 5.4/5.1，D6）重新同步信号：服务端检测到
   * 本连接的订阅落后（broadcast Lagged）或 core 通道重连给出快照重取信号时
   * 推送。无载荷——消费端清本地增量游标与流式缓冲后全量重取，不得因陈旧
   * 游标永久拒收增量。
   */
  'session.resync': { type: 'session.resync' }
  'config.updated': { type: 'config.updated' }
  /**
   * （workbench-live-conversation-flow 2.2）实时回合内容：同一合并窗内某
   * 会话追加的 transcript 条目（落库序、position 单调）。`seq` 是本帧最后
   * 一条的 position（去重锚）。纯增量补充——乱序/迟到/重复以快照重取收敛：
   * position ≤ 已知最大值 = 已见过，跳过。思考/工具条目不计未读（与
   * session-unread-badge 的计数口径一致）。
   */
  'turn.append': {
    type: 'turn.append'
    session_id: string
    entries: import('./client.js').ConversationEntryView[]
    seq: number
  }
  /**
   * A gated tool call awaits an operator decision (the review card).
   * `session_id` is the URL-safe encoded session key; `request_id` equals
   * the kernel's tool_use_id and is what `api.answerPermission` takes
   * back. `args` is the call's arguments verbatim (arbitrary JSON).
   */
  'permission.requested': {
    type: 'permission.requested'
    request_id: string
    session_id: string
    tool_name: string
    args: unknown
    reason: string
  }
  /**
   * （add-core-reachability-ws-push D3/D5）核心可达性翻转推送。params 与
   * `/api/summary` 的 `reachability` 段同形：可达只有 `{ok:true}`；不可达
   * 携带机器可读 `kind`（startup_failed | auth_rejected | disconnected）与
   * 原文 cause。横幅/提交门据此更新，kind 分文案、不靠 cause 字符串匹配。
   */
  'core.reachability': {
    type: 'core.reachability'
    ok: boolean
    kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'
    cause?: string
  }
}

export type WsEvent = WsEvents[keyof WsEvents]

/**
 * （add-core-reachability-ws-push D4）结构化核心可达性状态：`core.reachability`
 * 推送与 `core.reachability.get` 响应归一后的形状（`type` 标签不入状态）。
 * app-shell 独占持有，横幅渲染与 composer 提交门（下传）同源消费。
 */
export interface CoreReachabilityState {
  ok: boolean
  kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'
  cause?: string
}

/** Known event type names; anything else arriving is ignored. */
const EVENTS = {
  'session.created': true,
  'session.updated': true,
  'session.removed': true,
  'session.pending_dropped': true,
  'session.turn_stalled': true,
  'session.resync': true,
  'config.updated': true,
  'permission.requested': true,
  'turn.append': true,
  'core.reachability': true,
}

export type WsEventHandler = (event: WsEvent) => void

// ---- Protocol layer (add-ws-rpc-protocol) --------------------------------

/** The `error` side of a Response: stable machine `code`, operator-facing
 * `message`. Mirrors the Rust `ws_rpc::RpcError`. */
export interface WsRpcErrorShape {
  code: string
  message: string
}

/** The three envelope frames in memory. `kind` is this side's discriminator
 * only — the wire carries no extra field (shapes are told apart by their
 * fields: id+method vs id+result/error vs bare method). */
export type WsFrame =
  | { kind: 'request'; id: number; method: string; params?: unknown }
  | { kind: 'response'; id: number; result?: unknown; error?: WsRpcErrorShape }
  | { kind: 'notification'; method: string; params?: unknown }

/** Rejection shape for `request()` failures: timeout, not_connected,
 * disconnect mid-flight, or the server's error Response verbatim. */
export class WsRpcError extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'WsRpcError'
  }
}

/**
 * Codec seam (design D2): symmetric to the Rust `WsCodec` trait. Swapping
 * the implementation changes only the byte representation — frame
 * semantics, id correlation, and dispatch behavior stay put.
 */
export interface WsFrameCodec {
  encode(frame: WsFrame): string
  /** null = malformed or not one of the three shapes: callers ignore. */
  decode(raw: string): WsFrame | null
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function asRpcError(v: unknown): WsRpcErrorShape | undefined {
  if (isRecord(v) && typeof v.code === 'string' && typeof v.message === 'string') {
    return { code: v.code, message: v.message }
  }
  return undefined
}

/** JSON first implementation: snake_case fields; decode tolerates unknown
 * fields (forward compatibility) and returns null for anything that is not
 * one of the three frame shapes. */
export const jsonFrameCodec: WsFrameCodec = {
  encode(frame) {
    if (frame.kind === 'request') {
      return JSON.stringify({ id: frame.id, method: frame.method, params: frame.params ?? {} })
    }
    if (frame.kind === 'response') {
      // result 与 error 互斥：出错只带 error（构造侧保证，同 Rust 侧）。
      const raw: Record<string, unknown> = { id: frame.id }
      if (frame.error !== undefined) raw.error = frame.error
      else raw.result = frame.result
      return JSON.stringify(raw)
    }
    return JSON.stringify({ method: frame.method, params: frame.params ?? {} })
  },
  decode(raw) {
    let v: unknown
    try {
      v = JSON.parse(raw)
    } catch {
      return null // malformed frame: ignore, the channel is advisory
    }
    if (!isRecord(v)) return null
    const { id, method, params } = v
    if (typeof id === 'number' && Number.isFinite(id)) {
      if (typeof method === 'string') return { kind: 'request', id, method, params }
      if ('result' in v || 'error' in v) {
        return { kind: 'response', id, result: v.result, error: asRpcError(v.error) }
      }
      return null
    }
    if (typeof method === 'string') return { kind: 'notification', method, params }
    return null
  },
}

// ---- Client ---------------------------------------------------------------

interface PendingRequest {
  resolve: (result: unknown) => void
  reject: (err: WsRpcError) => void
  timer: ReturnType<typeof setTimeout>
}

export interface WsClientOptions {
  /** Called after a dropped connection has been re-established. */
  onReconnect?: () => void
  /**
   * Connection-state transitions (add-webui-allowed-roots D6): `true` on
   * open, `false` on an unintended close (user-driven closes are silent).
   * The shell listens to render the global disconnect banner.
   */
  onStateChange?: (connected: boolean) => void
  /** Base backoff in ms; doubles per failed attempt up to `maxBackoffMs`. */
  backoffMs?: number
  maxBackoffMs?: number
  /** Frame codec (add-ws-rpc-protocol D2/D5); defaults to the JSON codec. */
  codec?: WsFrameCodec
  /** `request()` timeout window in ms; default 10s (design D5). */
  requestTimeoutMs?: number
  /** Overridable for tests. */
  socketFactory?: (url: string) => WebSocket
}

export class WsClient implements ReactiveController {
  private handlers = new Set<WsEventHandler>()
  private socket: WebSocket | null = null
  private attempts = 0
  private backoffMs: number
  private maxBackoffMs: number
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private closedByUser = false
  private readonly onReconnect?: () => void
  private readonly onStateChange?: (connected: boolean) => void
  private readonly socketFactory: (url: string) => WebSocket
  private readonly codec: WsFrameCodec
  private readonly requestTimeoutMs: number
  /** In-flight `request()`s by id; rejected on timeout / close. */
  private pending = new Map<number, PendingRequest>()
  private nextId = 1

  constructor(host: ReactiveControllerHost, options: WsClientOptions = {}) {
    this.backoffMs = options.backoffMs ?? 500
    this.maxBackoffMs = options.maxBackoffMs ?? 15_000
    this.onReconnect = options.onReconnect
    this.onStateChange = options.onStateChange
    this.socketFactory = options.socketFactory ?? ((url) => new WebSocket(url))
    this.codec = options.codec ?? jsonFrameCodec
    this.requestTimeoutMs = options.requestTimeoutMs ?? 10_000
    host.addController(this)
  }

  hostConnected(): void {
    this.connect()
  }

  hostDisconnected(): void {
    this.closedByUser = true
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    this.failPending('disconnected', 'client disconnected with requests in flight')
    this.socket?.close()
    this.socket = null
  }

  subscribe(handler: WsEventHandler): () => void {
    this.handlers.add(handler)
    return () => this.handlers.delete(handler)
  }

  get connected(): boolean {
    return this.socket?.readyState === WebSocket.OPEN
  }

  /**
   * add-ws-rpc-protocol D5: send a Request and complete with the
   * matching-id Response's `result`. Honest rejection semantics: no
   * queueing while disconnected (`not_connected`), a 10s timeout window,
   * and batch rejection of every in-flight request when the socket closes.
   * Responses may arrive in any order — correlation is by id.
   */
  request(method: string, params?: unknown): Promise<unknown> {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(
        new WsRpcError('not_connected', `not connected: cannot call ${method}`),
      )
    }
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(
          new WsRpcError(
            'timeout',
            `request ${method} (id ${id}) timed out after ${this.requestTimeoutMs}ms`,
          ),
        )
      }, this.requestTimeoutMs)
      this.pending.set(id, { resolve, reject, timer })
      try {
        socket.send(this.codec.encode({ kind: 'request', id, method, params }))
      } catch {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(new WsRpcError('not_connected', `send failed: cannot call ${method}`))
      }
    })
  }

  /** Reject and drop every in-flight request (timeout entries included). */
  private failPending(code: string, message: string): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer)
      pending.reject(new WsRpcError(code, message))
    }
    this.pending.clear()
  }

  private connect(): void {
    if (this.socket) return
    const proto = location.protocol === 'https:' ? 'wss://' : 'ws://'
    const socket = this.socketFactory(`${proto}${location.host}/ws`)
    this.socket = socket

    socket.onopen = () => {
      const wasRetry = this.attempts > 0
      this.attempts = 0
      this.onStateChange?.(true)
      if (wasRetry) this.onReconnect?.()
    }

    socket.onmessage = (msg) => {
      const frame = this.codec.decode(String(msg.data))
      if (!frame) return // malformed / non-frame: ignore, the channel is advisory
      if (frame.kind === 'notification') {
        // D4：method 当作既有 type 分发——订阅 handler 集合与分发 key 不变，
        // 各视图零改动。未知 method 容忍（忽略）。
        if (!(frame.method in EVENTS)) return
        const params = frame.params
        const event = {
          type: frame.method,
          ...(isRecord(params) ? params : {}),
        } as WsEvent
        for (const handler of this.handlers) handler(event)
        return
      }
      if (frame.kind === 'response') {
        // 迟到（已超时）或未知 id 的 Response 直接忽略。
        const pending = this.pending.get(frame.id)
        if (!pending) return
        this.pending.delete(frame.id)
        clearTimeout(pending.timer)
        if (frame.error) pending.reject(new WsRpcError(frame.error.code, frame.error.message))
        else pending.resolve(frame.result)
      }
      // Requests server → client are not part of the protocol: ignored.
    }

    socket.onclose = () => {
      this.socket = null
      // D5：断线批量拒付在途请求——诚实优于悬挂。
      this.failPending('disconnected', 'connection lost with requests in flight')
      if (this.closedByUser) return
      this.onStateChange?.(false)
      this.scheduleReconnect()
    }

    socket.onerror = () => {
      // onclose follows; nothing to do here beyond avoiding unhandled logs.
    }
  }

  private scheduleReconnect(): void {
    const delay = Math.min(this.backoffMs * 2 ** this.attempts, this.maxBackoffMs)
    this.attempts += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this.connect()
    }, delay)
  }
}
