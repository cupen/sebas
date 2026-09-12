// @vitest-environment jsdom
/**
 * rail-declutter-unread 2.1 — the shared per-session read cursor.
 * Covers: write/read round-trip, first-visit (no anchor = fully read),
 * legacy bare-number migration, monotonic writes, and focus anchoring and
 * clearing (badge = msg_count − anchor_count reaches 0).
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { readAnchor, unreadCount, writeFocusAnchor, writeSeen } from './unread-cursor.js'

const store = new Map<string, string>()
beforeEach(() => store.clear())

const ls = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => {
    store.set(k, v)
  },
  removeItem: (k: string) => {
    store.delete(k)
  },
  clear: () => store.clear(),
  key: () => null,
  get length() {
    return store.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })

describe('unread cursor (shared seen + badge anchor)', () => {
  it('first visit has no anchor → treated as fully read', () => {
    expect(readAnchor('oc_1%00')).toBeNull()
    expect(unreadCount('oc_1%00', 42)).toBe(0)
  })

  it('focus anchor write makes the badge count new messages and clears on refocus', () => {
    writeFocusAnchor('oc_1%00', 3)
    expect(unreadCount('oc_1%00', 3)).toBe(0)
    expect(unreadCount('oc_1%00', 5)).toBe(2)
    // 负数不出现（消息不可能比已读的少，但口径要兜底）。
    expect(unreadCount('oc_1%00', 1)).toBe(0)
    // 再次聚焦 → 锚推进 → 清零。
    writeFocusAnchor('oc_1%00', 5)
    expect(unreadCount('oc_1%00', 5)).toBe(0)
  })

  it('writeSeen advances both the seam timestamp and the badge anchor', () => {
    writeSeen('oc_2%00', 1_700, 4)
    const anchor = readAnchor('oc_2%00')
    expect(anchor?.seenTs).toBe(1_700)
    expect(anchor?.anchorCount).toBe(4)
    expect(unreadCount('oc_2%00', 4)).toBe(0)
    expect(unreadCount('oc_2%00', 6)).toBe(2)
  })

  it('writes are monotonic — older values never regress stored ones', () => {
    writeSeen('oc_3%00', 2_000, 5)
    writeSeen('oc_3%00', 1_000, 2)
    const anchor = readAnchor('oc_3%00')
    expect(anchor?.seenTs).toBe(2_000)
    expect(anchor?.anchorCount).toBe(5)
  })

  it('legacy bare-number seen values migrate read-only (no false unread)', () => {
    // 旧 transcript 实现写入的裸时间戳。
    store.set('sebas:seen:oc_4%00', '1700')
    const anchor = readAnchor('oc_4%00')
    expect(anchor).toEqual({ seenTs: 1700, anchorCount: null })
    // 段数不可重构 → 按已读，历史不集体冒红点。
    expect(unreadCount('oc_4%00', 99)).toBe(0)
    // 下一次聚焦补全锚。
    writeFocusAnchor('oc_4%00', 99)
    expect(readAnchor('oc_4%00')?.anchorCount).toBe(99)
  })

  it('omitting anchorCount in writeSeen keeps the existing count', () => {
    writeSeen('oc_5%00', 5_000, 7)
    // 更晚的调用只推进时间戳、不携带段数 → 既有锚保持。
    writeSeen('oc_5%00', 6_000)
    const anchor = readAnchor('oc_5%00')
    expect(anchor?.seenTs).toBe(6_000)
    expect(anchor?.anchorCount).toBe(7)
  })

  it('disabled storage degrades silently', () => {
    const breaking = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('blocked')
      },
    }
    Object.defineProperty(globalThis, 'localStorage', { value: breaking, configurable: true })
    expect(readAnchor('oc_x')).toBeNull()
    expect(() => writeFocusAnchor('oc_x', 1)).not.toThrow()
    expect(() => writeSeen('oc_x', 5)).not.toThrow()
    Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })
  })
})
