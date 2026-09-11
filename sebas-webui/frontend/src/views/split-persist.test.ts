// @vitest-environment jsdom
/**
 * split-persist（workbench-interaction-polish 5.1/5.2，design D1）：假
 * storage 上的持久化与恢复、clamp 边界、无 storage / throwing storage
 * （隐私模式）下的诚实退化。
 *
 * jsdom 在 opaque origin 下没有 localStorage（window.localStorage 为
 * undefined）——持久化函数因此全部接受可注入的 `Storage`；组件侧不传参
 * （走带探针的 defaultStorage()），测试传 Map 假 storage。
 */

import { describe, expect, it } from 'vitest'
import {
  clampComposerHeight,
  clampRailWidth,
  COMPOSER_HEIGHT_KEY,
  defaultStorage,
  loadComposerHeight,
  loadRailWidth,
  RAIL_WIDTH_KEY,
  saveComposerHeight,
  saveRailWidth,
} from './split-persist.js'

/** Map-backed fake Storage（tasks 5.1/5.2 的「假 storage」）。 */
function fakeStorage(initial: Record<string, string> = {}): Storage {
  const m = new Map(Object.entries(initial))
  return {
    get length() {
      return m.size
    },
    clear: () => m.clear(),
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    key: (i: number) => [...m.keys()][i] ?? null,
    removeItem: (k: string) => void m.delete(k),
    setItem: (k: string, v: string) => void m.set(k, v),
  } as Storage
}

/** getItem/setItem 都抛错的假 storage（隐私模式）。 */
function throwingStorage(): Storage {
  return {
    length: 0,
    clear: () => {},
    getItem: () => {
      throw new Error('SecurityError: denied')
    },
    key: () => null,
    removeItem: () => {},
    setItem: () => {
      throw new Error('SecurityError: denied')
    },
  } as Storage
}

describe('rail width persistence (5.1)', () => {
  it('round-trips a saved width under the agreed key', () => {
    const store = fakeStorage()
    saveRailWidth(320, store)
    expect(store.getItem(RAIL_WIDTH_KEY)).toBe('320')
    expect(loadRailWidth(store)).toBe(320)
  })

  it('clamps into [180, 480] on save, and clamps out-of-range history on load', () => {
    const store = fakeStorage()
    saveRailWidth(50, store)
    expect(loadRailWidth(store)).toBe(180)
    saveRailWidth(9999, store)
    expect(loadRailWidth(store)).toBe(480)
    store.setItem(RAIL_WIDTH_KEY, '9999')
    expect(loadRailWidth(store)).toBe(480)
  })

  it('returns null when nothing (or garbage) is stored', () => {
    const store = fakeStorage()
    expect(loadRailWidth(store)).toBeNull()
    store.setItem(RAIL_WIDTH_KEY, 'not-a-number')
    expect(loadRailWidth(store)).toBeNull()
  })

  it('degrades silently on a throwing storage (privacy mode)', () => {
    const store = throwingStorage()
    expect(() => saveRailWidth(300, store)).not.toThrow()
    expect(loadRailWidth(store)).toBeNull()
  })

  it('degrades silently when there is no storage at all (jsdom opaque origin)', () => {
    expect(() => saveRailWidth(300, null)).not.toThrow()
    expect(loadRailWidth(null)).toBeNull()
    // 探针化的默认存储在此环境同样不可用且不抛。
    expect(defaultStorage()).toBeNull()
    expect(() => saveRailWidth(300)).not.toThrow()
  })

  it('clampRailWidth falls back to the default on non-finite input', () => {
    expect(clampRailWidth(Number.NaN)).toBe(220)
    expect(clampRailWidth(220)).toBe(220)
  })
})

describe('composer height persistence (5.2)', () => {
  it('round-trips a saved height under the agreed key', () => {
    const store = fakeStorage()
    saveComposerHeight(260, 1000, store)
    expect(store.getItem(COMPOSER_HEIGHT_KEY)).toBe('260')
    expect(loadComposerHeight(store)).toBe(260)
  })

  it('keeps the 120px floor and caps at half the measured area', () => {
    const store = fakeStorage()
    saveComposerHeight(50, 1000, store)
    expect(loadComposerHeight(store)).toBe(120)
    saveComposerHeight(900, 1000, store)
    expect(loadComposerHeight(store)).toBe(500)
    // 没有可信面积（0）时只保下限。
    saveComposerHeight(400, 0, store)
    expect(loadComposerHeight(store)).toBe(120)
  })

  it('degrades silently on a throwing storage or missing storage', () => {
    expect(() => saveComposerHeight(300, 1000, throwingStorage())).not.toThrow()
    expect(loadComposerHeight(throwingStorage())).toBeNull()
    expect(() => saveComposerHeight(300, 1000, null)).not.toThrow()
    expect(loadComposerHeight(null)).toBeNull()
  })

  it('clampComposerHeight accepts non-finite input at the floor', () => {
    expect(clampComposerHeight(Number.NaN, 1000)).toBe(120)
  })
})
