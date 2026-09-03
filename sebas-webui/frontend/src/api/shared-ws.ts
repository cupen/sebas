/**
 * The app's single shared WebSocket client ("exactly one connection").
 * Views import this instance and subscribe; the client connects eagerly at
 * module load and reconnects with backoff on loss.
 */
import type { ReactiveController, ReactiveControllerHost } from 'lit'
import { WsClient } from './ws.js'

// A no-op host: the shared client outlives any single view. Lit requires a
// host to invoke the controller lifecycle itself, so the shim forwards
// `hostConnected` here — that is what makes the eager connect actually
// happen. Without the forward, `connect()` never runs and the app would sit
// with a dead socket while every view believes it is subscribed.
const eagerHost = {
  addController: (controller: ReactiveController) => controller.hostConnected?.(),
  requestUpdate: () => {},
  updateComplete: Promise.resolve(true),
} as never as ReactiveControllerHost

export const sharedWs = new WsClient(eagerHost, {
  onReconnect: () => window.dispatchEvent(new CustomEvent('sebas:refetch')),
})
