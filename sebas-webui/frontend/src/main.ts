// Entry point: registers the app shell and every routed view. The settings
// modal renders its sections in place (no routed settings/router pages —
// those routes redirect to / in IA v2); the about/admin views are deleted.
// session-detail 已退休（workbench-conversation-view 3.4）：/sessions/:key
// 深链由 dashboard 渲染聚焦态。
import './app-shell.js'
import './views/dashboard.js'
import './views/sessions.js'

// Web Awesome theme + base styles (self-hosted, no CDN).
import '@awesome.me/webawesome/dist/styles/webawesome.css'
import '@awesome.me/webawesome/dist/styles/themes/default.css'
// （fix-webui-approval-restore-and-session-identity 5.4，design D8）图标本地
// 化：`<wa-icon>` 缺省从外部图标 CDN 拉 SVG（离线/受限网络 403 + 破图）。
// setIconPath 指到同源 `/icons`——打包进 dist 的本地子集（仅实际用到的
// folder / spinner，见 public/icons/），不再出网。
import { setIconPath } from '@awesome.me/webawesome/dist/utilities/base-path.js'
setIconPath('/icons')
// sebas's theme mapping on top of Web Awesome (indigo brand, dark surfaces).
import './styles/wa-overrides.css'

// Theme: `wa-dark` on <html> is the single switch (dark is the default; the
// mode lives in src/theme.ts and index.html applies it before first paint).
// System mode live-follows an OS preference change.
import { applyThemeMode } from './theme.js'
applyThemeMode()
if (typeof window.matchMedia === 'function') {
  window
    .matchMedia('(prefers-color-scheme: light)')
    .addEventListener('change', applyThemeMode)
}
