import { theme } from 'antd'
import { describe, expect, it } from 'vitest'
import {
  DARK,
  LIGHT,
  buildThemeConfig,
  contrastRatio,
  cssVariables,
  paletteFor,
  relativeLuminance,
} from './tokens.js'

describe('theme tokens', () => {
  it('selects the approved palette per mode', () => {
    expect(paletteFor('dark')).toBe(DARK)
    expect(paletteFor('light')).toBe(LIGHT)
  })

  it('emits the documented CSS custom properties', () => {
    const vars = cssVariables('dark')
    expect(Object.keys(vars)).toEqual([
      '--sebas-canvas',
      '--sebas-surface',
      '--sebas-surface-2',
      '--sebas-well',
      '--sebas-border',
      '--sebas-border-strong',
      '--sebas-text',
      '--sebas-text-dim',
      '--sebas-text-faint',
      '--sebas-accent',
      '--sebas-accent-strong',
      '--sebas-accent-ink',
      '--sebas-signal',
      '--sebas-ok',
      '--sebas-warn',
      '--sebas-err',
      '--sebas-info',
    ])
    expect(vars['--sebas-canvas']).toBe(DARK.canvas)
    expect(cssVariables('light')['--sebas-canvas']).toBe(LIGHT.canvas)
  })

  it('wires AntD to the dark/default algorithms with semantic tokens', () => {
    const dark = buildThemeConfig('dark')
    expect(dark.algorithm).toBe(theme.darkAlgorithm)
    expect(dark.token?.colorBgBase).toBe(DARK.canvas)
    expect(dark.token?.colorText).toBe(DARK.text)
    expect(dark.token?.colorPrimary).toBe(DARK.accentStrong)

    const light = buildThemeConfig('light')
    expect(light.algorithm).toBe(theme.defaultAlgorithm)
    expect(light.token?.colorBgBase).toBe(LIGHT.canvas)
    expect(light.components?.Button?.controlHeight).toBe(28)
  })

  it('computes WCAG luminance and contrast for both palettes', () => {
    expect(relativeLuminance('#000000')).toBe(0)
    expect(relativeLuminance('#ffffff')).toBeCloseTo(1, 10)
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 10)
    expect(contrastRatio('#ffffff', '#ffffff')).toBeCloseTo(1, 10)
    expect(() => relativeLuminance('nope')).toThrow()

    for (const palette of [DARK, LIGHT]) {
      expect(contrastRatio(palette.text, palette.canvas)).toBeGreaterThanOrEqual(7)
      expect(contrastRatio(palette.text, palette.surface)).toBeGreaterThanOrEqual(7)
      expect(contrastRatio(palette.dim, palette.surface)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(palette.accentInk, palette.accentStrong)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(palette.faint, palette.surface)).toBeGreaterThanOrEqual(3)
    }
  })
})
