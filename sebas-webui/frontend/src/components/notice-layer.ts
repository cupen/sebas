/**
 * 视口级分级通知层（add-webui-tiered-notices D1/D2/D3/D7）——
 * `sebas-notice-layer`，挂 app-shell 工作台分支根部（auth 门禁页在 shell
 * 渲染分支之外，天然不受 fatal 锁定影响）。
 *
 * 三个视口级 fixed 容器（互不嵌套——wa-toast 是 fixed popover，塞不进文档
 * 流）：
 * 1. `.banners` 持续横幅列（warn 断线 / fatal core 不可达），全宽贴顶，
 *    z-index 101（settings-modal 自绘 overlay 实测 100 + 1，D1「tasks 落数
 *    字」）；空列 `pointer-events: none` 不挡下层点击，横幅自身 auto。
 * 2. `wa-toast` 官方瞬时栈（placement="top-center"、`--width: 28rem`），
 *    `inset-block-start` 经共享 `--sebas-notice-top-offset` 下移横幅高度，
 *    两个 fixed 容器不重叠（层内 `updated()` 实测横幅列高度写回变量）。
 * 3. fatal 锁定遮罩（`--wa-color-overlay-modal` 同款半透明 + 居中原因卡与
 *    恢复提示），z-index 99——横幅在遮罩之上保持可交互。
 *
 * fatal 锁定的 inert 由 app-shell 施加在 `wa-split-panel.frame` 上（rail +
 * main 的共同祖先）；本层负责视觉遮罩与焦点管理：锁定时焦点移入横幅
 * （tabindex=-1，不进 Tab 序），恢复归还此前焦点、元素已失则落 body。
 * 断言边界（design Risks）：测试环境 happy-dom/jsdom 只断言 attribute 层。
 */
import { LitElement, css, html, nothing, render, type PropertyValues } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import {
  dismiss,
  subscribeNotices,
  type FatalNotice,
  type NoticeItem,
  type NoticeLevel,
  type NoticeState,
} from '../notify.js'
import { icon } from './icons.js'
import { fatalTitle } from './notice-banner.js'
import './notice-banner.js'
// 副效果导入（注册自定义元素）+ 纯类型别名：WaToast/WaToastItem 在本文件
// 只出现在类型位——不写成值导入会被 esbuild 当类型导入整行删除，注册就
// 永远不发生（happy-dom/jsdom 下 wa-toast 保持未升级）。
import '@awesome.me/webawesome/dist/components/toast/toast.js'
import '@awesome.me/webawesome/dist/components/toast-item/toast-item.js'
import type WaToast from '@awesome.me/webawesome/dist/components/toast/toast.js'
import type WaToastItem from '@awesome.me/webawesome/dist/components/toast-item/toast-item.js'

/** settings-modal 自绘 overlay 的实测 z-index（+1 = 本层横幅，D1）。 */
const BANNER_Z_INDEX = 101
/** fatal 锁定遮罩：压过应用内容，让位于横幅（101）。 */
const LOCK_Z_INDEX = 99

/** D2 映射表：级别 → wa-toast-item 变体 + accent token（attributes 优先、
 * token 次之，全部为 3.12.0 已核实的文档化 API）。 */
const LEVEL_STYLE: Record<NoticeLevel, { variant: WaToastItem['variant']; accent: string; iconName: string }> = {
  info: { variant: 'brand', accent: 'var(--sebas-notice-info)', iconName: 'about' },
  warn: { variant: 'warning', accent: 'var(--sebas-notice-warn)', iconName: 'alert' },
  error: { variant: 'danger', accent: 'var(--sebas-notice-error)', iconName: 'alert' },
}

/** fatal 横幅的 testid（旧 `core-unreachable-banner` 的迁移落点，5.1）。 */
export const CORE_FATAL_BANNER_TESTID = 'core-fatal-banner'
/** /ws 断线持续 warn 横幅的 testid。 */
export const WS_DOWN_BANNER_TESTID = 'ws-down-banner'

@customElement('sebas-notice-layer')
export class SebasNoticeLayer extends LitElement {
  @state() private items: NoticeItem[] = []
  @state() private wsDown = false
  @state() private fatal: FatalNotice | null = null

