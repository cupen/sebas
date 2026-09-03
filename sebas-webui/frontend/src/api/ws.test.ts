import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WsClient, type WsEvent } from './ws.js'

/** Minimal EventTarget-free host: WsClient only needs addController. */
function fakeHost() {
  return {
    addController: vi.fn(),
    requestUpdate: vi.fn(),
    updateComplete: Promise.resolve(true),
  } as never
}

/** Scriptable WebSocket double. */
class FakeSocket {
  static instances: FakeSocket[] = []
  static OPEN = 1
  static CLOSED = 3
  readyState = 1
  onopen: (() => void) | null = null
  onmessage: ((ev: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  closed = false
  constructor(public url: string) {
    FakeSocket.instances.push(this)
  }
  close(): void {
    this.closed = true
    this.readyState = 3
    this.onclose?.()
  }
  send(): void {}
  /** Test hooks. */
  open(): void {
    this.onopen?.()
  }
  emit(event: WsEvent | unknown): void {
    this.onmessage?.({ data: JSON.stringify(event) })
  }
  drop(): void {
    this.readyState = 3
    this.onclose?.()
  }
}

describe('WsClient', () => {
  beforeEach(() => {
    FakeSocket.instances = []
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  function makeClient(onReconnect?: () => void) {
    return new WsClient(fakeHost(), {
      backoffMs: 100,
      socketFactory: (url) => new FakeSocket(url) as unknown as WebSocket,
      onReconnect,
    })
  }

  it('opens exactly one socket on connect and dispatches tagged events', () => {
    const client = makeClient()
    client.hostConnected()
    expect(FakeSocket.instances).toHaveLength(1)
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    socket.emit({ type: 'session.created', session_id: 'oc_1' })
    expect(received).toEqual([{ type: 'session.created', session_id: 'oc_1' }])
  })

  it('tolerates unknown event types and malformed frames', () => {
    const client = makeClient()
    client.hostConnected()
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    // Unknown type: tolerated (ignored, not dispatched).
    socket.emit({ type: 'session.exploded' })
    socket.onmessage?.({ data: 'not-json' })
    socket.emit({ type: 'session.removed', session_id: 'oc_2' })
    expect(received).toEqual([{ type: 'session.removed', session_id: 'oc_2' }])
  })

  it('dispatches permission.requested frames with the full payload', () => {
    // Review-card feed (tasks 5.3): the frame mirrors the backend's
    // WebUiEvent::PermissionRequested — request_id == kernel tool_use_id,
    // session_id is the URL-safe encoded session key, args is verbatim.
    const client = makeClient()
    client.hostConnected()
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    const frame = {
      type: 'permission.requested',
      request_id: 'toolu_01ABC',
      session_id: 'oc_enc%00key',
      tool_name: 'bash',
      args: { command: 'rm -rf build' },
      reason: 'may modify state',
    }
    socket.emit(frame)
    expect(received).toEqual([frame])
    // The event table keeps tolerating unknown types alongside it.
    socket.emit({ type: 'session.exploded' })
    expect(received).toHaveLength(1)
  })

  it('reconnects with exponential backoff and fires onReconnect on success', async () => {
    const reconnected = vi.fn()
    const client = makeClient(reconnected)
    client.hostConnected()
    // First socket opens (no retry yet), then drops: backoff #1 = 100ms.
    FakeSocket.instances[0]!.open()
    FakeSocket.instances[0]!.drop()
    vi.advanceTimersByTime(100)
    expect(FakeSocket.instances).toHaveLength(2)
    // Second socket never opens, just drops: backoff #2 = 200ms.
    FakeSocket.instances[1]!.drop()
    vi.advanceTimersByTime(100) // insufficient for 200ms
    expect(FakeSocket.instances).toHaveLength(2)
    vi.advanceTimersByTime(100) // cumulative 200ms → third socket
    expect(FakeSocket.instances).toHaveLength(3)
    // Only a successful re-open counts as reconnected.
    FakeSocket.instances[2]!.open()
    expect(reconnected).toHaveBeenCalledTimes(1)
  })

  it('does not reconnect after explicit host disconnect', () => {
    const client = makeClient()
    client.hostConnected()
    client.hostDisconnected()
    vi.advanceTimersByTime(60_000)
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('an eager host (addController forwards hostConnected) connects at construction', () => {
    // Regression: the sharedWs shim originally used a no-op addController,
    // which never invoked hostConnected → connect() never ran and the app
    // had no live socket at all. The shim's contract is that constructing
    // the client through a lifecycle-forwarding host opens the socket.
    const host = {
      addController: (c: { hostConnected?: () => void }) => c.hostConnected?.(),
      requestUpdate: () => {},
      updateComplete: Promise.resolve(true),
    } as never
    new WsClient(host, {
      socketFactory: (url) => new FakeSocket(url) as unknown as WebSocket,
    })
    expect(FakeSocket.instances).toHaveLength(1)
    expect(FakeSocket.instances[0]!.url).toBe('ws://localhost:3000/ws')
  })
})
