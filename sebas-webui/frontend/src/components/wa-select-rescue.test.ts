// @vitest-environment jsdom
/**
 * wa-select 浮层残留清理（fix-webui-qa-defects-round3 1.2，第三轮修订）。
 *
 * jsdom 没有 Popover API，更没有浏览器「指针序列期间 hidePopover 吞 click」
 * 的 dismiss 行为——本文件按契约分两层钉住：
 *
 * 行为层（假 WA 元素按 WA 源码同构模拟：wa-popup 的 active 反应链在微任务里
 * 跑 stop() → hidePopover()，且对已摘注册/已隐藏的容器照真实浏览器抛
 * NotSupportedError）：
 *   - 残留判定：open=false + wa-popup[active] = 残留；真开（open=true）与
 *     已收好（popup 未 active）都不是
 *   - 摘层：popover 属性移除（脱离注册）+ hidePopover 短路（WA 的 stop()
 *     收尾绝不抛异常、绝不触发真 hidePopover）+ active 复位 + listbox 补
 *     hidden；0ms 后恢复 popover 注册与原型 hidePopover（下一轮 open 的
 *     showPopover 路径原样可用）
 *   - composedPath 派发：只检测路径里的残留 select；无 select / 正常 select
 *     零干预；多个残留各收各的
 *   - 文档级安装：change 与 pointerdown 都是「同步只检测、动作经 0ms 定时器
 *     延后」；定时器落地时重查判定（用户已重新打开的 select 不碰）；卸载后
 *     不再干预
 *
 * 契约层（源码读回，wa-overrides.test.ts 同款）：
 *   - rescue 模块不含任何 hidePopover( 调用（dismiss 吞 click 的逃逸线）
 *   - change 后清理路径存在（document 级 change 捕获监听）
 *   - 摘层入口只有一个调用点、且在 0ms 定时器内（pointerdown 序列内不可能
 *     同步改 popover 状态）
 *   - main.ts 顶层装配
 */

import { afterEach, describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  findStuckSelectPopupsInPath,
  hasStuckSelectPopup,
  installWaSelectRescue,
  rescueStuckSelectPopup,
  type WaSelectRescueProbe,
} from './wa-select-rescue.js'

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0))

/**
 * 假 div[popover]：按 HTML 规范同构——popover 属性移除时 showing 的容器经
 * 属性变更路径同步隐藏；showPopover/hidePopover 对「未注册或状态不符」抛
 * NotSupportedError（真实浏览器行为，钉住摘层路径绝不触发真 hide 入口）。
 */
class FakePopoverDiv extends HTMLElement {
  static get observedAttributes(): string[] {
    return ['popover']
  }

  showing = true
  hidePopoverCalls = 0

  attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void {
    if (name !== 'popover' || oldValue === newValue) return
    if (newValue === null && this.showing) this.showing = false
  }

  showPopover(): void {
    if (!this.hasAttribute('popover') || this.showing) {
      throw new DOMException('showPopover on a non-registered/visible popover', 'NotSupportedError')
    }
    this.showing = true
  }

  hidePopover(): void {
    if (!this.hasAttribute('popover') || !this.showing) {
      throw new DOMException('hidePopover on a non-showing popover', 'NotSupportedError')
    }
    this.showing = false
    this.hidePopoverCalls += 1
  }
}

/**
 * 假 wa-popup：shadow 里挂 div.popup[popover=manual]（top-layer 真身）；
 * `active` 的反应链按 WA 源码同构——值变化时在微任务里跑 updated() →
 * stop() → hidePopover()（Lit 的 updated 不吞异常，这里记 flag 供断言）。
 */
class FakeWaPopup extends HTMLElement {
  private activeState = false
  stopCalls = 0
  threwInStop = false
  updateComplete: Promise<void> = Promise.resolve()
  popup: FakePopoverDiv | null = null

  constructor() {
    super()
    const root = this.attachShadow({ mode: 'open' })
    const inner = document.createElement('fake-popover-div') as unknown as FakePopoverDiv
    inner.className = 'popup'
    root.appendChild(inner)
    this.popup = inner
  }

  connectedCallback(): void {
    if (this.popup && !this.popup.hasAttribute('popover')) {
      this.popup.setAttribute('popover', 'manual')
    }
  }

  get active(): boolean {
    return this.activeState
  }

  set active(v: boolean) {
    if (v === this.activeState) return // Lit：值未变不触发更新（也就没有第二次 stop）
    this.activeState = v
    this.updateComplete = Promise.resolve().then(() => {
      if (!v) this.stop()
    })
  }

