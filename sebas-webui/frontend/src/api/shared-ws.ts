/**
 * The app's single shared WebSocket client ("exactly one connection").
 * Views import this instance and subscribe; reconnects use backoff on loss.
 *
 * （fix-webui-qa-round11 3.2，D4/B-6）**认证态驱动连接生命周期**：本模块
 * 不再在装载时急连——未认证（登录页 / 首启设置页 / 登出态）的 SPA 一次升级
 * 尝试都不发（浏览器不再打「HTTP Authentication failed」的冷启动噪音），
 * app-shell 的鉴权探测（`/api/auth/me` + 登录/登出事件）经 `setAuthGated`
 * 驱动建连：认证就绪（含 `auth = false` 部署）撤闸即连，未认证上闸静默。
 */
import { WsClient, jsonFrameCodec } from './ws.js'

// The shared client outlives any single view. Constructing with options (no
// host) keeps connection start owned by the auth gate (app-shell), not module
// load (see module doc).
export const sharedWs = new WsClient({
  // add-ws-rpc-protocol D2/D5：codec 在构造点注入（JSON 首实现）——换单
  // 侧线上格式只动这里，帧语义与订阅面不动。
  codec: jsonFrameCodec,
  onReconnect: () => window.dispatchEvent(new CustomEvent('sebas:refetch')),
  // add-webui-allowed-roots D6：连接状态变化广播给 app-shell，渲染全局
  // 断线横幅（重连成功即消失）。
  onStateChange: (connected) =>
    window.dispatchEvent(new CustomEvent('sebas:ws-state', { detail: { connected } })),
})
