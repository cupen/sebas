// fix-webui-qa-round8 7.5：技能预览剥离 frontmatter（纯函数单测）。
import { describe, expect, it } from 'vitest'
import { stripFrontmatter } from './skill-frontmatter.js'

describe('stripFrontmatter (agent-skills 技能预览只渲染正文)', () => {
  it('strips a leading --- block and keeps the body', () => {
    const md = '---\nname: demo\ndescription: 演示\n---\n\n# 正文\n\n说明文字。\n'
    expect(stripFrontmatter(md)).toBe('# 正文\n\n说明文字。\n')
  })

  it('tolerates a BOM and CRLF', () => {
    const md = '﻿---\r\nname: demo\r\n---\r\nbody line\r\n'
    expect(stripFrontmatter(md)).toBe('body line\r\n')
  })

  it('returns plain bodies untouched', () => {
    expect(stripFrontmatter('# no frontmatter\nbody')).toBe('# no frontmatter\nbody')
    // 正文里出现的 --- 不构成头块（首行不是 ---）。
    expect(stripFrontmatter('text\n---\nmore')).toBe('text\n---\nmore')
  })

  it('an unterminated opening fence is not treated as frontmatter', () => {
    const md = '---\nname: demo\nno closing fence'
    expect(stripFrontmatter(md)).toBe(md)
  })
})
