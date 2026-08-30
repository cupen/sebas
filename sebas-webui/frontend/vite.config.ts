import { defineConfig } from 'vitest/config'

// Dev workflow: two processes. The Rust backend (axum) serves the JSON API,
// WebSocket, and health check on 127.0.0.1:9797; Vite serves the SPA here
// with hot reload and proxies the backend surfaces so the app is deployed
// and developed against identical paths.
export default defineConfig({
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:9797',
        changeOrigin: false,
      },
      // Gateway BFF mutation surface (POST/PUT/DELETE only; reads stay on
      // /api/gateway). The backend's loopback-origin guard accepts the vite
      // dev origin — both are 127.0.0.1.
      '/gateway': {
        target: 'http://127.0.0.1:9797',
        changeOrigin: false,
      },
      '/ws': {
        target: 'ws://127.0.0.1:9797',
        ws: true,
      },
      '/health': {
        target: 'http://127.0.0.1:9797',
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
})
