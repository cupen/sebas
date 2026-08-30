// Entry point: registers the app shell and every routed view.
import './app-shell.js'
import './views/dashboard.js'
import './views/sessions.js'
import './views/session-detail.js'
import './views/settings.js'
import './views/gateway.js'
import './views/about.js'
import './views/admin/admin.js'
import './views/admin/login.js'

// Web Awesome theme + base styles (self-hosted, no CDN).
import '@awesome.me/webawesome/dist/styles/webawesome.css'
import '@awesome.me/webawesome/dist/styles/themes/default.css'
// sebas's theme mapping on top of Web Awesome (indigo brand, dark surfaces).
import './styles/wa-overrides.css'

// Dark is the default theme (html ships with class="wa-dark"); an explicit
// light system preference switches both Web Awesome and the sebas tokens.
// tokens.css mirrors the same logic in pure CSS.
if (typeof window.matchMedia === 'function') {
  const lightMq = window.matchMedia('(prefers-color-scheme: light)')
  const applyTheme = () => {
    document.documentElement.classList.toggle('wa-dark', !lightMq.matches)
  }
  applyTheme()
  lightMq.addEventListener('change', applyTheme)
}
