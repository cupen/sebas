import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'
import { ConfigProvider } from 'antd'
import zhCN from 'antd/locale/zh_CN'
import type { ThemeMode } from '../../theme.js'
import { readTheme, writeTheme } from '../boundary/preferences.js'
import { buildThemeConfig, cssVariables, type PaletteMode } from './tokens.js'

interface ThemeContextValue {
  mode: ThemeMode
  resolved: PaletteMode
  setMode: (mode: ThemeMode) => void
}

const ThemeContext = createContext<ThemeContextValue | null>(null)

export function useThemeMode(): ThemeContextValue {
  const value = useContext(ThemeContext)
  if (value === null) throw new Error('useThemeMode must be used inside <ThemeProvider>')
  return value
}

function systemPrefersLight(): boolean {
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: light)').matches
  )
}

export interface ThemeProviderProps {
  children: ReactNode
  /** Initial mode; defaults to the persisted `sebas:theme` value. */
  initialMode?: ThemeMode
  /** Injected for tests; defaults to window.localStorage. */
  storage?: Storage | null
}

/**
 * AntD locale + dark/light algorithm + semantic tokens (2.3). The effective
 * mode follows the OS only while `mode === 'system'`; an explicit choice is
 * persisted under the existing `sebas:theme` key.
 */
export function ThemeProvider({ children, initialMode, storage }: ThemeProviderProps) {
  const [mode, setModeState] = useState<ThemeMode>(() => initialMode ?? readTheme(storage ?? undefined))
  const [systemLight, setSystemLight] = useState<boolean>(systemPrefersLight)
  const resolved: PaletteMode = mode === 'system' ? (systemLight ? 'light' : 'dark') : mode

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const query = window.matchMedia('(prefers-color-scheme: light)')
    const onChange = (event: MediaQueryListEvent) => setSystemLight(event.matches)
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])

  useEffect(() => {
    const root = document.documentElement
    for (const [key, value] of Object.entries(cssVariables(resolved))) {
      root.style.setProperty(key, value)
    }
    root.dataset.theme = resolved
    return () => {
      for (const key of Object.keys(cssVariables(resolved))) root.style.removeProperty(key)
      delete root.dataset.theme
    }
  }, [resolved])

  const setMode = useCallback(
    (next: ThemeMode) => {
      setModeState(next)
      writeTheme(next, storage ?? undefined)
    },
    [storage],
  )

  const context = useMemo(() => ({ mode, resolved, setMode }), [mode, resolved, setMode])
  const reducedMotion = useReducedMotion()
  const themeConfig = useMemo(
    () => buildThemeConfig(resolved, !reducedMotion),
    [resolved, reducedMotion],
  )

  return (
    <ThemeContext.Provider value={context}>
      <ConfigProvider locale={zhCN} theme={themeConfig}>
        <div data-theme={resolved} data-reduced-motion={reducedMotion ? 'true' : 'false'}>
          {children}
        </div>
      </ConfigProvider>
    </ThemeContext.Provider>
  )
}

/** True while the OS requests reduced motion; React surfaces drop transitions. */
export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(() => {
    return (
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    )
  })
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const query = window.matchMedia('(prefers-reduced-motion: reduce)')
    const onChange = (event: MediaQueryListEvent) => setReduced(event.matches)
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])
  return reduced
}
