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

// ── fix-webui-qa-round7 4.2：浅色 signal 不得近似错误语义色 ──────────────────

describe('light-theme signal stays clear of the error hue (round7 4.2)', () => {
  // 读 tokens.css 的浅色段，做色相几何断言而不是钉死 hex：
  // 「signal 与 failed 在浅色下的色相距离 ≥ 24°」即验收口径（composer 聚焦
  // 环不再呈现红棕错误观感），未来调色只要不跌回错误族就放行。
  it('light signal hue keeps its distance from the light failed hue and stays AA on white', async () => {
    const fs = await import('node:fs')
    const path = await import('node:path')
    const { fileURLToPath } = await import('node:url')
    const here = path.dirname(fileURLToPath(import.meta.url))
    const css = fs.readFileSync(path.join(here, './styles/tokens.css'), 'utf8')
    const hexToHslHue = (hex: string): number => {
      const r = parseInt(hex.slice(1, 3), 16) / 255
      const g = parseInt(hex.slice(3, 5), 16) / 255
      const b = parseInt(hex.slice(5, 7), 16) / 255
      const max = Math.max(r, g, b)
      const min = Math.min(r, g, b)
      if (max === min) return 0
      const d = max - min
      let h: number
      if (max === r) h = ((g - b) / d) % 6
      else if (max === g) h = (b - r) / d + 2
      else h = (r - g) / d + 4
      h *= 60
      return h < 0 ? h + 360 : h
    }
    // lastIndexOf：文件头注释里也提到该选择器字面量，indexOf 会切到暗色段。
    const lightBlock = css.slice(css.lastIndexOf(':root:not(.wa-dark)'))
    const pick = (token: string): string => {
      const m = lightBlock.match(new RegExp(`--sebas-${token}:\\s*(#[0-9a-fA-F]{6})`))
      if (!m) throw new Error(`--sebas-${token} missing in the light block`)
      return m[1]!
    }
    const signal = pick('signal')
    const failed = pick('status-failed')
    const hueSignal = hexToHslHue(signal)
    const hueFailed = hexToHslHue(failed)
    const distance = Math.min(Math.abs(hueSignal - hueFailed), 360 - Math.abs(hueSignal - hueFailed))
    // 旧值 #c2410c（hue≈18°）对 failed #d13438（hue≈358°）只隔 20° 边缘且
    // 同为低明度红族——观感即错误。现值 #b45309（hue≈26° 深琥珀橙）与
    // failed 拉开到 ≥ 24°，且对白底对比 ≥ 4.5:1（审批标签文本 AA）。
    expect(distance).toBeGreaterThanOrEqual(24)
    // 文本用途（等待批复标注 color: var(--sebas-signal)）对白底 ≥ 4.5:1。
    const lin = (c: number) => {
      const s = c / 255
      return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
    }
    const r = parseInt(signal.slice(1, 3), 16)
    const g = parseInt(signal.slice(3, 5), 16)
    const b = parseInt(signal.slice(5, 7), 16)
    const L = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    expect((1.05) / (L + 0.05)).toBeGreaterThanOrEqual(4.5)
  })
})
