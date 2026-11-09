import { describe, expect, it } from 'vitest'
import {
  STORAGE_KEYS,
  loadPreferences,
  readLastSettingsSection,
  readLastUsedModelPair,
  readRailCollapsed,
  readRailExpanded,
  readRailHistoryOpen,
  readSeenAnchor,
  readTheme,
  writeLastSettingsSection,
  writeRailCollapsed,
  writeRailExpanded,
  writeRailHistoryOpen,
  writeSeenAnchor,
  writeTheme,
} from './preferences.js'

class MemoryStorage implements Storage {
  private readonly data = new Map<string, string>()

  get length(): number {
    return this.data.size
  }

  clear(): void {
    this.data.clear()
  }

  getItem(key: string): string | null {
    return this.data.get(key) ?? null
  }

  key(index: number): string | null {
    return Array.from(this.data.keys())[index] ?? null
  }

  removeItem(key: string): void {
    this.data.delete(key)
  }

  setItem(key: string, value: string): void {
    this.data.set(key, String(value))
  }

  raw(key: string): string | null {
    return this.getItem(key)
  }
}

const throwing: Storage = {
  length: 0,
  clear: () => {
    throw new Error('denied')
  },
  getItem: () => {
    throw new Error('denied')
  },
  key: () => {
    throw new Error('denied')
  },
  removeItem: () => {
    throw new Error('denied')
  },
  setItem: () => {
    throw new Error('denied')
  },
}