  stop(): void {
    this.stopCalls += 1
    try {
      // WA 原样：this.popup?.hidePopover?.()——对已摘注册的容器会抛。
      this.popup?.hidePopover?.()
    } catch (err) {
      this.threwInStop = true
      throw err
    }
  }
}

/** 假 wa-select：shadow 里挂 .listbox 与 wa-popup，open 状态可控。 */
class FakeSelect extends HTMLElement {
  open = false
  private popupEl: FakeWaPopup | null = null
  listbox: HTMLElement

  constructor() {
    super()
    const root = this.attachShadow({ mode: 'open' })
    this.listbox = document.createElement('div')
    this.listbox.className = 'listbox'
    root.appendChild(this.listbox)
  }

  connectedCallback(): void {
    if (!this.popupEl) {
      // 模块的路径匹配与探针都按真标签（wa-popup）找：显式经 unknown 双跳转。
      this.popupEl = document.createElement('wa-popup') as unknown as FakeWaPopup
      this.shadowRoot!.appendChild(this.popupEl)
    }
  }

  /** 测试辅助：把浮层置为 active（模拟 showPopover 后的状态）。 */
  activatePopup(): void {
    if (this.popupEl) this.popupEl.active = true
  }

  popupElement(): FakeWaPopup | null {
    return this.popupEl
  }
}

// 假元素注册在**真标签** wa-select / wa-popup 下：模块按 localName 严格等值
// 查找（本文件不导入真组件，独立测试环境内注册安全）。
if (!customElements.get('fake-popover-div')) customElements.define('fake-popover-div', FakePopoverDiv)
if (!customElements.get('wa-popup')) customElements.define('wa-popup', FakeWaPopup)
if (!customElements.get('wa-select')) customElements.define('wa-select', FakeSelect)

async function makeSelect(open: boolean, popupActive: boolean): Promise<FakeSelect> {
  const el = document.createElement('wa-select') as unknown as FakeSelect
  document.body.appendChild(el)
  el.open = open
  if (popupActive) el.activatePopup()
  await flush() // 自定义元素反应（popover 属性置位等）落地
  return el
}

