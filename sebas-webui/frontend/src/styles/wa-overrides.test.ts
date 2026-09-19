/**
 * wa-overrides.css 的既有契约断言（a11y.test.ts 同款源码读回）。
 * round3 4.1：wa-select 选项面板放宽最小宽度、选项单行省略——中文长模式
 * 文案（MODE_OPTIONS 的「allow（放行并留审计）」等）不再按字折行。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const css = readFileSync(join(here, 'wa-overrides.css'), 'utf8')

describe('wa-select option panel (round3 4.1)', () => {
  it('widens the listbox to content width with a cap', () => {
    expect(css).toMatch(/wa-select::part\(listbox\)\s*\{[^}]*min-width:\s*max-content/)
    expect(css).toMatch(/wa-select::part\(listbox\)\s*\{[^}]*max-width:\s*320px/)
  })

  it('keeps option labels on a single line with ellipsis', () => {
    expect(css).toMatch(/wa-option::part\(label\)\s*\{[^}]*white-space:\s*nowrap/)
    expect(css).toMatch(/wa-option::part\(label\)\s*\{[^}]*text-overflow:\s*ellipsis/)
  })
})
