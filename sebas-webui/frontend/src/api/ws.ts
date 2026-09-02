/**
 * Single-connection WebSocket client for `/ws`, implemented as a Lit
 * reactive controller so any view can subscribe to live events.
 *
 * Contract (from the `webui-api` capability):
 * - every connected client receives every event;
 * - events are JSON objects tagged with a dotted `type`
 *   (session.created / session.updated / session.removed / config.updated /
 *   permission.requested);
 * - unknown event types must be tolerated (forward compatibility);
 * - reconnect with exponential backoff after a drop, and refetch the
 *   visible view's data afterwards (the `onReconnect` hook).
 */

import type { ReactiveController, ReactiveControllerHost } from 'lit'

export interface WsEvents {
  'session.created': { type: 'session.created'; session_id: string }
  'session.updated': { type: 'session.updated'; session_id: string; status: string }
  'session.removed': { type: 'session.removed'; session_id: string }
  'config.updated': { type: 'config.updated' }
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
}

export type WsEvent = WsEvents[keyof WsEvents]

/** Known event type names; anything else arriving is ignored. */
const EVENTS = {
  'session.created': true,
  'session.updated': true,
  'session.removed': true,
  'config.updated': true,
  'permission.requested': true,
}

export type WsEventHandler = (event: WsEvent) => void

export interface WsClientOptions {
  /** Called after a dropped connection has been re-established. */
  onReconnect?: () => void
  /** Base backoff in ms; doubles per failed attempt up to `maxBackoffMs`. */
  backoffMs?: number
  maxBackoffMs?: number
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
  private readonly socketFactory: (url: string) => WebSocket

  constructor(host: ReactiveControllerHost, options: WsClientOptions = {}) {
    this.backoffMs = options.backoffMs ?? 500
    this.maxBackoffMs = options.maxBackoffMs ?? 15_000
    this.onReconnect = options.onReconnect
    this.socketFactory = options.socketFactory ?? ((url) => new WebSocket(url))
    host.addController(this)
  }

  hostConnected(): void {
    this.connect()
  }

  hostDisconnected(): void {
    this.closedByUser = true
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
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

  private connect(): void {
    if (this.socket) return
    const proto = location.protocol === 'https:' ? 'wss://' : 'ws://'
    const socket = this.socketFactory(`${proto}${location.host}/ws`)
    this.socket = socket

    socket.onopen = () => {
      const wasRetry = this.attempts > 0
      this.attempts = 0
      if (wasRetry) this.onReconnect?.()
    }

    socket.onmessage = (msg) => {
      let event: unknown
      try {
        event = JSON.parse(String(msg.data))
      } catch {
        return // malformed frame: ignore, the channel is advisory
      }
      const type = (event as { type?: unknown })?.type
      // Only known types are dispatched; unknown types are tolerated
      // (ignored) so the server can add events without breaking clients.
      if (typeof type === 'string' && type in EVENTS) {
        for (const handler of this.handlers) handler(event as WsEvent)
      }
    }

    socket.onclose = () => {
      this.socket = null
      if (this.closedByUser) return
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
