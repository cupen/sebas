/**
 * wa-select 浮层残留的清理（fix-webui-qa-defects-round3 1.2，第三轮修订：
 * 根因改判为 popover dismiss 吞 click，修复随之换面——design 附录「缺陷 1」）。
 *
 * ## 根因（真实浏览器回归定位）
 *
 * 浏览器对 Popover API 有 dismiss 兼容语义：**popover 的 hidePopover() 在
 * 一条指针事件序列（pointerdown → pointerup → click）期间被调用时，该序列
 * 的 click 会被吞掉**（防止「关闭弹层的这一下点击」同时击穿到下层控件）。
 * new-session 弹窗里「操作过 wa-select 后第一次点创建无任何效果、第二次才
 * 行」正是踩中它，两个来源：
 *   1. WA 的收起链是异步的（`open=false` → `animateWithClass(popup, "hide")`
 *      → `listbox.hidden` → `popup.active=false` → wa-popup `updated()` →
 *      `stop()` → `hidePopover()`），且 `animateWithClass` 有永不 settle 的
 *      早退分支——`hidePopover()` 可能拖到下一次指针交互期间才落地；
 *   2. 前两轮的 rescue 自己挂在 document pointerdown 捕获阶段**同步**调
 *      `hidePopover()`——恰好在指针序列内亲手制造吞点击（这就是「换原生
 *      button」救不了的原因：吞点击发生在 popover 层，命中测试本来就到得了
 *      按钮，换控件实现无从幸免）。
 *
 * ## 本轮修复面（三条一起，绕开 dismiss 吞 click）
 *
 *   1. **摘层不再调用 hidePopover()**：对残留 popup 容器（wa-popup 影子里的
 *      `div[popover]`，top-layer 真身）直接 `removeAttribute('popover')`——
 *      脱离 popover 注册（规范的属性变更路径同步退出 top layer，不走
 *      hidePopover 入口），之后的任何操作都与 dismiss 吞 click 无缘；随后
 *      照常压灭视觉态（`active=false` → `popup-active` class 撤下 →
 *      display:none；`listbox.hidden` 补齐挂起链的剩余步骤）。
 *   2. **挂载点提前**：document 级 `change` 捕获监听（change 事件 composed，
 *      沿 composedPath 找 wa-select），收起链一启动就经 0ms 定时器清掉该
 *      select 的残留——把残留消灭在下一次用户点击**之前**，而不是等下一次
 *      pointerdown 才发现。
 *   3. **pointerdown 捕获兜底只做「检测 + 记录」**：命中残留也只把清理动作
 *      排进 0ms 定时器，当前事件分发完成后才动 popover 状态——绝不在指针
 *      序列内同步改 popover 状态（前两轮的教训）。
 *
 * WA 自身会在 `active` 翻 false 时经 `updated()` → `stop()` 调
 * `hidePopover()`；容器脱离注册后那条调用只会抛 NotSupportedError（还会
 * 掐断 stop() 的收尾）。摘层时同步把容器上的 hidePopover 短路成 no-op，
 * 待 WA 的收尾微任务跑完（0ms 定时器）再恢复原型方法与 popover 属性——
 * 下一轮 open 的 `showPopover()` 路径原样可用，WA 与摘层互不踩踏。
 *
 * 判定刻意收窄，零误伤：
 *   - 真开着的下拉（`open === true`）不碰——点选项的 mouseup 在 pointerdown
 *     之后，提前摘层会破坏选项选择；
 *   - 路径里没有 wa-select（残留没挡路）就什么都不做；
 *   - `open === false` 且浮层已收好也不碰——正常收起链已走完。
 * 清理动作全部经 0ms 定时器执行，且 `rescueStuckSelectPopup` 落地时重查判定
 * （定时器窗口里用户可能已把下拉重新打开——那是正常交互，不许干预）。
 */

/** 结构化探针：只读 WA select 上本模块用到的面（测试用假对象即可）。 */
export interface WaSelectRescueProbe {
  open: boolean
  shadowRoot: { querySelector(selectors: string): Element | null } | null
}

/** wa-popup 的本模块可写面（`active` 是公开反射属性，`popup` 是模板里的 div[popover]）。 */
interface PopupProbe extends Element {
  active: boolean
  /** wa-popup 模板里的 `div[popover]`（top-layer 真身）。 */
  popup?: Element | null
}

/** select 浮层的 wa-popup（不存在或未 active 返回 null）。 */
function activePopup(select: WaSelectRescueProbe): PopupProbe | null {
  const el = select.shadowRoot?.querySelector('wa-popup') as PopupProbe | null | undefined
  return el && el.active ? el : null
}

