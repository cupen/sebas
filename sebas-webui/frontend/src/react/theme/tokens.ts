/**
 * Product theme tokens (2.3). Values mirror the approved blueprint and the
 * existing Lit palette; AntD consumes the semantic subset through
 * `ConfigProvider`, while product surfaces can read the same tokens.
 */

import { theme, type ThemeConfig } from 'antd'

export type PaletteMode = 'dark' | 'light'

export interface Palette {
  canvas: string
  surface: string
  surface2: string
  well: string
  border: string
  borderStrong: string
  text: string
  dim: string
  faint: string
  accent: string
  accentStrong: string
  accentInk: string
  signal: string
  ok: string
  warn: string
  err: string
  info: string
}

export const DARK: Palette = {
  canvas: '#06080c',
  surface: '#10141c',
  surface2: '#161b26',
  well: '#0b0e15',
  border: '#212a3a',
  borderStrong: '#313d52',
  text: '#dde3ee',
  dim: '#939db1',
  faint: '#5f6a80',
  accent: '#8b93ff',
  accentStrong: '#5b64f2',
  accentInk: '#ffffff',
  signal: '#ff9457',
  ok: '#46d189',
  warn: '#f2b04e',
  err: '#f47174',
  info: '#6ea8fe',
}

export const LIGHT: Palette = {
  canvas: '#e7e9f1',
  surface: '#ffffff',
  surface2: '#f3f5f9',
  well: '#f6f8fa',
  border: '#dfe3ec',
  borderStrong: '#c6cedd',
  text: '#1b2230',
  dim: '#5b6577',
  faint: '#7a8496',
  accent: '#4f5bd5',
  accentStrong: '#4350c9',
  accentInk: '#ffffff',
  signal: '#b45309',
  ok: '#177a4b',
  warn: '#9a6206',
  err: '#b4232a',
  info: '#1f5fa8',
}

export function paletteFor(mode: PaletteMode): Palette {
  return mode === 'dark' ? DARK : LIGHT
}

/** User-facing CSS custom properties shared by React surfaces. */
export function cssVariables(mode: PaletteMode): Record<string, string> {
  const p = paletteFor(mode)
  return {
    '--sebas-canvas': p.canvas,
    '--sebas-surface': p.surface,
    '--sebas-surface-2': p.surface2,
    '--sebas-well': p.well,
    '--sebas-border': p.border,
    '--sebas-border-strong': p.borderStrong,
    '--sebas-text': p.text,
    '--sebas-text-dim': p.dim,
    '--sebas-text-faint': p.faint,
    '--sebas-accent': p.accent,
    '--sebas-accent-strong': p.accentStrong,
    '--sebas-accent-ink': p.accentInk,
    '--sebas-signal': p.signal,
    '--sebas-ok': p.ok,
    '--sebas-warn': p.warn,
    '--sebas-err': p.err,
    '--sebas-info': p.info,
  }
}

/**
 * AntD theme: approved algorithm + semantic token overrides. No full AntD
 * reset import; CSS Modules own layout and AntD owns control primitives.
 */
export function buildThemeConfig(mode: PaletteMode, motion = true): ThemeConfig {
  const p = paletteFor(mode)
  return {
    algorithm: mode === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm,
    token: {
      // AntD 6 exposes motion as a seed token (not a ConfigProvider prop).
      motion,
      colorPrimary: p.accentStrong,
      colorInfo: p.info,
      colorSuccess: p.ok,
      colorWarning: p.warn,
      colorError: p.err,
      colorBgBase: p.canvas,
      colorBgContainer: p.surface,
      colorBgElevated: p.surface2,
      colorBorder: p.border,
      colorBorderSecondary: p.borderStrong,
      colorText: p.text,
      colorTextSecondary: p.dim,
      // AntD 6 maps `Typography.Text type="secondary"` to the tertiary token;
      // keep 13px secondary copy at >=4.5:1 instead of the decorative faint tone.
      colorTextTertiary: p.dim,
      borderRadius: 8,
      fontFamily:
        'ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, "Noto Sans", sans-serif',
      fontSize: 13,
    },
    components: {
      Button: { controlHeight: 28, borderRadius: 7 },
      Input: { controlHeight: 28, borderRadius: 7 },
      Select: { controlHeight: 28, borderRadius: 7 },
    },
  }
}

function channel(value: number): number {
  const normalized = value / 255
  return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4
}

export function relativeLuminance(hex: string): number {
  const match = /^#([0-9a-f]{6})$/i.exec(hex)
  if (!match) throw new Error(`unsupported colour: ${hex}`)
  const raw = parseInt(match[1]!, 16)
  const r = (raw >> 16) & 0xff
  const g = (raw >> 8) & 0xff
  const b = raw & 0xff
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/** WCAG contrast ratio between two opaque hex colours. */
export function contrastRatio(foreground: string, background: string): number {
  const a = relativeLuminance(foreground)
  const b = relativeLuminance(background)
  const lighter = Math.max(a, b)
  const darker = Math.min(a, b)
  return (lighter + 0.05) / (darker + 0.05)
}
