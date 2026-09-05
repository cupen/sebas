/**
 * 登录视图：webui 启用登录鉴权（服务端配置了凭据）时的全屏门禁。
 *
 * Shell 在 `/api/auth/me` 返回 `authenticated: false` 时渲染本组件替代整个
 * 工作台。提交 → `api.authLogin` → 成功后冒泡 `login-success`（携带用户名），
 * shell 据此挂回工作台。401（密码错）就地显示错误文案，429 显示限速提示。
 */

import { LitElement, css, html } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { ApiError, api } from '../api/client.js'

@customElement('sebas-login')
export class SebasLogin extends LitElement {
  /** 服务端配置的账户名提示（登录页可预填用户名；null = 不预填）。 */
  @property() hintUsername: string | null = null
  @state() private username = ''
  @state() private password = ''
  @state() private error: string | null = null
  @state() private busy = false

  static styles = css`
    :host {
      display: grid;
      place-items: center;
      width: 100%;
      height: 100%;
      min-height: 0;
      background: var(--sebas-bg);
      background-image: radial-gradient(1100px 480px at 82% -12%, rgba(91, 100, 242, 0.09), transparent 62%),
        radial-gradient(900px 420px at -8% 108%, rgba(56, 209, 221, 0.05), transparent 60%);
      background-attachment: fixed;
      color: var(--sebas-text);
    }
    .card {
      width: min(360px, calc(100vw - 48px));
      box-sizing: border-box;
      padding: var(--sebas-space-6, 28px);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg, 14px);
      background: var(--sebas-surface);
      box-shadow: var(--sebas-shadow-2, 0 18px 48px rgba(0, 0, 0, 0.35));
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-4, 16px);
    }
    .brand {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3, 12px);
      color: var(--sebas-text-bright);
      text-decoration: none;
    }
    .brand .mark {
      display: grid;
      place-items: center;
      width: 32px;
      height: 32px;
      border-radius: var(--sebas-radius-md, 10px);
      background: linear-gradient(135deg, var(--sebas-accent-strong, #6366f1), #4338ca);
      color: var(--sebas-accent-ink, #fff);
      font-family: var(--sebas-font-mono, monospace);
      font-weight: 700;
      box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18);
    }
    .brand .name {
      font-weight: 700;
      font-size: 1.05rem;
    }
    .brand .name small {
      display: block;
      font-weight: 500;
      font-size: 0.64rem;
      letter-spacing: 0.09em;
      text-transform: uppercase;
      color: var(--sebas-text-faint);
    }
    .title {
      margin: 0;
      font-size: 0.92rem;
      font-weight: 600;
      color: var(--sebas-text-dim);
    }
    form {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3, 12px);
    }
    label {
      display: flex;
      flex-direction: column;
      gap: 6px;
      font-size: 0.8rem;
      font-weight: 550;
      color: var(--sebas-text-dim);
    }
    input {
      padding: 9px 11px;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md, 10px);
      background: var(--sebas-surface-2, rgba(255, 255, 255, 0.03));
      color: var(--sebas-text-bright);
      font: inherit;
      outline: none;
      transition: border-color var(--sebas-dur, 150ms) var(--sebas-ease, ease);
    }
    input:focus-visible {
      border-color: var(--sebas-accent-strong, #6366f1);
    }
    button {
      margin-top: 4px;
      padding: 10px 12px;
      border: none;
      border-radius: var(--sebas-radius-md, 10px);
      background: linear-gradient(135deg, var(--sebas-accent-strong, #6366f1), #4338ca);
      color: var(--sebas-accent-ink, #fff);
      font: inherit;
      font-weight: 650;
      cursor: pointer;
      transition: filter var(--sebas-dur, 150ms) var(--sebas-ease, ease);
    }
    button:hover:enabled {
      filter: brightness(1.08);
    }
    button:disabled {
      opacity: 0.6;
      cursor: default;
    }
    .error {
      margin: 0;
      font-size: 0.8rem;
      color: #f87171;
      min-height: 1.1em;
    }
    :focus-visible {
      outline: var(--sebas-focus-ring, 2px solid rgba(99, 102, 241, 0.7));
      outline-offset: 2px;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    if (this.hintUsername) this.username = this.hintUsername
  }

  private async submit(e: Event): Promise<void> {
    e.preventDefault()
    if (this.busy) return
    this.busy = true
    this.error = null
    try {
      await api.authLogin(this.username.trim(), this.password)
      this.dispatchEvent(
        new CustomEvent('login-success', {
          detail: { username: this.username.trim() },
          bubbles: true,
          composed: true,
        }),
      )
    } catch (err) {
      this.error =
        err instanceof ApiError && err.status === 429
          ? '尝试次数过多，请稍后再试'
          : '用户名或密码错误'
    } finally {
      this.busy = false
    }
  }

  render() {
    return html`
      <div class="card">
        <div class="brand">
          <span class="mark" aria-hidden="true">❯</span>
          <span class="name">sebas<small>agent router</small></span>
        </div>
        <p class="title">登录以继续</p>
        <form @submit=${this.submit}>
          <label>
            用户名
            <input
              name="username"
              autocomplete="username"
              required
              .value=${this.username}
              @input=${(e: Event) => (this.username = (e.target as HTMLInputElement).value)}
            />
          </label>
          <label>
            密码
            <input
              name="password"
              type="password"
              autocomplete="current-password"
              required
              .value=${this.password}
              @input=${(e: Event) => (this.password = (e.target as HTMLInputElement).value)}
            />
          </label>
          <p class="error" role="alert">${this.error ?? ''}</p>
          <button type="submit" ?disabled=${this.busy}>
            ${this.busy ? '登录中…' : '登录'}
          </button>
        </form>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-login': SebasLogin
  }
}
