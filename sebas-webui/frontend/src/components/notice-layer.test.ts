/**
 * sebas-notice-layer 单测（add-webui-tiered-notices 2.1/2.3 + 3.1 焦点）：
 * - 四级→wa-toast 变体/时长/accent 映射与 live role（danger=alert/assertive，
 *   其余 status/polite）；fatal 不走 toast；
 * - 栈上限挤占的层内同步（store 移除 → 元素出栈）与手动关闭回写；
 * - 布局：横幅列与 toast 栈的叠放次序、`--sebas-notice-top-offset` 偏移、
 *   z-index 读数；fatal 焦点移入/恢复归还（attribute/元素层断言，不模拟
 *   焦点漫游——design Risks 约定）。
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'
import { dismiss, notify, resetNotices, setFatal, setWsDown } from '../notify.js'
import { CORE_FATAL_BANNER_TESTID, SebasNoticeLayer, WS_DOWN_BANNER_TESTID } from './notice-layer.js'
import './notice-layer.js'
import type WaToastItem from '@awesome.me/webawesome/dist/components/toast-item/toast-item.js'
import type { CSSResult } from 'lit'

installWaDomPolyfills()

/**
 * 组件样式的读取面：happy-dom 的 adoptedStyleSheets 是空壳（cssText 读不出
 * 内容），直接断言 `static styles` 的 authored CSS（cssText）——与运行环境
 * 无关，钉的是「样式表契约」。
 */
function componentCss(component: { styles: unknown }): string {
  const styles = component.styles
  if (Array.isArray(styles)) return styles.map((s) => (s as CSSResult).cssText).join('\n')
  return (styles as CSSResult).cssText
}

const layerCss = () => componentCss(SebasNoticeLayer)

/** rAF×2 + 宏任务：WA 的 hide 动画垫片路径（getAnimations=[]）一拍落定。 */
async function flush(): Promise<void> {
  await new Promise((r) => requestAnimationFrame(() => r(null)))
  await new Promise((r) => requestAnimationFrame(() => r(null)))
  await new Promise((r) => setTimeout(r, 0))
}

let layer: SebasNoticeLayer

