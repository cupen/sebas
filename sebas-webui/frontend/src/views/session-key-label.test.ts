// fix-webui-qa-round8 4.4：编码会话标识的展示层友好化（纯函数单测）。
import { describe, expect, it } from 'vitest'
import { friendlySessionKey } from './session-key-label.js'

describe('friendlySessionKey (agent-workbench 编码会话标识展示友好化)', () => {
  it('decodes web keys to 渠道 · 本地段', () => {
    expect(friendlySessionKey('web%00web-1709-0')).toBe('Web · web-1709-0')
  })

  it('decodes feishu (native) keys', () => {
    expect(friendlySessionKey('feishu%00agent-e449d52f')).toBe('飞书 · agent-e449d52f')
  })

  it('takes the channel and first local segment for threaded keys', () => {
    expect(friendlySessionKey('web%00ref%00thread')).toBe('Web · ref')
  })

  it('handles already-decoded NUL separators', () => {
    expect(friendlySessionKey('web\0web-1')).toBe('Web · web-1')
  })

  it('returns unknown channels verbatim', () => {
    expect(friendlySessionKey('slack%00chan-1')).toBe('slack · chan-1')
  })

  it('never blanks the display spot: unparseable input passes through', () => {
    expect(friendlySessionKey('')).toBe('')
    expect(friendlySessionKey('no-separator')).toBe('no-separator')
    // 无效 percent 序列（decodeURIComponent 抛错）→ 原样返回。
    expect(friendlySessionKey('web%ZZ')).toBe('web%ZZ')
  })
})
