/**
 * Admin login view. Shown whenever an admin API call returns 401; posts
 * JSON credentials and on success routes to the admin status view. The
 * session cookie is HttpOnly — nothing secret lives in JS-land.
 */

import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { api, ApiError } from '../../api/client.js'
import { navigate } from '../../router.js'
import { viewStyles } from '../../styles/shared.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'

@customElement('sebas-admin-login')
export class SebasAdminLogin extends LitElement {
  @state() private password = ''
  @state() private error = ''
  @state() private busy = false

  static styles = [
    viewStyles,
    css`
      .wrap {
        max-width: 380px;
        margin: var(--sebas-space-10) auto 0;
      }
      .card {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-2);
        padding: var(--sebas-space-6);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-4);
      }
      .lockup {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        margin-bottom: var(--sebas-space-2);
      }
      .lockup .mark {
        display: grid;
        place-items: center;
        width: 34px;
        height: 34px;
        border-radius: var(--sebas-radius-md);
        background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
        color: var(--sebas-accent-ink);
        font-family: var(--sebas-font-mono);
        font-size: 1rem;
        font-weight: 700;
        box-shadow:
          var(--sebas-shadow-1),
          inset 0 1px 0 rgba(255, 255, 255, 0.18);
      }
      .lockup h1 {
        margin: 0;
        font-size: 1.1rem;
        font-weight: 650;
        letter-spacing: -0.01em;
        color: var(--sebas-text-bright);
      }
      .lockup p {
        margin: 0;
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
      }
      .error {
        color: var(--sebas-status-failed);
        margin: 0;
        font-size: 0.875rem;
      }
    `,
  ]

  private async submit(e: Event): Promise<void> {
    e.preventDefault()
    if (this.busy || !this.password) return
    this.busy = true
    this.error = ''
    try {
      await api.adminLogin(this.password)
      this.password = ''
      navigate('/admin/status')
    } catch (err) {
      this.error =
        err instanceof ApiError && err.status === 403
          ? 'Invalid password.'
          : err instanceof ApiError && err.status === 429
            ? 'Too many attempts — wait a moment and retry.'
            : String(err)
    } finally {
      this.busy = false
    }
  }

  render() {
    return html`
      <div class="wrap">
        <form class="card" @submit=${this.submit}>
          <div class="lockup">
            <span class="mark" aria-hidden="true">❯</span>
            <div>
              <h1>Admin sign-in</h1>
              <p>sebas control plane</p>
            </div>
          </div>
          ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : ''}
          <wa-input
            type="password"
            password-toggle
            label="Password"
            autocomplete="current-password"
            value=${this.password}
            @input=${(e: Event) => (this.password = (e.target as HTMLInputElement).value)}
          ></wa-input>
          <wa-button variant="brand" appearance="accent" type="submit" ?loading=${this.busy}
            >Sign in</wa-button
          >
        </form>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-admin-login': SebasAdminLogin
  }
}