  private unsubscribe: (() => void) | null = null
  /** 已入栈的 toast 条目（store id → 元素），双向同步的差集基础。 */
  private toastEls = new Map<number, WaToastItem>()
  /** 锁定开始前被聚焦的元素（恢复时归还；元素已失则落 body）。 */
  private restoreFocus: HTMLElement | null = null
  /** 锁定开始 → 渲染完成后把焦点移入横幅（updated 里消费）。 */
  private focusPending = false

  static styles = css`
    :host {
      /* 纯容器：三个子容器各自 fixed，宿主不占布局。 */
      display: contents;
    }
    /* ── 持续横幅列（D1）：全宽贴顶，纵缝堆叠（warn + fatal 同场不遮挡） */
    .banners {
      position: fixed;
      top: var(--sebas-notice-top, 12px);
      left: 0;
      right: 0;
      z-index: ${BANNER_Z_INDEX};
      display: flex;
      flex-direction: column;
      gap: 6px;
      /* 空列不挡下层点击；横幅自身恢复 auto。 */
      pointer-events: none;
    }
    .banners > sebas-notice-banner {
      pointer-events: auto;
    }
    /* ── 官方瞬时栈（D2）：placement top-center + sebas 宽度；纵向位置让位
       横幅（--sebas-notice-top-offset 由 updated() 实测写回宿主）。 */
    wa-toast {
      --width: 28rem;
      z-index: ${BANNER_Z_INDEX};
      /* 内联层叠次之宿主阴影规则（wa-toast 自带 :host([placement]) 的
         inset-block-start: 0）——inline style 赢，横幅在场时整体下移。 */
      inset-block-start: var(--sebas-notice-top-offset, 0px);
    }
    /* ── fatal 锁定遮罩（D3）：--wa-color-overlay-modal 同款半透明盖住整屏，
       居中原因卡与恢复提示；横幅列 z-index 更高、保持可交互。 */
    .lock-overlay {
      position: fixed;
      inset: 0;
      z-index: ${LOCK_Z_INDEX};
      display: grid;
      place-items: center;
      padding: var(--sebas-space-4);
      background: var(--wa-color-overlay-modal, rgba(4, 6, 12, 0.72));
    }
    .lock-card {
      max-width: 30rem;
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
      padding: var(--sebas-space-5) var(--sebas-space-6);
      background: var(--sebas-notice-fatal-bg);
      color: #fff;
      border: 1px solid rgba(255, 255, 255, 0.25);
      border-radius: var(--sebas-radius-lg);
      box-shadow: var(--sebas-shadow-2);
      font-size: 0.9rem;
    }
    .lock-card .title {
      display: flex;
      align-items: center;
      gap: 8px;
      font-weight: 700;
      font-size: 1rem;
    }
    .lock-card .cause {
      font-size: 0.76rem;
      opacity: 0.88;
      word-break: break-all;
    }
    .lock-card .hint {
      font-size: 0.76rem;
      opacity: 0.8;
    }
    /* D7：≤480px 横幅本就全宽贴顶，卡片收窄内边距即可。 */
    @media (max-width: 480px) {
      .lock-card {
        max-width: none;
        width: 100%;
        padding: var(--sebas-space-4);
      }
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    // 订阅即回放当前态：登录后才挂载本层也不会错过锁定中的 fatal。
    this.unsubscribe = subscribeNotices(this.onState)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    this.unsubscribe = null
    this.toastEls.clear()
    super.disconnectedCallback()
  }

  private onState = (s: NoticeState): void => {
    const wasFatal = this.fatal !== null
    const isFatal = s.fatal !== null
    this.items = s.items
    this.wsDown = s.wsDown
    this.fatal = s.fatal
    if (isFatal && !wasFatal) {
      // 锁定开始：此刻焦点仍在锁外，先记下归还目标（元素已失则落 body）。
      this.restoreFocus =
        document.activeElement instanceof HTMLElement ? document.activeElement : null
      this.focusPending = true
    } else if (!isFatal && wasFatal) {
      const target = this.restoreFocus
      this.restoreFocus = null
      if (target && target.isConnected) target.focus()
      else document.body.focus()
    }
  }

  protected updated(changed: PropertyValues): void {
    super.updated(changed)
    this.syncToasts()
    this.measureBanners()
    if (this.focusPending && this.fatal) {
      this.focusPending = false
      this.fatalBanner()?.focus()
    }
  }

  render() {
    return html`
      ${this.fatal
        ? html`<div class="lock-overlay" data-testid="core-lock-overlay">
            <div class="lock-card">
              <span class="title"
                >${icon('alert', 16)}
                <span>${fatalTitle(this.fatal.kind)}</span></span
              >
              ${this.fatal.cause ? html`<span class="cause">${this.fatal.cause}</span>` : nothing}
              <span class="hint">会话与操作已锁定，核心恢复后将自动解锁并刷新工作台</span>
            </div>
          </div>`
        : nothing}
      <div class="banners">
        ${this.wsDown
          ? html`<sebas-notice-banner
              level="warn"
              message="与服务器的连接已断开，正在重连…（当前显示可能已过期）"
              data-testid=${WS_DOWN_BANNER_TESTID}
            ></sebas-notice-banner>`
          : nothing}
        ${this.fatal
          ? html`<sebas-notice-banner
              level="fatal"
              message=${fatalTitle(this.fatal.kind)}
              .cause=${this.fatal.cause ?? null}
              data-testid=${CORE_FATAL_BANNER_TESTID}
            ></sebas-notice-banner>`
          : nothing}
      </div>
      <wa-toast placement="top-center"></wa-toast>
    `
  }

  /** store 条目 ↔ wa-toast-item 双向同步：新增入栈、移除（挤占/手动关）出栈。 */
  private syncToasts(): void {
    const stack = this.renderRoot.querySelector('wa-toast')
    if (!stack) return
    const liveIds = new Set(this.items.map((it) => it.id))
    for (const item of this.items) {
      if (this.toastEls.has(item.id)) continue
      const style = LEVEL_STYLE[item.level]
      const el = document.createElement('wa-toast-item')
      el.variant = style.variant
      el.duration = item.duration
      // D2：左侧 4px 色条 + 图标 + 倒计时环同吃 --accent-color，sebas 化。
      el.style.setProperty('--accent-color', style.accent)
      const iconHolder = document.createElement('span')
      iconHolder.setAttribute('slot', 'icon')
      render(icon(style.iconName, 16), iconHolder)
      el.prepend(iconHolder)
      // 纯文本内容（默认槽）：XSS 面为零，错误消息里的任意字符如实呈现。
      const label = document.createElement('span')
      label.textContent = item.message
      el.append(label)
      el.addEventListener(
        'wa-after-hide',
        () => {
          // 关闭按钮（WA 内建恒在）或自动消失后的回写；store 已移除时无害。
          this.toastEls.delete(item.id)
          dismiss(item.id)
        },
        { once: true },
      )
      this.toastEls.set(item.id, el)
      // 直接子元素挂入官方栈：slotchange 自动 showStack/startTimer/announce。
      stack.appendChild(el)
    }
    for (const [id, el] of [...this.toastEls]) {
      if (liveIds.has(id)) continue
      this.toastEls.delete(id)
      if (el.isConnected) void el.hide()
    }
  }

  /** 实测横幅列高度 → 宿主 `--sebas-notice-top-offset`（toast 栈整体下移）。 */
  private measureBanners(): void {
    const banners = this.renderRoot.querySelector('.banners')
    const height = banners instanceof HTMLElement ? banners.offsetHeight : 0
    const next = `${height}px`
    if (this.style.getPropertyValue('--sebas-notice-top-offset') !== next) {
      this.style.setProperty('--sebas-notice-top-offset', next)
    }
  }

  private fatalBanner(): HTMLElement | null {
    return this.renderRoot.querySelector<HTMLElement>(
      `sebas-notice-banner[data-testid="${CORE_FATAL_BANNER_TESTID}"]`,
    )
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-notice-layer': SebasNoticeLayer
    'wa-toast': WaToast
    'wa-toast-item': WaToastItem
  }
}
