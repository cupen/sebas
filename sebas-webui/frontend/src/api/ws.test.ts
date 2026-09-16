import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WsClient, WsRpcError, jsonFrameCodec, type WsClientOptions, type WsEvent, type WsFrameCodec } from './ws.js'

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
  static CONNECTING = 0
  static OPEN = 1
  static CLOSED = 3
  readyState = FakeSocket.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((ev: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  closed = false
  /** Everything the client sent, in order (raw wire strings). */
  sent: string[] = []
  constructor(public url: string) {
    FakeSocket.instances.push(this)
  }
  close(): void {
    this.closed = true
    this.readyState = 3
    this.onclose?.()
  }
  send(data: string): void {
    this.sent.push(data)
  }
  /** Test hooks. */
  open(): void {
    this.readyState = FakeSocket.OPEN
    this.onopen?.()
  }
  /** Emit a Notification envelope — what the server now sends. */
  emit(method: string, params: Record<string, unknown> = {}): void {
    this.onmessage?.({ data: JSON.stringify({ method, params }) })
  }
  /** Emit a raw wire payload verbatim (malformed / legacy shapes). */
  emitRaw(data: string): void {
    this.onmessage?.({ data })
  }
  /** Reply to a request with a Response envelope (result or error). */
  reply(id: number, body: { result?: unknown; error?: { code: string; message: string } }): void {
    this.onmessage?.({ data: JSON.stringify({ id, ...body }) })
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

  function makeClient(options: Partial<WsClientOptions> = {}) {
    return new WsClient(fakeHost(), {
      backoffMs: 100,
      socketFactory: (url) => new FakeSocket(url) as unknown as WebSocket,
      ...options,
    })
  }

  it('opens exactly one socket on connect and dispatches notification envelopes as typed events', () => {
    const client = makeClient()
    client.hostConnected()
    expect(FakeSocket.instances).toHaveLength(1)
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    // Server pushes a Notification envelope; the handler still receives the
    // reconstituted `{type, ...payload}` event (dispatch key unchanged).
    socket.emit('session.created', { session_id: 'oc_1' })
    expect(received).toEqual([{ type: 'session.created', session_id: 'oc_1' }])
  })

  it('tolerates unknown methods, malformed frames and legacy bare frames', () => {
    const client = makeClient()
    client.hostConnected()
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    // Unknown method: tolerated (ignored, not dispatched).
    socket.emit('session.exploded')
    // Malformed payloads: ignored.
    socket.emitRaw('not-json')
    socket.emitRaw('42')
    socket.emitRaw('{}')
    // A legacy bare {type, ...} frame no longer matches any envelope shape.
    socket.emitRaw(JSON.stringify({ type: 'session.created', session_id: 'oc_legacy' }))
    // Then a known method still arrives.
    socket.emit('session.removed', { session_id: 'oc_2' })
    expect(received).toEqual([{ type: 'session.removed', session_id: 'oc_2' }])
  })

  it('dispatches permission.requested envelopes with the full payload', () => {
    // Review-card feed: the params mirror the backend's WebUiEvent payload —
    // request_id == kernel tool_use_id, session_id is the URL-safe encoded
    // session key, args is verbatim.
    const client = makeClient()
    client.hostConnected()
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    const params = {
      request_id: 'toolu_01ABC',
      session_id: 'oc_enc%00key',
      tool_name: 'bash',
      args: { command: 'rm -rf build' },
      reason: 'may modify state',
    }
    socket.emit('permission.requested', params)
    expect(received).toEqual([{ type: 'permission.requested', ...params }])
    // The method table keeps tolerating unknown methods alongside it.
    socket.emit('session.exploded')
    expect(received).toHaveLength(1)
  })

  it('request() correlates the matching-id response (ping self-proof)', async () => {
    const client = makeClient()
    client.hostConnected()
    const socket = FakeSocket.instances[0]!
    socket.open()
    const promise = client.request('ping')
    // Ids auto-increment from 1 within the client; wire shape is the
    // Request envelope.
    expect(socket.sent).toEqual([JSON.stringify({ id: 1, method: 'ping', params: {} })])
    socket.reply(1, { result: 'pong' })
    await expect(promise).resolves.toBe('pong')
  })

  it('resolves out-of-order responses to their own request', async () => {
    const client = makeClient()
    client.hostConnected()
    const socket = FakeSocket.instances[0]!
    socket.open()
    const first = client.request('slow')
    const second = client.request('fast')
    expect(JSON.parse(socket.sent[0]!)).toMatchObject({ id: 1, method: 'slow' })
    expect(JSON.parse(socket.sent[1]!)).toMatchObject({ id: 2, method: 'fast' })
    // The second request's Response arrives first; each completes with its
    // own result.
    socket.reply(2, { result: 'second-result' })
    socket.reply(1, { result: 'first-result' })
    await expect(second).resolves.toBe('second-result')
    await expect(first).resolves.toBe('first-result')
  })

  it('a swapped codec changes the bytes only — dispatch and id correlation stay put', async () => {
    // add-ws-rpc-protocol spec「换 codec 不改帧语义」：第二个实现换成
    // base64(JSON)——同三帧、不同线上字节。分发行为与 id 关联必须原样。
    const base64FrameCodec: WsFrameCodec = {
      encode: (frame) => btoa(jsonFrameCodec.encode(frame)),
      decode: (raw) => {
        try {
          return jsonFrameCodec.decode(atob(raw))
        } catch {
          return null
        }
      },
    }
    const client = makeClient({ codec: base64FrameCodec })
    client.hostConnected()
    const received: WsEvent[] = []
    client.subscribe((e) => received.push(e))
    const socket = FakeSocket.instances[0]!
    socket.open()
    // Notification 经换上的 codec 解封后仍重建出原 {type, ...params} 事件
    // ——分发 key 不变（params 字段名保真，view 层契约）。
    socket.emitRaw(
      btoa(
        JSON.stringify({
          method: 'turn.append',
          params: { session_id: 'oc_a', entries: [{ position: 3 }], seq: 3 },
        }),
      ),
    )
    expect(received).toEqual([
      { type: 'turn.append', session_id: 'oc_a', entries: [{ position: 3 }], seq: 3 },
    ])
    // 线上字节是 base64，不再是 JSON——变的只有字节表示。
    const promise = client.request('ping')
    expect(socket.sent).toHaveLength(1)
    expect(() => JSON.parse(socket.sent[0]!)).toThrow()
    expect(atob(socket.sent[0]!)).toBe(JSON.stringify({ id: 1, method: 'ping', params: {} }))
    // id 关联依旧按 id 配对（ Response 也走 base64 到达）。
    socket.emitRaw(btoa(JSON.stringify({ id: 1, result: 'pong' })))
    await expect(promise).resolves.toBe('pong')
  })

  it('rejects with the server error code and message verbatim', async () => {
    const client = makeClient()
    client.hostConnected()
    const socket = FakeSocket.instances[0]!
    socket.open()
    const promise = client.request('nope')
    const rejection = expect(promise).rejects.toMatchObject({
      code: 'unknown_method',
      message: 'no handler registered for method "nope"',
    })
    socket.reply(1, {
      error: { code: 'unknown_method', message: 'no handler registered for method "nope"' },
    })
    await rejection
    expect(new WsRpcError('x', 'y').name).toBe('WsRpcError')
  })

  it('rejects with timeout when no response arrives within the window', async () => {
    const client = makeClient({ requestTimeoutMs: 1_000 })
    client.hostConnected()
    const socket = FakeSocket.instances[0]!
    socket.open()
    const promise = client.request('slow')
    const rejection = expect(promise).rejects.toMatchObject({ code: 'timeout' })
    vi.advanceTimersByTime(1_000)
    await rejection
    // A late Response after the timeout is ignored — no unhandled rejection,
    // and the next request reuses the freed slot cleanly.
    socket.reply(1, { result: 'late' })
    const next = client.request('ping')
    expect(JSON.parse(socket.sent[1]!)).toMatchObject({ id: 2, method: 'ping' })
    socket.reply(2, { result: 'pong' })
    await expect(next).resolves.toBe('pong')
  })

  it('rejects request() immediately with not_connected when the socket is down', async () => {
    const client = makeClient()
    client.hostConnected()
    // Never opened.
    await expect(client.request('ping')).rejects.toMatchObject({ code: 'not_connected' })
    // Opened then dropped (reconnect backoff pending): still no queueing.
    const socket = FakeSocket.instances[0]!
    socket.open()
    socket.drop()
    await expect(client.request('ping')).rejects.toMatchObject({ code: 'not_connected' })
    expect(FakeSocket.instances[0]!.sent).toEqual([])
  })

  it('rejects every in-flight request when the connection drops', async () => {
    const client = makeClient()
    client.hostConnected()
    const socket = FakeSocket.instances[0]!
    socket.open()
    const first = client.request('a')
    const second = client.request('b')
    const rejections = Promise.all([
      expect(first).rejects.toMatchObject({ code: 'disconnected' }),
      expect(second).rejects.toMatchObject({ code: 'disconnected' }),
    ])
    socket.drop()
    await rejections
    // After reconnect, ids keep counting up on the fresh socket.
    vi.advanceTimersByTime(100)
    FakeSocket.instances[1]!.open()
    const after = client.request('ping')
    expect(JSON.parse(FakeSocket.instances[1]!.sent[0]!)).toMatchObject({ id: 3 })
    FakeSocket.instances[1]!.reply(3, { result: 'pong' })
    await expect(after).resolves.toBe('pong')
  })

  it('reconnects with exponential backoff and fires onReconnect on success', async () => {
    const reconnected = vi.fn()
    const client = makeClient({ onReconnect: reconnected })
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

  it('reports connection-state transitions (open → true, drop → false)', () => {
    // add-webui-allowed-roots D6：shell 依赖 onStateChange 渲染断线横幅。
    const states: boolean[] = []
    const client = makeClient({ onStateChange: (connected: boolean) => states.push(connected) })
    client.hostConnected()
    FakeSocket.instances[0]!.open()
    FakeSocket.instances[0]!.drop()
    vi.advanceTimersByTime(60_000)
    FakeSocket.instances[1]!.open()
    expect(states).toEqual([true, false, true])
  })

  it('does not report a state change for a user-driven close', () => {
    const states: boolean[] = []
    const client = makeClient({ onStateChange: (connected: boolean) => states.push(connected) })
    client.hostConnected()
    FakeSocket.instances[0]!.open()
    client.hostDisconnected()
    expect(states).toEqual([true])
  })
})

describe('jsonFrameCodec', () => {
  it('encodes the three envelope shapes with snake_case fields only', () => {
    expect(jsonFrameCodec.encode({ kind: 'request', id: 7, method: 'ping' })).toBe(
      JSON.stringify({ id: 7, method: 'ping', params: {} }),
    )
    expect(
      jsonFrameCodec.encode({ kind: 'response', id: 7, result: 'pong' }),
    ).toBe(JSON.stringify({ id: 7, result: 'pong' }))
    expect(
      jsonFrameCodec.encode({
        kind: 'response',
        id: 8,
        error: { code: 'unknown_method', message: 'nope' },
      }),
    ).toBe(JSON.stringify({ id: 8, error: { code: 'unknown_method', message: 'nope' } }))
    expect(
      jsonFrameCodec.encode({ kind: 'notification', method: 'turn.append', params: { seq: 4 } }),
    ).toBe(JSON.stringify({ method: 'turn.append', params: { seq: 4 } }))
  })

  it('decodes the three shapes without a wire discriminator and tolerates unknown fields', () => {
    expect(
      jsonFrameCodec.decode('{"id":1,"method":"ping","params":{},"future":true}'),
    ).toEqual({ kind: 'request', id: 1, method: 'ping', params: {} })
    expect(jsonFrameCodec.decode('{"id":2,"result":"pong"}')).toEqual({
      kind: 'response',
      id: 2,
      result: 'pong',
      error: undefined,
    })
    expect(
      jsonFrameCodec.decode('{"id":3,"error":{"code":"timeout","message":"late"}}'),
    ).toEqual({ kind: 'response', id: 3, error: { code: 'timeout', message: 'late' } })
    expect(
      jsonFrameCodec.decode('{"method":"session.created","params":{"session_id":"oc_a"}}'),
    ).toEqual({ kind: 'notification', method: 'session.created', params: { session_id: 'oc_a' } })
  })

  it('returns null for malformed payloads and shapes matching no frame', () => {
    expect(jsonFrameCodec.decode('not-json')).toBeNull()
    expect(jsonFrameCodec.decode('42')).toBeNull()
    expect(jsonFrameCodec.decode('[1,2,3]')).toBeNull()
    expect(jsonFrameCodec.decode('{}')).toBeNull()
    expect(jsonFrameCodec.decode('{"id":true,"result":1}')).toBeNull()
    // A non-numeric id falls through to the bare-method shape — the same
    // tolerance as the Rust untagged decode (extra fields don't reject).
    expect(jsonFrameCodec.decode('{"id":"nan","method":"x"}')).toEqual({
      kind: 'notification',
      method: 'x',
      params: undefined,
    })
    // Legacy bare frames are not envelope frames anymore.
    expect(jsonFrameCodec.decode('{"type":"session.created","session_id":"oc_a"}')).toBeNull()
  })
})
