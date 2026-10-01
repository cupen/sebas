// 会话用量芯片投影的单测（add-webui-round7-gaps 1.1，usage-statistics spec
// 「会话级 token 用量可见」）：usage 在场 = 实数呈现；缺省/null = 「未上报」
// 语义，绝不以 0 冒充。

import { describe, expect, it } from 'vitest'
import { sessionUsageView, USAGE_UNREPORTED_TEXT } from './session-usage.js'
import type { SessionUsage } from '../api/client.js'

describe('sessionUsageView（add-webui-round7-gaps 1.1）', () => {
  it('usage 在场：input/output 实数上屏，title 带累计口径', () => {
    const v = sessionUsageView({ total_input: 5200, total_output: 310 })
    expect(v.unreported).toBe(false)
    expect(v.tone).toBe('reported')
    expect(v.text).toContain('5200')
    expect(v.text).toContain('310')
    expect(v.text).not.toContain('null')
    expect(v.title).toContain('累计 input 5200')
    expect(v.title).toContain('output 310')
  })

  it('usage 带模型名：title 前缀模型，text 不变', () => {
    const v = sessionUsageView({ model: 'claude-sonnet-4-20250514', total_input: 1, total_output: 2 })
    expect(v.title.startsWith('claude-sonnet-4-20250514')).toBe(true)
    expect(v.text).toBe('Token in 1 · out 2')
  })

  it('usage 为 null / undefined：未上报语义，绝不冒充 0', () => {
    for (const usage of [null, undefined]) {
      const v = sessionUsageView(usage)
      expect(v.unreported).toBe(true)
      expect(v.tone).toBe('unreported')
      expect(v.text).toBe(USAGE_UNREPORTED_TEXT)
      expect(v.text).toMatch(/未上报/)
      expect(v.text).not.toMatch(/\b0\b/)
      expect(v.title).toContain('未上报')
    }
  })

  it('零累计（真实报告的 0）仍按实数呈现——与「未上报」是两回事', () => {
    const usage: SessionUsage = { total_input: 0, total_output: 0 }
    const v = sessionUsageView(usage)
    expect(v.unreported).toBe(false)
    expect(v.tone).toBe('reported')
    expect(v.text).toBe('Token in 0 · out 0')
  })
})
