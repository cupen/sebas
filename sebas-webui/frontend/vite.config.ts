import { defineConfig } from 'vitest/config'

// Dev workflow: two processes. The Rust backend (axum) serves the JSON API,
// WebSocket, and health check; Vite serves the SPA here with hot reload and
// proxies the backend surfaces so the app is deployed and developed against
// identical paths.
//
// Backend target resolution (highest priority first):
//   1. `SEBAS_WEBUI_BACKEND` env var — full origin, e.g.
//      `http://127.0.0.1:9879` (sandbox on a non-default port).
//   2. Mode default: `vite --mode sandbox` (`pnpm dev:sandbox`) → 9877, the
//      throwaway sandbox instance (AGENTS.md 联调 section); any other mode
//      (incl. plain `pnpm dev`) → 9797, the watchdog-managed instance.
const backendForMode = (mode: string): string =>
  mode === 'sandbox' ? 'http://127.0.0.1:9877' : 'http://127.0.0.1:9797'

export default defineConfig(({ mode }) => {
  const backend = process.env.SEBAS_WEBUI_BACKEND || backendForMode(mode)
  const wsBackend = backend.replace(/^http/, 'ws')
  return {
    server: {
      // Dedicated port: other projects on this machine run their own Vite on
      // the default 5173 — a fixed, non-default port avoids cross-project
      // collisions and keeps the URL stable across restarts.
      port: 5273,
      strictPort: true,
      proxy: {
        '/api': {
          target: backend,
          changeOrigin: false,
        },
        // Gateway BFF mutation surface (POST/PUT/DELETE only; reads stay on
        // /api/gateway). The backend's loopback-origin guard accepts the vite
        // dev origin — both are 127.0.0.1.
        '/gateway': {
          target: backend,
          changeOrigin: false,
        },
        '/ws': {
          target: wsBackend,
          ws: true,
        },
        '/health': {
          target: backend,
        },
      },
    },
    build: {
      outDir: 'dist',
      sourcemap: false,
    },
    test: {
      environment: 'happy-dom',
      include: ['src/**/*.test.ts'],
    },
  }
})
