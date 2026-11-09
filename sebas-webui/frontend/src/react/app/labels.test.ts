import { describe, expect, it } from 'vitest'
import { relativeTime, sessionLabel, statusLabel, statusTone } from './labels.js'

describe('React shell labels', () => {
  it('prefers operator label, then prompt preview, then the readable placeholder', () => {
    const base = { label: null, prompt_preview: null, session_id_short: 's1', encoded_key: 'oc_1' }
    expect(sessionLabel({ ...base, label: '我的会话' })).toBe('我的会话')
    expect(sessionLabel({ ...base, prompt_preview: 'first prompt' })).toBe('first prompt')
    // Machine identifiers (short id / encoded-key tail) are not display names.
    expect(sessionLabel(base)).toBe('未命名会话')
    expect(
      sessionLabel({ ...base, session_id_short: null, encoded_key: 'web%00web-1709-4' }),
    ).toBe('未命名会话')
  })

  it('never surfaces an undecoded encoded key for a placeholder row', () => {
    const placeholder = {
      label: null,
      prompt_preview: null,
      session_id_short: 'web%00web-1794174660061252841-62',
      encoded_key: 'web%00web-1794174660061252841-62',
    }
    const text = sessionLabel(placeholder)
    expect(text).toBe('未命名会话')
    expect(text).not.toContain('%00')
    // Whitespace-only values are not names either.
    expect(sessionLabel({ ...placeholder, label: '  ', prompt_preview: ' ' })).toBe('未命名会话')
  })

  it('maps machine states to Chinese labels with a non-colour channel', () => {
    expect(statusLabel('working')).toBe('运行中')
    expect(statusLabel('waiting')).toBe('等待你处理')
    expect(statusLabel('mystery')).toBe('mystery')
    expect(statusTone('done')).toBe('ok')
    expect(statusTone('failed')).toBe('err')
    expect(statusTone('waiting')).toBe('warn')
    expect(statusTone('dormant')).toBe('neutral')
  })

  it('renders relative times in Chinese', () => {
    const now = 1_700_000_000_000
    expect(relativeTime(0, now)).toBe('未知')
    expect(relativeTime(now / 1000 - 10, now)).toBe('10 秒前')
    expect(relativeTime(now / 1000 - 120, now)).toBe('2 分钟前')
    expect(relativeTime(now / 1000 - 7200, now)).toBe('2 小时前')
    expect(relativeTime(now / 1000 - 172_800, now)).toBe('2 天前')
  })
})
