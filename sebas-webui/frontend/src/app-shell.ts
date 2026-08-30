/**
 * App shell: sidebar navigation + routed outlet. Web Awesome supplies the
 * interactive primitives (buttons, inputs); sebas-* components own layout.
 *
 * Link interception is document-level (composedPath) so anchors rendered
 * inside any view's shadow root navigate SPA-side too — shadow retargeting
 * hides them from a shell-scoped listener.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { matchRoute, navigate, type RouteDef } from './router.js'
import { icon } from './components/icons.js'

// Web Awesome primitives used across the app (self-registered, bundler
// tree-shakes the rest of the library).
import '@awesome.me/webawesome/dist/components/button/button.js'

const ROUTES: RouteDef[] = [
  { id: 'dashboard', pattern: '/' },
  { id: 'sessions', pattern: '/sessions' },
  { id: 'session-detail', pattern: '/sessions/:key' },
  { id: 'settings', pattern: '/settings' },
  { id: 'gateway', pattern: '/gateway' },
  { id: 'about', pattern: '/about' },
  { id: 'admin-login', pattern: '/admin/login' },
  { id: 'admin', pattern: '/admin/:view' },
]

const NAV_ITEMS = [
  { label: 'Dashboard', href: '/', icon: 'dashboard' },
  { label: 'Sessions', href: '/sessions', icon: 'sessions' },
  { label: 'Settings', href: '/settings', icon: 'settings' },
  { label: 'Gateway', href: '/gateway', icon: 'gateway' },
  { label: 'About', href: '/about', icon: 'about' },
  { label: 'Admin', href: '/admin/status', icon: 'shield' },
]

@customElement('sebas-app')
export class SebasApp extends LitElement {
  @state()
  private routeId: string = 'dashboard'

  private params: Record<string, string> = {}
  private onNavigateBound: () => void = () => {}
  private onClick: (e: MouseEvent) => void = () => {}

  static styles = css`
    :host {
      display: flex;
      min-height: 100vh;
      background: var(--sebas-bg);
      color: var(--sebas-text);
    }
    nav {
      width: 232px;
      flex: 0 0 auto;
      position: sticky;
      top: 0;
      height: 100vh;
      overflow-y: auto;
      background: var(--sebas-surface);
      border-right: 1px solid var(--sebas-border);
      padding: var(--sebas-space-4) var(--sebas-space-3);
      display: flex;
      flex-direction: column;
      gap: 2px;
    }
    .brand {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-2) var(--sebas-space-2) var(--sebas-space-5);
      text-decoration: none;
      color: var(--sebas-text-bright);
    }
    .brand .mark {
      display: grid;
      place-items: center;
      width: 28px;
      height: 28px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-md);
      background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
      color: var(--sebas-accent-ink);
      font-family: var(--sebas-font-mono);
      font-size: 0.9rem;
      font-weight: 700;
      box-shadow:
        var(--sebas-shadow-1),
        inset 0 1px 0 rgba(255, 255, 255, 0.18);
    }
    .brand .name {
      font-weight: 700;
      font-size: 1rem;
      letter-spacing: 0.01em;
    }
    .brand .name small {
      display: block;
      font-weight: 500;
      font-size: 0.66rem;
      letter-spacing: 0.09em;
      text-transform: uppercase;
      color: var(--sebas-text-faint);
    }
    nav a.item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 7px 10px;
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim);
      font-size: 0.875rem;
      font-weight: 500;
      text-decoration: none;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    nav a.item svg {
      opacity: 0.8;
      flex: 0 0 auto;
    }
    nav a.item:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    nav a.item[aria-current='page'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    nav a.item[aria-current='page'] svg {
      opacity: 1;
    }
    nav a:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .spacer {
      flex: 1;
    }
    .side-foot {
      padding: var(--sebas-space-3) var(--sebas-space-2);
      color: var(--sebas-text-faint);
      font-size: 0.72rem;
      letter-spacing: 0.02em;
    }
    main {
      flex: 1;
      min-width: 0;
      padding: var(--sebas-space-8) var(--sebas-space-10) var(--sebas-space-10);
    }
    .outlet {
      max-width: 1080px;
      margin: 0 auto;
    }
    /* Route change mounts a fresh view — replay a soft rise-in. */
    .outlet > * {
      animation: sebas-view-in 0.28s var(--sebas-ease) both;
    }
    @keyframes sebas-view-in {
      from {
        opacity: 0;
        transform: translateY(6px);
      }
      to {
        opacity: 1;
        transform: none;
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .outlet > * {
        animation: none;
      }
    }
    @media (max-width: 900px) {
      main {
        padding: var(--sebas-space-6) var(--sebas-space-5);
      }
    }
    @media (max-width: 640px) {
      :host {
        flex-direction: column;
      }
      nav {
        position: static;
        height: auto;
        width: auto;
        flex-direction: row;
        align-items: center;
        flex-wrap: wrap;
        gap: var(--sebas-space-1);
        border-right: none;
        border-bottom: 1px solid var(--sebas-border);
        padding: var(--sebas-space-3) var(--sebas-space-4);
      }
      .brand {
        padding: 0 var(--sebas-space-4) 0 0;
      }
      .brand .name small {
        display: none;
      }
      nav a.item {
        padding: 6px 9px;
      }
      .spacer,
      .side-foot {
        display: none;
      }
      main {
        padding: var(--sebas-space-4);
      }
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    this.onNavigateBound = this.onNavigate.bind(this)
    this.onClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey)
        return
      for (const node of e.composedPath()) {
        if (!(node instanceof HTMLAnchorElement)) continue
        const href = node.getAttribute('href')
        if (href && href.startsWith('/')) {
          e.preventDefault()
          navigate(href)
        }
        break
      }
    }
    window.addEventListener('popstate', this.onNavigateBound)
    document.addEventListener('click', this.onClick)
    this.onNavigate()
  }

  disconnectedCallback(): void {
    window.removeEventListener('popstate', this.onNavigateBound)
    document.removeEventListener('click', this.onClick)
    super.disconnectedCallback()
  }

  private onNavigate(): void {
    const match = matchRoute(ROUTES, location.pathname)
    if (!match) {
      // Unknown path: render the dashboard rather than a dead screen.
      this.routeId = 'dashboard'
      this.params = {}
    } else {
      this.routeId = match.id
      this.params = match.params
    }
  }

  private renderOutlet() {
    switch (this.routeId) {
      case 'dashboard':
        return html`<sebas-dashboard></sebas-dashboard>`
      case 'sessions':
        return html`<sebas-sessions></sebas-sessions>`
      case 'session-detail':
        return html`<sebas-session-detail key=${this.params['key'] ?? ''}></sebas-session-detail>`
      case 'settings':
        return html`<sebas-settings></sebas-settings>`
      case 'gateway':
        return html`<sebas-gateway></sebas-gateway>`
      case 'about':
        return html`<sebas-about></sebas-about>`
      case 'admin-login':
        return html`<sebas-admin-login></sebas-admin-login>`
      case 'admin':
        return html`<sebas-admin view=${this.params['view'] ?? 'status'}></sebas-admin>`
      default:
        return html`<sebas-dashboard></sebas-dashboard>`
    }
  }

  render() {
    return html`
      <nav aria-label="Primary">
        <a class="brand" href="/" aria-label="sebas console home">
          <span class="mark" aria-hidden="true">❯</span>
          <span class="name">sebas<small>agent router</small></span>
        </a>
        ${NAV_ITEMS.map(
          (item) => html`<a
            class="item"
            href=${item.href}
            aria-current=${location.pathname === item.href ? 'page' : nothing}
          >
            ${icon(item.icon)}${item.label}</a
          >`,
        )}
        <div class="spacer"></div>
        <div class="side-foot">local agent router console</div>
      </nav>
      <main><div class="outlet">${this.renderOutlet()}</div></main>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-app': SebasApp
  }
}
