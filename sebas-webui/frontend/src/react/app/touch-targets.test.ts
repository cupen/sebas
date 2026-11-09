// @vitest-environment jsdom
/**
 * 375px Drawer 触控预算（blueprint ≥44px，视觉 gate 唯一遗留缺陷）。
 *
 * 断言对象是 CSS Modules 源码本身：vitest 不做真实布局（happy-dom 无排版、
 * CSS Modules 在测试里也不注入），所以量像素只能落到 Playwright；这里钉住
 * 的是「规则形态」——≤600px（Shell 唯一启用 Drawer 的档位）块内导航行、
 * 行操作与主题分段确有 44px 下限，且桌面紧凑密度（基础规则）没有被顺手放大。
 *
 * 选择器只用类名与媒体条件，不做截图，也不依赖 hash 后的类名。
 */
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const here = dirname(fileURLToPath(import.meta.url))
const read = (file: string): string => readFileSync(join(here, file), 'utf8')

/** 取 `@media (max-width: 600px)` 块的正文（大括号配平）。 */
function narrowBlock(source: string): string {
  const start = source.indexOf('@media (max-width: 600px)')
  expect(start, 'expected a @media (max-width: 600px) block').toBeGreaterThanOrEqual(0)
  const open = source.indexOf('{', start)
  expect(open).toBeGreaterThan(start)
  let depth = 0
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === '{') depth += 1
    else if (source[i] === '}') {
      depth -= 1
      if (depth === 0) return source.slice(open + 1, i)
    }
  }
  throw new Error('unterminated @media block')
}

/** 把一段 CSS 拆成 [选择器, 声明体] 对（本文件只用单层规则，无嵌套）。 */
function rules(source: string): Array<[string, string]> {
  const out: Array<[string, string]> = []
  let cursor = 0
  while (cursor < source.length) {
    const open = source.indexOf('{', cursor)
    if (open === -1) break
    let depth = 0
    let close = -1
    for (let i = open; i < source.length; i += 1) {
      if (source[i] === '{') depth += 1
      else if (source[i] === '}') {
        depth -= 1
        if (depth === 0) {
          close = i
          break
        }
      }
    }
    if (close === -1) throw new Error('unbalanced braces')
    out.push([source.slice(cursor, open).trim(), source.slice(open + 1, close)])
    cursor = close + 1
  }
  return out
}

/** 选择器恰为 `selector` 的规则声明体。 */
function exactBody(source: string, selector: string): string {
  const matched = rules(source).filter(([found]) => found === selector)
  expect(matched.length, `expected an exact ${selector} rule`).toBe(1)
  return matched[0][1]
}

/** 选择器含 `needle` 的规则并集声明体。 */
function bodyFor(source: string, needle: string): string {
  const matched = rules(source).filter(([selector]) => selector.includes(needle))
  expect(matched.length, `expected a rule whose selector contains ${needle}`).toBeGreaterThan(0)
  return matched.map(([, body]) => body).join('\n')
}

/** 移除所有 `@media` 块，只留基础（桌面）规则。 */
function outsideMedia(source: string): string {
  let out = ''
  let cursor = 0
  while (true) {
    const at = source.indexOf('@media', cursor)
    if (at === -1) return out + source.slice(cursor)
    out += source.slice(cursor, at)
    const open = source.indexOf('{', at)
    if (open === -1) return out
    let depth = 0
    let close = -1
    for (let i = open; i < source.length; i += 1) {
      if (source[i] === '{') depth += 1
      else if (source[i] === '}') {
        depth -= 1
        if (depth === 0) {
          close = i
          break
        }
      }
    }
    if (close === -1) throw new Error('unterminated @media block')
    cursor = close + 1
  }
}

const shell = read('shell.module.css')
const rail = read('rail.module.css')
const composer = read('../conversation/composer.module.css')

describe('Drawer touch budget (≤600px)', () => {
  it('grows the drawer navigation rows to 44px', () => {
    expect(bodyFor(narrowBlock(shell), '.navItem')).toMatch(/min-height:\s*44px/)
  })

  it('grows the theme switch segments to 44px and centers the label box', () => {
    const label = bodyFor(narrowBlock(shell), '.ant-segmented-item-label')
    expect(label).toMatch(/min-height:\s*44px/)
    expect(label).toMatch(/align-items:\s*center/)
  })

  it('grows the AntD drawer close control, which the global .ant-btn rule misses', () => {
    const close = bodyFor(narrowBlock(shell), '.ant-drawer-close')
    expect(close).toMatch(/min-width:\s*44px/)
    expect(close).toMatch(/min-height:\s*44px/)
  })

  it('grows project rows, session rows, row actions and the archive toggle', () => {
    const block = narrowBlock(rail)
    for (const selector of ['.projectButton', '.sessionButton', '.archiveToggle']) {
      expect(bodyFor(block, selector)).toMatch(/min-height:\s*44px/)
    }
    expect(bodyFor(block, '.archiveRow > button:first-child')).toMatch(/min-height:\s*44px/)
    expect(bodyFor(block, '.archiveRow :global(.ant-btn)')).toMatch(/min-height:\s*44px/)
  })

  it('keeps the desktop rows compact: base rules carry no 44px floor', () => {
    for (const [source, selector] of [
      [shell, '.navItem'],
      [rail, '.projectButton'],
      [rail, '.sessionButton'],
      [rail, '.archiveToggle'],
    ] as const) {
      expect(exactBody(outsideMedia(source), selector)).not.toMatch(/min-height/)
    }
  })
})

describe('Composer touch budget (≤600px)', () => {
  it('grows the mode/model selects to 44px', () => {
    const block = narrowBlock(composer)
    expect(bodyFor(block, '.ant-select-single')).toMatch(/min-height:\s*44px/)
    expect(bodyFor(block, '.ant-select-selector')).toMatch(/min-height:\s*44px/)
    expect(bodyFor(block, '.ant-select-selection-item')).toMatch(/line-height:\s*42px/)
  })

  it('keeps the desktop composer bar compact: base rules carry no 44px floor', () => {
    expect(exactBody(outsideMedia(composer), '.bar')).not.toMatch(/min-height/)
  })
})
