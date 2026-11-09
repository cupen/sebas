/**
 * Theme mode for the console: `system` (follow the OS), or an explicit
 * dark/light override chosen in Settings → Appearance.
 *
 * The single switch is the `data-theme` attribute on <html>: index.html paints
 * it before the bundle runs, `react/theme/ThemeProvider.tsx` owns it afterwards
 * (and mirrors the semantic palette as inline custom properties), and
 * `styles/tokens.css` keys its light re-map off `[data-theme='light']`.
 *
 * Only the type lives here — the React theme provider owns the storage
 * read/write and the DOM application (`react/boundary/preferences.ts`).
 */

export type ThemeMode = 'system' | 'dark' | 'light'