function asProbe(el: FakeSelect): WaSelectRescueProbe {
  return el as unknown as WaSelectRescueProbe
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('hasStuckSelectPopup / rescueStuckSelectPopup', () => {
  it('flags open=false with an active popup as stuck, and detaches it without ever calling hidePopover', async () => {
    const el = await makeSelect(false, true)
    const popup = el.popupElement()!
    const inner = popup.popup!
    expect(hasStuckSelectPopup(asProbe(el))).toBe(true)

    expect(rescueStuckSelectPopup(asProbe(el))).toBe(true)
    // 同步面：脱离 popover 注册 + hidePopover 短路 + active 复位 + listbox
    // 补 hidden（display:none 退出命中测试）。
    expect(inner.hasAttribute('popover')).toBe(false)
    expect(typeof (inner as unknown as Record<string, unknown>)['hidePopover']).toBe('function')
    expect(popup.active).toBe(false)
    expect(el.listbox.hidden).toBe(true)
    // WA 的 stop() 收尾（微任务）走了 no-op 短路：绝不抛 NotSupportedError，
    // 也绝不触发真 hidePopover（真身从未被 hide 入口碰过）。
    await popup.updateComplete
    expect(popup.threwInStop).toBe(false)
    expect(inner.hidePopoverCalls).toBe(0)

    // 0ms 后恢复可复用：popover 属性重新注册、原型 hidePopover 还原——
    // 下一轮 open 的 showPopover 路径原样可用。
    await flush()
    expect(inner.showing).toBe(false) // 属性移除已经让它退出 showing 态
    expect(inner.hasAttribute('popover')).toBe(true)
    expect((inner as unknown as Record<string, unknown>)['hidePopover']).toBe(
      FakePopoverDiv.prototype.hidePopover,
    )
    expect(() => inner.showPopover()).not.toThrow()
    expect(inner.showing).toBe(true)

    // 残留已清：再探为 false，重复 rescue 不再动作。
    expect(hasStuckSelectPopup(asProbe(el))).toBe(false)
    expect(rescueStuckSelectPopup(asProbe(el))).toBe(false)
  })

  it('re-checks the stuck predicate when the deferred cleanup lands (user re-opened the select)', async () => {
    const el = await makeSelect(false, true)
    const popup = el.popupElement()!
    const uninstall = installWaSelectRescue(document)
    const event = new PointerEvent('pointerdown', { bubbles: true, composed: true })
    Object.defineProperty(event, 'target', { value: el })
    el.dispatchEvent(event)
    // 定时器窗口里用户把下拉重新打开 = 正常交互，清理必须放行。
    el.open = true
    await flush()
    expect(popup.active).toBe(true)
    uninstall()
  })

  it('never touches a genuinely open select (option picks must survive)', async () => {
    // open=true + popup active = 正常交互中的下拉：摘层会破坏选择——判定必须放行。
    const el = await makeSelect(true, true)
    const popup = el.popupElement()!
    const inner = popup.popup!
    expect(hasStuckSelectPopup(asProbe(el))).toBe(false)
    expect(rescueStuckSelectPopup(asProbe(el))).toBe(false)
    expect(popup.active).toBe(true)
    expect(inner.hasAttribute('popover')).toBe(true)
    expect(el.listbox.hidden).toBe(false)
  })

  it('never touches a cleanly closed select', async () => {
    const el = await makeSelect(false, false)
    expect(hasStuckSelectPopup(asProbe(el))).toBe(false)
    expect(rescueStuckSelectPopup(asProbe(el))).toBe(false)
  })
})

describe('findStuckSelectPopupsInPath', () => {
  it('collects stuck selects found in the path', async () => {
    const stuck = await makeSelect(false, true)
    const wrapper = document.createElement('div')
    wrapper.appendChild(stuck)
    document.body.appendChild(wrapper)

    // composedPath 以 wrapper→stuck 命中为形状（浮层挡路时其宿主在路径里）。
    const found = findStuckSelectPopupsInPath([stuck.listbox, stuck, wrapper, document.body])
    expect(found).toHaveLength(1)
    expect(found[0]).toBe(asProbe(stuck))
  })

  it('does nothing when the path carries no stuck wa-select (residue not blocking)', async () => {
    const residue = await makeSelect(false, true) // 残留存在但不在路径里 = 没挡路
    const open = await makeSelect(true, true) // 真开的 select 在路径里也不碰
    expect(findStuckSelectPopupsInPath([document.body])).toHaveLength(0)
    expect(findStuckSelectPopupsInPath([open, document.body])).toHaveLength(0)
    expect(residue.popupElement()!.active).toBe(true)
    expect(open.popupElement()!.active).toBe(true)
  })

  it('collects each stuck select in the path', async () => {
    const a = await makeSelect(false, true)
    const b = await makeSelect(false, true)
    expect(findStuckSelectPopupsInPath([a, b])).toHaveLength(2)
  })

  it('ignores non-element path entries (text nodes etc.)', async () => {
    const junk = ['not-a-node', null, document.createTextNode('x')] as unknown as EventTarget[]
    expect(findStuckSelectPopupsInPath(junk)).toHaveLength(0)
  })
})

describe('installWaSelectRescue', () => {
  async function dispatchPointerDownOn(el: FakeSelect): Promise<void> {
    const event = new PointerEvent('pointerdown', { bubbles: true, composed: true })
    Object.defineProperty(event, 'target', { value: el })
    el.dispatchEvent(event)
  }

  async function dispatchChangeOn(el: FakeSelect): Promise<void> {
    const event = new Event('change', { bubbles: true, composed: true })
    Object.defineProperty(event, 'target', { value: el })
    el.dispatchEvent(event)
  }

  it('change after a pick schedules the cleanup — detected synchronously, applied after dispatch', async () => {
    const stuck = await makeSelect(false, true)
    const popup = stuck.popupElement()!
    const inner = popup.popup!
    const uninstall = installWaSelectRescue(document)

    await dispatchChangeOn(stuck)
    // 同步面零改动：检测 + 记录而已，绝不在当前事件序列内动 popover 状态。
    expect(inner.hasAttribute('popover')).toBe(true)
    expect(popup.active).toBe(true)
    expect(stuck.listbox.hidden).toBe(false)
    // 0ms 后：摘层落地。
    await flush()
    expect(inner.hasAttribute('popover')).toBe(false)
    expect(popup.active).toBe(false)
    expect(stuck.listbox.hidden).toBe(true)
    uninstall()
  })

  it('pointerdown fallback stays detect-only in the event, cleanup lands on a timer', async () => {
    const stuck = await makeSelect(false, true)
    const popup = stuck.popupElement()!
    const inner = popup.popup!
    const uninstall = installWaSelectRescue(document)

    await dispatchPointerDownOn(stuck)
    // 指针序列内：只检测 + 记录，popover 状态纹丝不动（round3 第三轮根因）。
    expect(inner.hasAttribute('popover')).toBe(true)
    expect(popup.active).toBe(true)
    expect(stuck.listbox.hidden).toBe(false)
    await flush()
    expect(inner.hasAttribute('popover')).toBe(false)
    expect(popup.active).toBe(false)
    uninstall()
  })

  it('change on an element with no stuck wa-select in the path is a no-op', async () => {
    const residue = await makeSelect(false, true) // 残留存在但不在 change 路径里
    const uninstall = installWaSelectRescue(document)
    const outsider = document.createElement('input')
    document.body.appendChild(outsider)
    const event = new Event('change', { bubbles: true, composed: true })
    Object.defineProperty(event, 'target', { value: outsider })
    outsider.dispatchEvent(event)
    await flush()
    expect(residue.popupElement()!.active).toBe(true)
    uninstall()
  })

  it('stops after uninstall', async () => {
    const stuck = await makeSelect(false, true)
    const popup = stuck.popupElement()!
    const uninstall = installWaSelectRescue(document)

    await dispatchPointerDownOn(stuck)
    await flush()
    expect(popup.active).toBe(false)

    // 重新制造残留（恢复流程已把 popover 属性装回，直接翻 active 即可），
    // 卸载后 pointerdown 与 change 都不再干预。
    await flush() // 上一次清理的恢复定时器落地
    stuck.activatePopup()
    uninstall()
    await dispatchPointerDownOn(stuck)
    await dispatchChangeOn(stuck)
    await flush()
    expect(popup.active).toBe(true)
  })
})

// ── 源码契约（round3 1.2 第三轮）───────────────────────────────────────────
// jsdom 复现不了 popover dismiss 吞 click，修复面用源码读回契约钉死：摘层
// 手段里绝不允许再出现 hidePopover( 调用；change 后清理路径存在；摘层入口
// 唯一且在定时器内（指针序列内不可能同步改 popover 状态）。

describe('source contracts (round3 1.2, third revision)', () => {
  const here = dirname(fileURLToPath(import.meta.url))
  const src = readFileSync(join(here, 'wa-select-rescue.ts'), 'utf8')
  // 去注释后的代码面（块注释与行注释都可能提及 API 名，先剥掉再断言）。
  const code = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')

  it('contains no hidePopover( call anywhere (the dismiss-swallow escape hatch)', () => {
    expect(code).not.toMatch(/hidePopover\s*\(/)
  })

  it('wires a document-level capture change listener (cleanup right after the pick)', () => {
    expect(code).toMatch(/addEventListener\(\s*'change'\s*,\s*onChange\s*,\s*true\)/)
    expect(code).toMatch(/addEventListener\(\s*'pointerdown'\s*,\s*onPointerDown\s*,\s*true\)/)
  })

  it('has exactly one rescue call site and it runs inside a 0ms timer (never in the pointer sequence)', () => {
    // 定义处 + 定时器回调内的唯一调用点。
    const callSites = code.match(/rescueStuckSelectPopup\(/g) ?? []
    expect(callSites).toHaveLength(2)
    expect(code).toMatch(/setTimeout\(\s*\(\)\s*=>\s*\{[\s\S]*?rescueStuckSelectPopup\(/)
  })
})

// ── 应用入口装配契约（round3 1.2）─────────────────────────────────────────
// 模块级单测再完备，入口没装配也是零效果——挂掉的修复长得和没修一样。本
// 文件不导入 main.ts（它还牵路由/主题/WS 等副作用），按仓库既有的源码读回
// 契约（wa-overrides.test.ts 同款）钉住装配。

describe('app entry wiring (round3 1.2)', () => {
  const here = dirname(fileURLToPath(import.meta.url))
  const mainSrc = readFileSync(join(here, '..', 'main.ts'), 'utf8')

  it('main.ts installs the rescue at document level', () => {
    expect(mainSrc).toMatch(
      /import\s*\{[^}]*installWaSelectRescue[^}]*\}\s*from\s*'\.\/components\/wa-select-rescue(\.js)?'/,
    )
    // 顶层实际调用（非注释）：文档级单点安装是修复生效的全部前提。
    const withoutComments = mainSrc.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
    expect(withoutComments).toMatch(/^\s*installWaSelectRescue\(\)/m)
  })
})
