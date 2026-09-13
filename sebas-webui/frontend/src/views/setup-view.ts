/**
 * 首启设置视图（add-webui-multiuser-rbac 5.2 / spec「首启 root 引导」）：
 * 鉴权开启且用户库零用户时，`/api/auth/me` 报告 `needs_setup: true`，shell
 * 以本页替代登录页/工作台。运营者在此自定义 root 用户名+密码（替代旧的
 * 自动生成随机密码引导），提交 → `api.authSetup` → 成功即建立会话进入
 * 工作台（冒泡 `setup-success`，shell 据此重探身份）。
 *
 * 就地校验优先：用户名缺省、密码 <8 字符、两次输入不一致都不发请求；
 * 服务端 400（弱密码/鉴权关闭）与 409（已有用户，抢注失败）取响应 error
 * 字段就地展示。
 */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { ApiError, api } from '../api/client.js'

/** 服务端与前端共用的最小密码长度（spec：不满足即 400，不做静默降级）。 */
export const MIN_PASSWORD_LENGTH = 8

@customElement('sebas-setup')
export class SebasSetup extends LitElement {
  @state() private username = ''
  @state() private password = ''
  @state() private confirm = ''
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
    .hint {
      margin: -8px 0 0;
      font-size: 0.78rem;
      color: var(--sebas-text-faint);
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

  /**
   * 提交前的就地校验（不发请求）：返回错误文案或 null。弱密码阈值与
   * 服务端 400 同一标准（MIN_PASSWORD_LENGTH），双保险而非替代。
   */
  private validate(): string | null {
    if (!this.username.trim()) return '请输入用户名'
    if (this.password.length < MIN_PASSWORD_LENGTH)
      return `密码至少需要 ${MIN_PASSWORD_LENGTH} 个字符`
    if (this.confirm !== this.password) return '两次输入的密码不一致'
    return null
  }

  private async submit(e: Event): Promise<void> {
    e.preventDefault()
    if (this.busy) return
    const invalid = this.validate()
    if (invalid) {
      this.error = invalid
      return
    }
    this.busy = true
    this.error = null
    try {
      const res = await api.authSetup(this.username.trim(), this.password)
      this.dispatchEvent(
        new CustomEvent('setup-success', {
          detail: { username: res.username },
          bubbles: true,
          composed: true,
        }),
      )
    } catch (err) {
      if (err instanceof ApiError) {
        // 400 弱密码/鉴权关闭、409 已有用户：文案取响应 error 字段就地展示。
        this.error = err.status === 409 ? `初始化被拒绝：${err.message}` : err.message
      } else {
        this.error = '网络连接失败，请检查服务是否可用'
      }
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
        <p class="title">创建管理员账户</p>
        <p class="hint">这是此实例的第一个账户（root），之后可在 Settings 内管理其他用户。</p>
        <form @submit=${this.submit}>
          <label>
            用户名
            <input
              name="username"
              type="text"
              autocomplete="username"
              autofocus
              required
              .value=${this.username}
              @input=${(e: Event) => (this.username = (e.target as HTMLInputElement).value)}
            />
          </label>
          <label>
            密码（至少 ${MIN_PASSWORD_LENGTH} 个字符）
            <input
              name="password"
              type="password"
              autocomplete="new-password"
              required
              .value=${this.password}
              @input=${(e: Event) => (this.password = (e.target as HTMLInputElement).value)}
            />
          </label>
          <label>
            确认密码
            <input
              name="confirm"
              type="password"
              autocomplete="new-password"
              required
              .value=${this.confirm}
              @input=${(e: Event) => (this.confirm = (e.target as HTMLInputElement).value)}
            />
          </label>
          <p class="error" role="alert">${this.error ?? ''}</p>
          <button type="submit" ?disabled=${this.busy}>
            ${this.busy ? '创建中…' : '创建并进入'}
          </button>
        </form>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-setup': SebasSetup
  }
}
