// @vitest-environment jsdom
/**
 * theme.ts 三态语义：wa-dark class 是唯一开关；system 模式跟随 OS 的
 * matchMedia，显式 dark/light 覆盖 OS 并持久化到 localStorage。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { applyThemeMode, getThemeMode, resolvesToLight, setThemeMode } from './theme.js'

// jsdom 这里不提供 localStorage（about:blank origin），沿用仓库的内存
// polyfill 约定（见 transcript-view.test.ts）。
const store = new Map<string, string>()
const ls = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
  clear: () => store.clear(),
  key: () => null,
  get length() {
    return store.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })
beforeEach(() => store.clear())

function stubScheme(light: boolean): void {
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockReturnValue({ matches: light, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  )
}

afterEach(() => {
  localStorage.removeItem('sebas:theme')
  document.documentElement.classList.remove('wa-dark')
  vi.unstubAllGlobals()
})

describe('theme', () => {
  it('defaults to system; a dark-OS system mode keeps wa-dark on', () => {
    stubScheme(false)
    expect(getThemeMode()).toBe('system')
    expect(resolvesToLight('system')).toBe(false)
    applyThemeMode()
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
  })

  it('a light-OS system mode removes wa-dark', () => {
    stubScheme(true)
    expect(resolvesToLight('system')).toBe(true)
    applyThemeMode()
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
  })

  it('an explicit mode overrides the OS preference and persists', () => {
    stubScheme(true) // OS 说要 light，但用户显式选了 dark
    setThemeMode('dark')
    expect(getThemeMode()).toBe('dark')
    expect(localStorage.getItem('sebas:theme')).toBe('dark')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    setThemeMode('light')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
    // 回到 system：重新交给 OS（light）。
    setThemeMode('system')
    expect(localStorage.getItem('sebas:theme')).toBe('system')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
  })

  it('absent matchMedia degrades system mode to dark (no crash)', () => {
    vi.stubGlobal('matchMedia', undefined)
    expect(resolvesToLight('system')).toBe(false)
    applyThemeMode()
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
  })

  it('corrupted storage values fall back to system', () => {
    localStorage.setItem('sebas:theme', 'hotpink')
    expect(getThemeMode()).toBe('system')
  })
})
