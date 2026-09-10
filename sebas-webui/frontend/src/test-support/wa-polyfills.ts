/**
 * 共享的 WA 渲染垫片（webui 前端单测）。
 *
 * jsdom / happy-dom 缺少 Web Awesome 组件在渲染期依赖的部分 DOM API，
 * 缺一个就会在 vitest 里变成未处理 rejection——用例本身通过，但整轮退出码
 * 为 1。当前覆盖两类：
 *
 *   1. `ElementInternals`：缺 `setFormValue` / `setValidity` 等，WA 的
 *      表单关联组件（wa-button / wa-input / wa-textarea …）在 `firstUpdated`
 *      里调用会抛 `TypeError: this.internals.setValidity is not a function`。
 *   2. `HTMLDialogElement`：缺 `showModal` / `show` / `close`，wa-dialog 在
 *      open 时抛 `TypeError: this.dialog.showModal is not a function`。
 *   3. `Element.getAnimations`：jsdom 无 Web Animations API，WA 的过渡代码
 *      抛 `TypeError: el.getAnimations is not a function`。
 *
 * 约定：任何渲染 WA 组件的测试文件，都应在导入被测模块前调用一次
 * `installWaDomPolyfills()`。安装幂等（原型标记），重复调用安全。
 */

const NOOP_INTERNALS_METHODS = [
  'setFormValue',
  'setValidity',
  'reportValidity',
  'checkValidity',
  'formStateRestoreCallback',
  'formResetCallback',
  'formDisabledCallback',
] as const

type AttachInternalsProto = {
  attachInternals?: (this: HTMLElement) => unknown
  __sebasWrappedAttachInternals?: boolean
}

let polyfillInvoked = false

/**
 * 包装 `HTMLElement.prototype.attachInternals`，让 WA 的表单关联组件在
 * jsdom/happy-dom 下也能走到稳定状态。原始实现（若存在）作为原型基座被
 * 委托调用；缺失成员补成 no-op 或平凡值。
 */
export function installElementInternalsPolyfill(): void {
  const proto = HTMLElement.prototype as unknown as AttachInternalsProto
  if (proto.__sebasWrappedAttachInternals) return
  const origAttach = proto.attachInternals
  proto.attachInternals = function (this: HTMLElement): unknown {
    let base: object = {}
    try {
      const r = origAttach?.call(this)
      if (r && typeof r === 'object') base = r as object
    } catch {
      /* non-custom-element or shim rejected */
    }
    const internals: Record<string, unknown> = Object.create(base)
    for (const name of NOOP_INTERNALS_METHODS) {
      if (typeof internals[name] !== 'function') internals[name] = () => {}
    }
    if (!('validity' in internals)) {
      internals.validity = { valid: true, valueMissing: false, customError: false }
    }
    if (!('willValidate' in internals)) internals.willValidate = false
    if (!('labels' in internals)) internals.labels = []
    if (!('form' in internals)) internals.form = null
    if (!('validationMessage' in internals)) internals.validationMessage = ''
    polyfillInvoked = true
    return internals
  }
  proto.__sebasWrappedAttachInternals = true
}

/** 垫片是否至少被真实调用过一次（供测试做 sanity-check）。 */
export function elementInternalsPolyfillInvoked(): boolean {
  return polyfillInvoked
}

type DialogProto = {
  showModal?: (this: HTMLDialogElement) => void
  show?: (this: HTMLDialogElement) => void
  close?: (this: HTMLDialogElement, returnValue?: string) => void
  __sebasPatchedDialog?: boolean
}

/**
 * 补 `HTMLDialogElement` 的 `showModal` / `show` / `close` 最小实现：
 * 维护 `open` 属性并派发 open/close 事件——wa-dialog 只依赖这些语义。
 */
export function installDialogPolyfill(): void {
  if (typeof HTMLDialogElement === 'undefined') return
  const proto = HTMLDialogElement.prototype as unknown as DialogProto
  if (proto.__sebasPatchedDialog) return
  if (typeof proto.showModal !== 'function') {
    proto.showModal = function (this: HTMLDialogElement): void {
      this.setAttribute('open', '')
      this.dispatchEvent(new Event('open'))
    }
  }
  if (typeof proto.show !== 'function') {
    proto.show = function (this: HTMLDialogElement): void {
      this.setAttribute('open', '')
      this.dispatchEvent(new Event('open'))
    }
  }
  if (typeof proto.close !== 'function') {
    proto.close = function (this: HTMLDialogElement, returnValue?: string): void {
      if (returnValue !== undefined) this.returnValue = returnValue
      this.removeAttribute('open')
      this.dispatchEvent(new Event('close'))
    }
  }
  proto.__sebasPatchedDialog = true
}

type AnimationProto = {
  getAnimations?: (this: Element) => unknown[]
  __sebasPatchedAnimations?: boolean
}

/**
 * 补 `Element.prototype.getAnimations`（jsdom 无 Web Animations API）。
 * WA 的过渡/弹窗代码会同步调用它并读返回数组，空数组即可让流程继续。
 */
export function installWebAnimationsPolyfill(): void {
  if (typeof Element === 'undefined') return
  const proto = Element.prototype as unknown as AnimationProto
  if (proto.__sebasPatchedAnimations) return
  if (typeof proto.getAnimations !== 'function') proto.getAnimations = () => []
  proto.__sebasPatchedAnimations = true
}

/**
 * 一次性装好 WA 渲染所需的全部垫片——**测试文件首选入口**。
 * 新增 jsdom 缺口时只改这里，避免每个测试文件各漏一处。
 */
export function installWaDomPolyfills(): void {
  installElementInternalsPolyfill()
  installDialogPolyfill()
  installWebAnimationsPolyfill()
}
