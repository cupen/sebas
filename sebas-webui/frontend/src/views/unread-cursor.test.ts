// @vitest-environment jsdom
/**
 * rail-declutter-unread 2.1 + session-parallel-liveness-and-unread-polish
 * 2.3 (design D3) — the shared per-session read cursor, single-field
 * `{anchor_count}`.
 *
 * Covers: write/read round-trip, first-visit (no anchor = fully read),
 * legacy `seen_ts`-shaped data reads as "no anchor" and is overwritten by a
 * pure anchor on first write, monotonic writes, and focus anchoring and
 * clearing (badge = msg_count − anchor_count reaches 0). The three write
 * paths — streaming advance, focus advance, manual read-to-bottom — all
 * funnel into the SAME stored field.
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { readAnchorCount, unreadCount, writeFocusAnchor, writeSeen } from './unread-cursor.js'

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

describe('unread cursor (single segment-count anchor)', () => {
  it('first visit has no anchor → treated as fully read', () => {
    expect(readAnchorCount('oc_1%00')).toBeNull()
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

  it('writeSeen stores exactly one field and advances the badge anchor', () => {
    writeSeen('oc_2%00', 4)
    const raw = store.get('sebas:seen:oc_2%00')!
    // 单字段形状钉死：seen_ts 不再存在（2.3）。
    expect(Object.keys(JSON.parse(raw) as Record<string, unknown>).sort()).toEqual([
      'anchor_count',
    ])
    expect(readAnchorCount('oc_2%00')).toBe(4)
    expect(unreadCount('oc_2%00', 4)).toBe(0)
    expect(unreadCount('oc_2%00', 6)).toBe(2)
  })

  it('writes are monotonic — older values never regress stored ones', () => {
    writeSeen('oc_3%00', 5)
    writeSeen('oc_3%00', 2)
    expect(readAnchorCount('oc_3%00')).toBe(5)
  })

  it('legacy seen_ts-shaped JSON reads as no anchor (fully read) and is overwritten (2.3)', () => {
    // 旧双字段形态：读取按「无 anchor」对待——段数不可重构 → 按已读，
    // 历史不集体冒红点；不做字段迁移。
    store.set('sebas:seen:oc_4%00', JSON.stringify({ seen_ts: 1700, anchor_count: null }))
    expect(readAnchorCount('oc_4%00')).toBeNull()
    expect(unreadCount('oc_4%00', 99)).toBe(0)
    // 下一次写入（聚焦/读到底/流式推进任一路）→ 纯 {anchor_count} 覆写。
    writeFocusAnchor('oc_4%00', 99)
    const stored = JSON.parse(store.get('sebas:seen:oc_4%00')!) as Record<string, unknown>
    expect(Object.keys(stored).sort()).toEqual(['anchor_count'])
    expect(readAnchorCount('oc_4%00')).toBe(99)
  })

  it('legacy bare-number seen values read as no anchor too (2.3)', () => {
    // 更老的单数字形态同罪：读为 fully-read，不迁移。
    store.set('sebas:seen:oc_6%00', '1700')
    expect(readAnchorCount('oc_6%00')).toBeNull()
    expect(unreadCount('oc_6%00', 7)).toBe(0)
  })

  it('streaming advance = focus advance = manual read-to-bottom: all three write the same field', () => {
    // 三条写路径共用同一存储键的同一字段（live-turn-stream delta：
    // 「流式推进与聚焦写锚、手动读到底推进 SHALL 写同一个存储键的同一个
    // 字段」）。各路径抵达同一水位后，badge 读数一致且互不回退。
    writeSeen('oc_7%00', 3) // 流式贴底推进（transcript commitMarkSeen）
    expect(readAnchorCount('oc_7%00')).toBe(3)
    writeFocusAnchor('oc_7%00', 3) // 聚焦写锚（rail switch）
    expect(readAnchorCount('oc_7%00')).toBe(3)
    writeSeen('oc_7%00', 3) // 手动读到底（mark-all-seen）
    expect(readAnchorCount('oc_7%00')).toBe(3)
    expect(unreadCount('oc_7%00', 3)).toBe(0)
    // 任何一路先到更高水位，其余路径不得回退它。
    writeSeen('oc_7%00', 5)
    writeFocusAnchor('oc_7%00', 4)
    expect(readAnchorCount('oc_7%00')).toBe(5)
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
    expect(readAnchorCount('oc_x')).toBeNull()
    expect(() => writeFocusAnchor('oc_x', 1)).not.toThrow()
    expect(() => writeSeen('oc_x', 5)).not.toThrow()
    Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })
  })
})
