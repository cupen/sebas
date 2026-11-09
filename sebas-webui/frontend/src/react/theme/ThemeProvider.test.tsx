import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ThemeProvider, useReducedMotion, useThemeMode } from './ThemeProvider.js'

class MemoryStorage implements Storage {
  readonly data = new Map<string, string>()
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
}

interface FakeQuery {
  matches: boolean
  media: string
  listeners: Set<(event: { matches: boolean }) => void>
  addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => void
  removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => void
}

function installMatchMedia(initial: { light: boolean; reduce: boolean }): {
  set: (patch: Partial<{ light: boolean; reduce: boolean }>) => void
} {
  const queries = new Map<string, FakeQuery>()
  const get = (media: string): FakeQuery => {
    const existing = queries.get(media)
    if (existing) return existing
    const query: FakeQuery = {
      matches: media.includes('light') ? initial.light : initial.reduce,
      media,
      listeners: new Set(),
      addEventListener: (_type, listener) => {
        query.listeners.add(listener as unknown as (event: { matches: boolean }) => void)
      },
      removeEventListener: (_type, listener) => {
        query.listeners.delete(listener as unknown as (event: { matches: boolean }) => void)
      },
    }
    queries.set(media, query)
    return query
  }
  vi.stubGlobal('matchMedia', (media: string) => get(media))
  return {
    set(patch) {
      for (const query of queries.values()) {
        const next = query.media.includes('light') ? patch.light : patch.reduce
        if (next === undefined || next === query.matches) continue
        query.matches = next
        for (const listener of query.listeners) listener({ matches: next })
      }
    },
  }
}

function ModeProbe() {
  const { mode, resolved, setMode } = useThemeMode()
  const reduced = useReducedMotion()
  return (
    <div>
      <span data-testid="mode">{mode}</span>
      <span data-testid="resolved">{resolved}</span>
      <span data-testid="reduced">{String(reduced)}</span>
      <button type="button" onClick={() => setMode('light')}>
        light
      </button>
      <button type="button" onClick={() => setMode('system')}>
        system
      </button>
    </div>
  )
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  document.documentElement.removeAttribute('style')
  delete document.documentElement.dataset.theme
})

describe('ThemeProvider', () => {
  let storage: MemoryStorage

  beforeEach(() => {
    storage = new MemoryStorage()
    installMatchMedia({ light: false, reduce: false })
  })

  it('applies the persisted theme, CSS variables and AntD tokens', () => {
    render(
      <ThemeProvider storage={storage}>
        <ModeProbe />
      </ThemeProvider>,
    )
    expect(screen.getByTestId('mode').textContent).toBe('system')
    expect(screen.getByTestId('resolved').textContent).toBe('dark')
    expect(document.documentElement.dataset.theme).toBe('dark')
    expect(document.documentElement.style.getPropertyValue('--sebas-canvas')).toBe('#06080c')
  })

  it('follows the OS while in system mode and stops once pinned', () => {
    const media = installMatchMedia({ light: false, reduce: false })
    render(
      <ThemeProvider storage={storage}>
        <ModeProbe />
      </ThemeProvider>,
    )
    act(() => media.set({ light: true }))
    expect(screen.getByTestId('resolved').textContent).toBe('light')
    expect(document.documentElement.style.getPropertyValue('--sebas-canvas')).toBe('#e7e9f1')

    act(() => screen.getByRole('button', { name: 'light' }).click())
    expect(storage.getItem('sebas:theme')).toBe('light')
    act(() => media.set({ light: false }))
    expect(screen.getByTestId('resolved').textContent).toBe('light')
    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('honours an initial mode over storage', () => {
    storage.setItem('sebas:theme', 'dark')
    render(
      <ThemeProvider storage={storage} initialMode="light">
        <ModeProbe />
      </ThemeProvider>,
    )
    expect(screen.getByTestId('mode').textContent).toBe('light')
    expect(screen.getByTestId('resolved').textContent).toBe('light')
  })

  it('exposes reduced-motion and updates with the media query', () => {
    const media = installMatchMedia({ light: false, reduce: true })
    render(
      <ThemeProvider storage={storage}>
        <ModeProbe />
      </ThemeProvider>,
    )
    expect(screen.getByTestId('reduced').textContent).toBe('true')
    expect(document.querySelector('[data-reduced-motion="true"]')).not.toBeNull()
    act(() => media.set({ reduce: false }))
    expect(screen.getByTestId('reduced').textContent).toBe('false')
  })

  it('rejects useThemeMode outside the provider', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    expect(() => render(<ModeProbe />)).toThrow(/ThemeProvider/)
    spy.mockRestore()
  })
})
