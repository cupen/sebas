/**
 * （fix-webui-qa-round2 2.3，M-C6）模式描述文案快照：allow 与 auto 同映射
 * bypass tier（permission-flow spec 明文），描述 SHALL 如实陈述等价——不
 * 捏造差异。四词菜单的标签与 wire 值同处钉住（simplify-mode-menus 契约）。
 */
import { describe, expect, it } from 'vitest'
import { MODE_OPTIONS, modeBadgeLabel } from './mode-vocabulary.js'

describe('mode descriptions state behavioral equivalence honestly (round2 2.3)', () => {
  it('exactly four entries with stable wire values and labels', () => {
    expect(MODE_OPTIONS.map((m) => m.value)).toEqual(['ask', 'edit', 'allow', 'auto'])
    expect(MODE_OPTIONS.map((m) => m.label)).toEqual(['Ask', 'Edit', 'Allow', 'Auto'])
  })

  it('allow and auto declare each other equivalent and promise no difference', () => {
    const allow = MODE_OPTIONS.find((m) => m.value === 'allow')!
    const auto = MODE_OPTIONS.find((m) => m.value === 'auto')!
    expect(allow.description).toContain('等价')
    expect(auto.description).toContain('等价')
    // 互相点名：allow 的描述提到 Auto，auto 的描述提到 Allow。
    expect(allow.description).toContain('Auto')
    expect(auto.description).toContain('Allow')
    // 同为全放行档：两段描述同词（同一门控行为，不暗示差异）。
    expect(allow.description).toContain('全部放行')
    expect(auto.description).toContain('全部放行')
    // 旧文案的差异暗示词退役。
    expect(auto.description).not.toContain('不门控')
    expect(allow.description).not.toContain('放行并留审计')
  })

  it('ask/edit descriptions keep their gating semantics distinct', () => {
    const ask = MODE_OPTIONS.find((m) => m.value === 'ask')!
    const edit = MODE_OPTIONS.find((m) => m.value === 'edit')!
    expect(ask.description).toContain('逐次询问')
    expect(edit.description).toContain('编辑')
    // 等价声明只在 allow/auto 之间。
    expect(ask.description).not.toContain('等价')
    expect(edit.description).not.toContain('等价')
  })

  it('mode badge labels stay short-form (unchanged contract)', () => {
    expect(modeBadgeLabel('ask')).toBe('逐次询问')
    expect(modeBadgeLabel('edit')).toBe('自动接受编辑')
    expect(modeBadgeLabel('allow')).toBe('放行')
    expect(modeBadgeLabel('auto')).toBe('自动执行')
    expect(modeBadgeLabel('whatever')).toBe('whatever')
  })
})