async function mountLayer(): Promise<SebasNoticeLayer> {
  const el = document.createElement('sebas-notice-layer') as SebasNoticeLayer
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

const root = () => layer.shadowRoot!
const toastItems = () => [...root().querySelectorAll<WaToastItem>('wa-toast-item')]
const banner = (testid: string) => root().querySelector(`sebas-notice-banner[data-testid="${testid}"]`)

beforeEach(async () => {
  resetNotices()
  layer = await mountLayer()
})

afterEach(async () => {
  resetNotices()
  layer.remove()
  await flush()
})

describe('四级→WA 变体/时长/accent 映射（D2 表）', () => {
  it('info → brand, 5s, sebas info accent', async () => {
    notify({ level: 'info', message: '核心已恢复' })
    await layer.updateComplete
    const [item] = toastItems()
    expect(item).toBeTruthy()
    expect(item.getAttribute('variant')).toBe('brand')
    expect(item.duration).toBe(5_000)
    expect(item.style.getPropertyValue('--accent-color')).toBe('var(--sebas-notice-info)')
    expect(item.textContent).toContain('核心已恢复')
  })

  it('warn → warning, 8s, sebas warn accent', async () => {
    notify({ level: 'warn', message: '操作失败：HTTP 504' })
    await layer.updateComplete
    const [item] = toastItems()
    expect(item.getAttribute('variant')).toBe('warning')
    expect(item.duration).toBe(8_000)
    expect(item.style.getPropertyValue('--accent-color')).toBe('var(--sebas-notice-warn)')
  })

  it('error → danger, duration 0 (驻留), sebas error accent', async () => {
    notify({ level: 'error', message: '视图不可用' })
    await layer.updateComplete
    const [item] = toastItems()
    expect(item.getAttribute('variant')).toBe('danger')
    expect(item.duration).toBe(0)
    expect(item.style.getPropertyValue('--accent-color')).toBe('var(--sebas-notice-error)')
  })

  it('fatal never produces a toast — banner + lock overlay instead', async () => {
    setFatal({ kind: 'startup_failed', cause: 'socket absent' })
    await layer.updateComplete
    expect(toastItems()).toHaveLength(0)
    expect(banner(CORE_FATAL_BANNER_TESTID)).toBeTruthy()
    expect(root().querySelector('[data-testid="core-lock-overlay"]')).toBeTruthy()
  })

  it('live role mapping: the layer pins the WA variants whose announce roles differ', async () => {
    // danger→alert/assertive、其余→status/polite 的广播是 wa-toast 内建行为；
    // vitest 解析的 lit 是 node 构（isServer=true），wa-toast 跳过 live
    // region 装载——该面只在真浏览器可断言（5.2 旅程覆盖），这里钉住层选
    // 用的变体词表（role 语义由 WA 按 variant 内建）。
    notify({ level: 'info', message: 'one' })
    notify({ level: 'error', message: 'two' })
    await layer.updateComplete
    const variants = toastItems().map((it) => it.getAttribute('variant'))
    expect(variants).toEqual(['brand', 'danger'])
  })
})

describe('栈行为：挤占出栈、驻留手动关、去重', () => {
  it('a fourth transient item evicts the oldest element from the stack', async () => {
    notify({ level: 'info', message: 'a' })
    notify({ level: 'info', message: 'b' })
    notify({ level: 'info', message: 'c' })
    notify({ level: 'info', message: 'd' })
    await layer.updateComplete
    await flush()
    expect(toastItems()).toHaveLength(3)
    const texts = toastItems().map((it) => it.textContent)
    expect(texts.some((t) => t?.includes('a'))).toBe(false)
    expect(texts.some((t) => t?.includes('d'))).toBe(true)
  })

  it('an error toast stays until closed; closing it writes back to the store', async () => {
    const id = notify({ level: 'error', message: '驻留错误' })!
    await layer.updateComplete
    const item = toastItems()[0]
    expect(item.duration).toBe(0)
    // 手动关闭（WA 3.12.0 内建关闭钮，无 closable 属性）。
    const close = item.shadowRoot!.querySelector('[part="close-button"]') as HTMLButtonElement
    expect(close).toBeTruthy()
    close.click()
    await flush()
    expect(toastItems()).toHaveLength(0)
    // 回写：store 亦空（dismiss 已触发）。
    const { subscribeNotices } = await import('../notify.js')
    const seen: number[] = []
    const unsub = subscribeNotices((s) => seen.push(s.items.length))
    unsub()
    expect(seen[0]).toBe(0)
    void id
  })

  it('the same message within the dedupe window does not spawn a second toast', async () => {
    notify({ level: 'warn', message: 'dup' })
    notify({ level: 'warn', message: 'dup' })
    await layer.updateComplete
    expect(toastItems()).toHaveLength(1)
  })

  it('store-side dismiss (eviction path) hides the element', async () => {
    const id = notify({ level: 'info', message: 'evict-me' })!
    await layer.updateComplete
    expect(toastItems()).toHaveLength(1)
    dismiss(id)
    await layer.updateComplete
    await flush()
    expect(toastItems()).toHaveLength(0)
  })
})

describe('层布局（D1/D7）：叠放次序、top-offset、z-index', () => {
  it('banner column stacks before the toast stack in the layer', () => {
    const children = [...root().children]
    const banners = children.findIndex((c) => c.classList.contains('banners'))
    const stack = children.findIndex((c) => c.localName === 'wa-toast')
    expect(banners).toBeGreaterThanOrEqual(0)
    expect(stack).toBeGreaterThan(banners)
  })

  it('measures the banner column into --sebas-notice-top-offset (0 without banners)', async () => {
    // 无横幅：偏移 0px；toast 栈的纵向位置由层样式表的变量承接。
    await layer.updateComplete
    expect(layer.style.getPropertyValue('--sebas-notice-top-offset')).toBe('0px')
    expect(layerCss()).toContain('inset-block-start: var(--sebas-notice-top-offset, 0px)')

    // 横幅在场：偏移 = 横幅列实测高度（happy-dom 无布局，垫 offsetHeight 读数）。
    setWsDown(true)
    setFatal({ kind: 'disconnected', cause: 'dropped' })
    await layer.updateComplete
    const banners = root().querySelector('.banners') as HTMLElement
    Object.defineProperty(banners, 'offsetHeight', { configurable: true, value: 58 })
    layer.requestUpdate()
    await layer.updateComplete
    expect(layer.style.getPropertyValue('--sebas-notice-top-offset')).toBe('58px')
    // warn + fatal 两横幅纵向同列（叠放次序）。
    expect(banner(WS_DOWN_BANNER_TESTID)).toBeTruthy()
    expect(banner(CORE_FATAL_BANNER_TESTID)).toBeTruthy()
  })

  it('sits above the settings-modal overlay (z 100) and above the app (z 99 lock)', () => {
    // D1「实测值 +1」落数字：settings-modal overlay = 100 → 横幅 101。
    expect(layerCss()).toContain(`z-index: ${101}`)
    expect(layerCss()).toContain(`z-index: ${99}`)
  })

  it('banner column ignores pointer events when empty; banners stay interactive', () => {
    expect(layerCss()).toMatch(/\.banners\s*\{[^}]*pointer-events:\s*none/)
    expect(layerCss()).toMatch(/\.banners\s*>\s*sebas-notice-banner\s*\{[^}]*pointer-events:\s*auto/)
  })
})

describe('fatal 焦点管理（3.1，attribute/元素层）', () => {
  it('moves focus into the fatal banner on lock and returns it on recovery', async () => {
    const elsewhere = document.createElement('button')
    document.body.appendChild(elsewhere)
    elsewhere.focus()
    expect(document.activeElement).toBe(elsewhere)

    setFatal({ kind: 'auth_rejected', cause: 'rejected' })
    await layer.updateComplete
    await flush()
    // 焦点已移入层内的横幅（happy-dom 对 shadow 内焦点回投到宿主链）。
    expect([document.activeElement, root().activeElement]).toContain(layer)

    setFatal(null)
    await layer.updateComplete
    // 恢复归还此前焦点（元素仍在文档中）。
    expect(document.activeElement).toBe(elsewhere)
    elsewhere.remove()
  })

  it('returns focus to body when the previous focus element is gone', async () => {
    const temp = document.createElement('button')
    document.body.appendChild(temp)
    temp.focus()
    setFatal({ kind: 'disconnected' })
    await layer.updateComplete
    temp.remove()
    setFatal(null)
    await layer.updateComplete
    expect(document.activeElement).toBe(document.body)
  })
})
