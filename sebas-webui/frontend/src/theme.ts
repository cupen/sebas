/**
 * Theme mode for the console: `system` (follow the OS), or an explicit
 * dark/light override chosen in Settings → Appearance. The single switch is
 * the `wa-dark` class on <html> — Web Awesome's theme, tokens.css (the light
 * re-map is `:root:not(.wa-dark)`) and wa-overrides.css all key off it, so
 * toggling the class re-skins everything at once. The choice is persisted
 * per-browser in localStorage; index.html carries an inline pre-paint copy of
 * the class decision so a light-OS user never flashes dark before the module
 * bundle runs.
 */

export type ThemeMode = 'system' | 'dark' | 'light'

const STORAGE_KEY = 'sebas:theme'

export function getThemeMode(): ThemeMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw === 'dark' || raw === 'light' || raw === 'system') return raw
  } catch {
    /* storage may be disabled; degrade to following the OS */
  }
  return 'system'
}

/** True when the effective palette is light; system mode consults the OS. */
export function resolvesToLight(mode: ThemeMode): boolean {
  if (mode === 'light') return true
  if (mode === 'dark') return false
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: light)').matches
  )
}

/** Re-derive the `wa-dark` class from the persisted mode (and OS preference). */
export function applyThemeMode(): void {
  document.documentElement.classList.toggle('wa-dark', !resolvesToLight(getThemeMode()))
}

/** Persist the mode and apply it immediately. */
export function setThemeMode(mode: ThemeMode): void {
  try {
    localStorage.setItem(STORAGE_KEY, mode)
  } catch {
    /* best-effort persistence; the class still flips for this page view */
  }
  applyThemeMode()
}
