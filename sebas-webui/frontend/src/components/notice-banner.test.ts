/**
 * sebas-notice-banner 单测（add-webui-tiered-notices 2.2）：fatal 文案按
 * kind 三档 + cause 原文小字 + kind 缺失退化通用；warn 断线态；role=alert、
 * 不可关（无关闭控件）、tabindex=-1 可聚焦。颜色不是唯一通道（图标 + 文案）。
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'
import './notice-banner.js'
import { SebasNoticeBanner, fatalTitle } from './notice-banner.js'
import { resetNotices } from '../notify.js'
import type { CSSResult } from 'lit'

installWaDomPolyfills()

/**
 * 组件样式读取面：happy-dom 的 adoptedStyleSheets 是空壳，直接断言
 * `static styles` 的 authored CSS（cssText）——钉「样式表契约」。
 */
function styleText(el: SebasNoticeBanner): string {
  const styles = (el.constructor as typeof SebasNoticeBanner).styles
  if (Array.isArray(styles)) return styles.map((s) => (s as CSSResult).cssText).join('\n')
  return (styles as CSSResult).cssText
}

beforeEach(() => resetNotices())

async function mount(level: 'warn' | 'fatal', message: string, cause?: string | null) {
  const el = document.createElement('sebas-notice-banner') as SebasNoticeBanner
  el.level = level
  el.message = message
  if (cause !== undefined) el.cause = cause
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

describe('fatal banner kind 分档（D5）', () => {
  it('startup_failed / auth_rejected / disconnected each get their own headline', async () => {
    const cases: Array<[string, string]> = [
      ['startup_failed', '核心启动失败'],
      ['auth_rejected', '核心拒绝接入'],
      ['disconnected', '与核心的连接已断开'],
    ]
    for (const [kind, headline] of cases) {
      expect(fatalTitle(kind)).toBe(headline)
      const el = await mount('fatal', fatalTitle(kind), `cause of ${kind}`)
      expect(el.shadowRoot!.textContent).toContain(headline)
      el.remove()
    }
  })

  it('a missing kind degrades to the generic headline', async () => {
    expect(fatalTitle(undefined)).toBe('核心不可达')
    expect(fatalTitle('something-new')).toBe('核心不可达')
    const el = await mount('fatal', fatalTitle(undefined))
    expect(el.shadowRoot!.textContent).toContain('核心不可达')
    el.remove()
  })

  it('keeps the cause verbatim as small print and omits the line when absent', async () => {
    const withCause = await mount('fatal', fatalTitle('disconnected'), 'dial tcp 127.0.0.1:9797: connect: connection refused')
    expect(withCause.shadowRoot!.querySelector('.cause')?.textContent).toBe(
      'dial tcp 127.0.0.1:9797: connect: connection refused',
    )
    withCause.remove()

    const noCause = await mount('fatal', fatalTitle(undefined), null)
    expect(noCause.shadowRoot!.querySelector('.cause')).toBeNull()
    noCause.remove()
  })
})

describe('banner 语义与形态（spec「全局核心可达性横幅」）', () => {
  it('renders role=alert, is NOT closable, and is focusable via tabindex=-1', async () => {
    const el = await mount('fatal', fatalTitle('startup_failed'), 'boom')
    const inner = el.shadowRoot!.querySelector('.banner')!
    expect(inner.getAttribute('role')).toBe('alert')
    // 不可手动关闭：横幅内没有任何关闭控件（区别于 toast 的内建关闭钮）。
    expect(el.shadowRoot!.querySelector('button')).toBeNull()
    // tabindex=-1（连接回调装上）：锁定时焦点可移入，不进 Tab 序。
    expect(el.getAttribute('tabindex')).toBe('-1')
    el.remove()
  })

  it('warn and fatal levels carry distinct surfaces + icon (colour is not the only channel)', async () => {
    const warn = await mount('warn', '与服务器的连接已断开，正在重连…')
    expect(warn.shadowRoot!.querySelector('.banner')?.classList.contains('warn')).toBe(true)
    expect(styleText(warn)).toContain('var(--sebas-notice-warn-bg)')
    warn.remove()

    const fatal = await mount('fatal', fatalTitle('disconnected'))
    expect(fatal.shadowRoot!.querySelector('.banner')?.classList.contains('fatal')).toBe(true)
    expect(styleText(fatal)).toContain('var(--sebas-notice-fatal-bg)')
    // 图标在文案之外并行传达级别。
    expect(fatal.shadowRoot!.querySelector('svg')).toBeTruthy()
    fatal.remove()
  })
})
