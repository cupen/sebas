/**
 * 持续态驻留横幅（add-webui-tiered-notices D3）——`sebas-notice-banner`。
 *
 * 持续态不合身 toast（会话级存续、不可关、要承载多行 cause），复用旧
 * app-shell banner 的视觉语言（全宽条、图标 + 文案、role=alert）自研：
 * - warn 态（/ws 断线）：深琥珀底（`--sebas-notice-warn-bg`，对白字 AA）；
 * - fatal 态（core 不可达）：深红底（`--sebas-notice-fatal-bg`），文案按
 *   `kind` 三档分句 + cause 原文小字，kind 缺失退化通用「核心不可达」。
 *
 * 两条 SHALL NOT 可手动关闭（spec）——不渲染任何关闭控件；`tabindex=-1`
 * 使横幅可聚焦（fatal 锁定时焦点移入，恢复后归还）。颜色不是唯一通道：
 * 图标 + 文案 + 左缘色条三通道传达级别。
 */
import { LitElement, css, html, nothing } from 'lit'
import { customElement, property } from 'lit/decorators.js'
import { icon } from './icons.js'

export type NoticeBannerLevel = 'warn' | 'fatal'

/** fatal 横幅按 kind 分档的标题（D5）；缺失退化为通用文案。 */
const FATAL_TITLES: Record<string, string> = {
  startup_failed: '核心启动失败',
  auth_rejected: '核心拒绝接入',
  disconnected: '与核心的连接已断开',
}

export function fatalTitle(kind?: string): string {
  return FATAL_TITLES[kind ?? ''] ?? '核心不可达'
}

@customElement('sebas-notice-banner')
export class SebasNoticeBanner extends LitElement {
  /** warn = 断线类持续提示；fatal = core 不可达（配锁定遮罩）。 */
  @property() level: NoticeBannerLevel = 'warn'
  /** 主文案（warn：断线提示；fatal：按 kind 分档的标题）。 */
  @property() message = ''
  /** fatal：cause 原文小字；缺省不渲染该行（不伪造原因）。 */
  @property() cause: string | null = null

  static styles = css`
    :host {
      display: block;
    }
    .banner {
      display: flex;
      align-items: baseline;
      justify-content: center;
      flex-wrap: wrap;
      gap: 3px 8px;
      padding: 6px 12px;
      font-size: 0.8rem;
      font-weight: 500;
      color: #fff;
      /* 左缘 4px 色条：颜色不是唯一通道（图标 + 文案并行）。 */
      border-inline-start: 4px solid rgba(255, 255, 255, 0.65);
      box-shadow: var(--sebas-shadow-1);
    }
    .banner.warn {
      background: var(--sebas-notice-warn-bg);
    }
    .banner.fatal {
      background: var(--sebas-notice-fatal-bg);
      font-weight: 600;
    }
    .banner svg {
      flex: 0 0 auto;
      align-self: center;
    }
    .cause {
      font-size: 0.72rem;
      font-weight: 400;
      opacity: 0.88;
      word-break: break-all;
    }
    /* 锁定时焦点移入横幅（tabindex=-1）：焦点环用白，深底上可见。 */
    .banner:focus-visible {
      outline: 2px solid rgba(255, 255, 255, 0.9);
      outline-offset: -2px;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    // 宿主可聚焦（fatal 锁定时层把焦点移进来）：tabindex=-1 不进 Tab 序。
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '-1')
  }

  render() {
    return html`
      <div class="banner ${this.level}" role="alert">
        ${icon('alert', 14)}<span class="msg">${this.message}</span>
        ${this.cause ? html`<span class="cause">${this.cause}</span>` : nothing}
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-notice-banner': SebasNoticeBanner
  }
}
