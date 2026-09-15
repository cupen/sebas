/**
 * wa-hide 来源守卫（redesign-provider-models-settings 2.1 / design D6）：
 * Web Awesome 子控件（如 `<wa-select>`）收起自身列表框时会冒泡 composed
 * `wa-hide`，`<wa-dialog>` 会把它误判为「用户要关我」——点选一个选项，
 * 整个弹窗跟着关闭。守卫只在事件源是对话框自身时才执行关闭，子控件冒泡
 * 上来的 hide 一律忽略。全站所有 `<wa-dialog>` 的 `@wa-hide` 都应挂本守卫。
 */
export function guardedHide(close: () => void): (e: Event) => void {
  return (e: Event) => {
    if (e.target === e.currentTarget) close()
  }
}
