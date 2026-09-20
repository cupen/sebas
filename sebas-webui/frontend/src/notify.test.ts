/**
 * 通知 store 单测（add-webui-tiered-notices 1.2）：栈上限挤占 / 去重窗口 /
 * 订阅退订 / 持久槽位。纯模块逻辑，无 DOM。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  DEDUPE_WINDOW_MS,
  ERROR_TOAST_DURATION_MS,
  MAX_TRANSIENT_ITEMS,
  dismiss,
  notify,
  resetNotices,
  setFatal,
  setWsDown,
  subscribeNotices,
  type NoticeItem,
  type NoticeState,
} from './notify.js'

function collect(): { states: NoticeState[]; unsub: () => void } {
  const states: NoticeState[] = []
  const unsub = subscribeNotices((s) => states.push(s))
  return { states, unsub }
}

beforeEach(() => {
  vi.useFakeTimers()
  resetNotices()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('notice store: 栈上限挤占（spec「栈上限与去重」）', () => {
  it('caps transient items at three, evicting the oldest', () => {
    const a = notify({ level: 'info', message: 'a' })!
    const b = notify({ level: 'warn', message: 'b' })!
    const c = notify({ level: 'info', message: 'c' })!
    notify({ level: 'warn', message: 'd' })
    const { states } = collect()
    const items = states[states.length - 1].items
    expect(items.map((it) => it.id)).toEqual([b, c, 4])
    expect(items).toHaveLength(MAX_TRANSIENT_ITEMS)
    expect(items.some((it) => it.id === a)).toBe(false)
  })

  it('error toasts auto-dismiss (round5 4.2): transient, evictable, cap-counted', () => {
    const e1 = notify({ level: 'error', message: 'e1' })!
    const e2 = notify({ level: 'error', message: 'e2' })!
    notify({ level: 'info', message: 'i0' })
    notify({ level: 'info', message: 'i1' })
    const { states } = collect()
    const items = states[states.length - 1].items
    // 4.2 起失败类默认瞬时（8s 自动消失）：不再驻留，参与 3 条上限挤占
    // ——e1 已被后续瞬时条挤出。
    expect(items.map((it) => it.id)).toEqual([e2, 3, 4])
    expect(items.every((it) => it.duration > 0)).toBe(true)
  })

  it('an explicit duration of 0 marks the item persistent regardless of level', () => {
    notify({ level: 'info', message: 'sticky', duration: 0 })
    const { states } = collect()
    expect(states[states.length - 1].items[0].duration).toBe(0)
  })
})

describe('notice store: 去重窗口', () => {
  it('drops the same message within the dedupe window', () => {
    expect(notify({ level: 'warn', message: 'same text' })).toBe(1)
    expect(notify({ level: 'warn', message: 'same text' })).toBeNull()
    const { states } = collect()
    expect(states[states.length - 1].items).toHaveLength(1)
  })

  it('allows the same message again after the window elapses', () => {
    notify({ level: 'warn', message: 'same text' })
    vi.advanceTimersByTime(DEDUPE_WINDOW_MS + 1)
    expect(notify({ level: 'warn', message: 'same text' })).toBe(2)
  })

  it('honours dedupeKey over the message text', () => {
    expect(notify({ level: 'info', message: 'one', dedupeKey: 'k' })).toBe(1)
    expect(notify({ level: 'info', message: 'two', dedupeKey: 'k' })).toBeNull()
    expect(notify({ level: 'info', message: 'two', dedupeKey: 'other' })).toBe(2)
  })

  it('dedupe is per message: different texts coexist', () => {
    notify({ level: 'info', message: 'x' })
    notify({ level: 'info', message: 'y' })
    const { states } = collect()
    expect(states[states.length - 1].items).toHaveLength(2)
  })
})

describe('notice store: dismiss 与订阅', () => {
  it('dismiss removes the item and a dismissed id can be re-notified', () => {
    const id = notify({ level: 'error', message: 'manual close' })!
    dismiss(id)
    const { states } = collect()
    expect(states[states.length - 1].items).toHaveLength(0)
    expect(notify({ level: 'error', message: 'manual close' })).toBe(2)
  })

  it('dismiss of an unknown id is a no-op (no broadcast)', () => {
    const { states, unsub } = collect()
    dismiss(999)
    expect(states).toHaveLength(1) // 仅订阅回放，无更新
    unsub()
  })

  it('subscribers replay the current state immediately and stop after unsubscribe', () => {
    notify({ level: 'info', message: 'before' })
    const { states, unsub } = collect()
    expect(states[0].items.map((it: NoticeItem) => it.message)).toEqual(['before'])
    notify({ level: 'info', message: 'after' })
    expect(states[states.length - 1].items).toHaveLength(2)
    unsub()
    notify({ level: 'info', message: 'silence' })
    expect(states[states.length - 1].items).toHaveLength(2)
  })

  it('default durations follow the level map (info 5s / warn 8s / error 8s, round5 4.2)', () => {
    notify({ level: 'info', message: 'i' })
    notify({ level: 'warn', message: 'w' })
    notify({ level: 'error', message: 'e' })
    const { states } = collect()
    const [i, w, e] = states[states.length - 1].items
    expect(i.duration).toBe(5_000)
    expect(w.duration).toBe(8_000)
    // 失败类不再驻留：与成功类策略分开常量化（ERROR_TOAST_DURATION_MS）。
    expect(e.duration).toBe(ERROR_TOAST_DURATION_MS)
    expect(ERROR_TOAST_DURATION_MS).toBe(8_000)
  })
})

describe('notice store: 持久槽位（ws 断线 / fatal）', () => {
  it('setWsDown toggles the slot without touching items', () => {
    notify({ level: 'info', message: 'toast' })
    setWsDown(true)
    const { states } = collect()
    const s = states[states.length - 1]
    expect(s.wsDown).toBe(true)
    expect(s.items).toHaveLength(1)
    setWsDown(true) // 幂等：不广播
    expect(states).toHaveLength(1)
  })

  it('setFatal carries kind/cause and clears on null', () => {
    setFatal({ kind: 'auth_rejected', cause: 'handshake refused' })
    const { states } = collect()
    expect(states[0].fatal).toEqual({ kind: 'auth_rejected', cause: 'handshake refused' })
    setFatal(null)
    expect(states[states.length - 1].fatal).toBeNull()
    expect(states[states.length - 1].wsDown).toBe(false)
  })
})