describe('preferences', () => {
  it('defaults every key when storage is empty', () => {
    const store = new MemoryStorage()
    expect(readTheme(store)).toBe('system')
    expect(readRailExpanded(store)).toEqual({})
    expect(readRailHistoryOpen(store)).toBe(false)
    expect(readLastSettingsSection(store)).toBe('generic')
    expect(readLastUsedModelPair(store)).toBeNull()
    expect(readSeenAnchor('oc_1', store)).toBeNull()
    expect(loadPreferences(store)).toEqual({
      theme: 'system',
      railWidth: null,
      composerHeight: null,
      workbenchFocus: null,
      railExpanded: {},
      railHistoryOpen: false,
      lastSettingsSection: 'generic',
      lastUsedModelPair: null,
    })
  })

  it('round-trips the rail collapse preference and clears it on expand', () => {
    const store = new MemoryStorage()
    expect(readRailCollapsed(store)).toBe(false)
    writeRailCollapsed(true, store)
    expect(store.raw(STORAGE_KEYS.railCollapsed)).toBe('1')
    expect(readRailCollapsed(store)).toBe(true)
    writeRailCollapsed(false, store)
    expect(readRailCollapsed(store)).toBe(false)
  })

  it('round-trips the theme under the existing sebas:theme key', () => {
    const store = new MemoryStorage()
    writeTheme('light', store)
    expect(store.raw(STORAGE_KEYS.theme)).toBe('light')
    expect(readTheme(store)).toBe('light')
    store.setItem(STORAGE_KEYS.theme, 'solarized')
    expect(readTheme(store)).toBe('system')
  })

  it('reads the legacy expanded-project array and ignores junk', () => {
    const store = new MemoryStorage()
    store.setItem(STORAGE_KEYS.railExpanded, JSON.stringify(['/a', '', 3, '/b']))
    expect(readRailExpanded(store)).toEqual({ '/a': true, '/b': true })
    store.setItem(STORAGE_KEYS.railExpanded, 'not json')
    expect(readRailExpanded(store)).toEqual({})
    // Unknown shapes stay a no-record read (focus default still applies).
    store.setItem(STORAGE_KEYS.railExpanded, JSON.stringify('nope'))
    expect(readRailExpanded(store)).toEqual({})
    store.setItem(STORAGE_KEYS.railExpanded, JSON.stringify({ '/a': 'yes', '/b': 1 }))
    expect(readRailExpanded(store)).toEqual({})
  })

  it('round-trips explicit collapse and reads the legacy expanded array', () => {
    const store = new MemoryStorage()
    // Explicit false is a decision: it must survive a reload, otherwise the
    // absent-key focus default silently re-expands a collapsed project.
    writeRailExpanded({ '/a': true, '/b': false }, store)
    expect(readRailExpanded(store)).toEqual({ '/a': true, '/b': false })
    expect(store.raw(STORAGE_KEYS.railExpanded)).toBe('{"\/a":true,"\/b":false}')
    // A fully collapsed tree is the record `{path:false}`, not an empty array.
    writeRailExpanded({ '/b': false }, store)
    expect(readRailExpanded(store)).toEqual({ '/b': false })

    // Legacy Lit array (expanded paths only) still migrates to true entries.
    store.setItem(STORAGE_KEYS.railExpanded, JSON.stringify(['/a', '/b']))
    expect(readRailExpanded(store)).toEqual({ '/a': true, '/b': true })
    // Empty array = no records (never "everything collapsed").
    store.setItem(STORAGE_KEYS.railExpanded, '[]')
    expect(readRailExpanded(store)).toEqual({})
  })

  it('toggles rail history open', () => {
    const store = new MemoryStorage()
    writeRailHistoryOpen(true, store)
    expect(store.raw(STORAGE_KEYS.railHistoryOpen)).toBe('1')
    expect(readRailHistoryOpen(store)).toBe(true)
    writeRailHistoryOpen(false, store)
    expect(readRailHistoryOpen(store)).toBe(false)
  })

  it('validates the last settings section', () => {
    const store = new MemoryStorage()
    writeLastSettingsSection('models', store)
    expect(readLastSettingsSection(store)).toBe('models')
    store.setItem(STORAGE_KEYS.lastSettingsSection, 'legacy-section')
    expect(readLastSettingsSection(store)).toBe('generic')
  })

  it('parses the last used model pair defensively', () => {
    const store = new MemoryStorage()
    store.setItem(
      STORAGE_KEYS.lastUsedModelPair,
      JSON.stringify({ provider: 'p', model: 'm' }),
    )
    expect(readLastUsedModelPair(store)).toEqual({ provider: 'p', model: 'm' })
    for (const bad of ['{', 'null', '[]', '{"provider":1,"model":"m"}']) {
      store.setItem(STORAGE_KEYS.lastUsedModelPair, bad)
      expect(readLastUsedModelPair(store)).toBeNull()
    }
  })

  it('keeps the raw encoded session key namespace and only advances anchors', () => {
    const store = new MemoryStorage()
    writeSeenAnchor('oc_%00a', 3, store)
    expect(store.raw(STORAGE_KEYS.seenPrefix + 'oc_%00a')).toBe('{"anchor_count":3}')
    expect(readSeenAnchor('oc_%00a', store)).toBe(3)
    writeSeenAnchor('oc_%00a', 2, store)
    expect(readSeenAnchor('oc_%00a', store)).toBe(3)
    writeSeenAnchor('oc_%00a', 5, store)
    expect(readSeenAnchor('oc_%00a', store)).toBe(5)
  })

  it('treats legacy seen_ts payloads and junk as fully read', () => {
    const store = new MemoryStorage()
    store.setItem(STORAGE_KEYS.seenPrefix + 'legacy', JSON.stringify({ seen_ts: 123 }))
    expect(readSeenAnchor('legacy', store)).toBeNull()
    store.setItem(STORAGE_KEYS.seenPrefix + 'junk', 'nope')
    expect(readSeenAnchor('junk', store)).toBeNull()
    store.setItem(STORAGE_KEYS.seenPrefix + 'negative', '{"anchor_count":-1}')
    expect(readSeenAnchor('negative', store)).toBeNull()
  })

  it('degrades to defaults when storage access throws', () => {
    expect(readTheme(throwing)).toBe('system')
    expect(readRailExpanded(throwing)).toEqual({})
    expect(readRailHistoryOpen(throwing)).toBe(false)
    expect(readLastSettingsSection(throwing)).toBe('generic')
    expect(readLastUsedModelPair(throwing)).toBeNull()
    expect(readSeenAnchor('x', throwing)).toBeNull()
    expect(() => writeTheme('dark', throwing)).not.toThrow()
    expect(() => writeSeenAnchor('x', 1, throwing)).not.toThrow()
  })
})