/** 「select 已收起（open === false）但浮层仍 active」= 不变量被破坏的残留态。 */
export function hasStuckSelectPopup(select: WaSelectRescueProbe): boolean {
  return !select.open && activePopup(select) !== null
}

/**
 * 摘除一个残留浮层。返回 true = 确实清了东西。**只在 0ms 定时器里跑**：
 * 摘层会改 popover 注册状态，绝不允许落在指针序列内（否则正是 dismiss 吞
 * click 的根因）；执行前重查残留判定，定时器窗口里已恢复正常的 select
 * 原样跳过。
 */
export function rescueStuckSelectPopup(select: WaSelectRescueProbe): boolean {
  if (!hasStuckSelectPopup(select)) return false
  const popup = activePopup(select) as PopupProbe
  const inner = popup.popup ?? null
  if (inner) {
    // ① 脱离 popover 注册：属性变更路径同步退出 top layer，不走 hidePopover
    //    入口——那条入口正是浏览器 dismiss 吞 click 的触发点。
    inner.removeAttribute('popover')
    // ② WA 自身会在 active 翻 false 时 stop() → hidePopover()；容器已脱离
    //    注册，那条调用只会抛 NotSupportedError——短路成 no-op，让 WA 的
    //    收尾微任务安静走完（恢复见 ④）。
    ;(inner as unknown as Record<string, unknown>)['hidePopover'] = () => {}
  }
  // ③ 视觉收尾 + WA 状态复位：active=false → wa-popup 渲染撤下 popup-active
  //    （display:none 立即生效、退出命中测试）→ hasStuckSelectPopup 判定复位。
  popup.active = false
  const listbox = select.shadowRoot?.querySelector('.listbox')
  if (listbox) (listbox as HTMLElement).hidden = true
  if (inner) {
    // ④ 0ms 后恢复：WA 的 stop() 收尾微任务此时已跑完。原型 hidePopover
    //    回位（下一次正常关闭还要用）、popover 属性重新注册（下一次 open
    //    的 showPopover 需要它在案）——select 的后续可复用性不受摘层影响。
    setTimeout(() => {
      const slot = inner as unknown as Record<string, unknown>
      delete slot['hidePopover']
      inner.setAttribute('popover', 'manual')
    }, 0)
  }
  return true
}

/**
 * 从一次指针事件的 composedPath 里收集残留（不变量被破坏：open === false 但
 * 浮层仍 active）的 wa-select。残留若真的挡了这次交互，其宿主必然在命中路径
 * 里；没挡路 = 路径里没有残留 select = 零干预。**只检测，不动状态**——同步
 * 摘除留给定时器（见 installWaSelectRescue）。
 */
export function findStuckSelectPopupsInPath(path: readonly EventTarget[]): WaSelectRescueProbe[] {
  const stuck: WaSelectRescueProbe[] = []
  for (const node of path) {
    if (!(node instanceof HTMLElement) || node.localName !== 'wa-select') continue
    const select = node as unknown as WaSelectRescueProbe
    if (hasStuckSelectPopup(select)) stuck.push(select)
  }
  return stuck
}

/**
 * 文档级安装（round3 1.2 第三轮）：
 *   - `change` 捕获监听：wa-select 的收起链在 change 后立刻启动，这里沿
 *     composedPath 找到该 select、经 0ms 定时器清残留——抢在下一次用户点击
 *     之前，pointerdown 兜底因此只在收起链挂死（change 之后仍残留）时才接手；
 *   - `pointerdown` 捕获兜底：只做「检测 + 记录」，清理动作同样排进 0ms
 *     定时器——绝不在指针序列内同步改 popover 状态。
 * 返回卸载函数。
 */
export function installWaSelectRescue(doc: Document = document): () => void {
  const schedule = (path: readonly EventTarget[]): void => {
    const stuck = findStuckSelectPopupsInPath(path)
    if (stuck.length === 0) return
    setTimeout(() => {
      for (const select of stuck) rescueStuckSelectPopup(select)
    }, 0)
  }
  const onPointerDown = (e: PointerEvent): void => schedule(e.composedPath())
  const onChange = (e: Event): void => schedule(e.composedPath())
  doc.addEventListener('change', onChange, true)
  doc.addEventListener('pointerdown', onPointerDown, true)
  return () => {
    doc.removeEventListener('change', onChange, true)
    doc.removeEventListener('pointerdown', onPointerDown, true)
  }
}
