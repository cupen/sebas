// @vitest-environment jsdom
/**
 * 共享 WS 客户端的认证态门禁单测（fix-webui-qa-round11 3.2，D4/B-6/A-3）。
 * 四种状态迁移逐一定钉：
 * 1. 未认证（登录页/首启设置页）——装载不建连、上闸静默（零升级尝试）；
 * 2. 登出——已建立的连接被闸侧关闭且不重连；
 * 3. 登录成功（撤闸）——立即建连；
 * 4. 认证失效（升级持续被拒）——收敛静默不刷屏（WsClient 容忍窗语义）。
 * `auth = false` 部署走第 3 条同一路径（me 探测后撤闸 = 启动即建连）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WsClient } from './ws.js'

/** Scriptable WebSocket double, installed as the global. */
class FakeSocket {
  static instances: FakeSocket[] = []
  static OPEN = 1
  static CLOSED = 3
  readyState = 0 // CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((ev: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  constructor(public url: string) {
    FakeSocket.instances.push(this)
  }
  close(): void {
    this.readyState = FakeSocket.CLOSED
    this.onclose?.()
  }
  send(): void {}
  open(): void {
    this.readyState = FakeSocket.OPEN
    this.onopen?.()
  }
  drop(): void {
    this.readyState = FakeSocket.CLOSED
    this.onclose?.()
  }
}

describe('sharedWs 认证态门禁（fix-webui-qa-round11 3.2）', () => {
  let ws: WsClient
  beforeEach(() => {
    FakeSocket.instances = []
    vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket)
    vi.useFakeTimers()
  })
  afterEach(async () => {
    // 单例跨用例存活：用例间上闸清场（关掉残余 socket、撤掉退避定时器），
    // 下一个用例从干净态出发。
    ws.setAuthGated(true)
    await Promise.resolve()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  // 动态 import 放用例内：模块单例在 stub 就位后才构造。
  async function client() {
    const mod = await import('./shared-ws.js')
    ws = mod.sharedWs
    return ws
  }

  it('装载零连接：未认证上闸后推进任意长时钟都没有升级尝试', async () => {
    const ws = await client()
    ws.setAuthGated(true) // checkAuth 探得未认证（登录页/首启设置页）
    vi.advanceTimersByTime(120_000)
    expect(FakeSocket.instances, '未认证态零升级尝试（B-6 冷启动噪音清除）').toHaveLength(0)
  })

  it('登出即断开：已建立的连接被关闭且不重连', async () => {
    const ws = await client()
    ws.setAuthGated(false) // 已登录工作
    ws.reconnectNow()
    expect(FakeSocket.instances).toHaveLength(1)
    FakeSocket.instances[0]!.open()
    // 登出：showLogin → 上闸 → 闸侧主动关闭既有 socket。
    ws.setAuthGated(true)
    expect(FakeSocket.instances[0]!.readyState).toBe(FakeSocket.CLOSED)
    vi.advanceTimersByTime(120_000)
    expect(FakeSocket.instances, '登出后不自动重连').toHaveLength(1)
  })

  it('登录成功建连自愈：撤闸立即发起一次尝试并打开', async () => {
    const ws = await client()
    ws.setAuthGated(true)
    vi.advanceTimersByTime(60_000)
    expect(FakeSocket.instances).toHaveLength(0)
    ws.setAuthGated(false) // markAuthReady（登录成功 / auth=false 部署）
    expect(FakeSocket.instances, '撤闸即建连').toHaveLength(1)
    FakeSocket.instances[0]!.open()
    expect(ws.connected).toBe(true)
  })

  it('认证失效不刷屏：升级持续被拒后收敛静默（容忍窗语义）', async () => {
    const ws = await client()
    ws.setAuthGated(false)
    ws.reconnectNow()
    // 首次升级成功过、随后会话过期：短退避梯上未打开的重连至多容忍窗次数
    // （sharedWs 用默认退避：500ms 起、逐次翻倍）。
    FakeSocket.instances[0]!.open()
    FakeSocket.instances[0]!.drop()
    vi.advanceTimersByTime(500)
    expect(FakeSocket.instances).toHaveLength(2)
    FakeSocket.instances[1]!.drop() // 401
    vi.advanceTimersByTime(1_000)
    expect(FakeSocket.instances).toHaveLength(3)
    FakeSocket.instances[2]!.drop()
    vi.advanceTimersByTime(2_000)
    expect(FakeSocket.instances).toHaveLength(4)
    FakeSocket.instances[3]!.drop()
    vi.advanceTimersByTime(4_000)
    expect(FakeSocket.instances).toHaveLength(5)
    FakeSocket.instances[4]!.drop() // 第 4 次未打开 → 疑似凭据失效
    vi.advanceTimersByTime(120_000)
    expect(FakeSocket.instances, '升级持续被拒后不再反复升级').toHaveLength(5)
    // 直到登录成功（撤闸）才复活。
    ws.setAuthGated(false)
    expect(FakeSocket.instances).toHaveLength(6)
  })
})
