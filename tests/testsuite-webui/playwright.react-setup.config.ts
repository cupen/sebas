import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * 首启建户装配（redesign-webui-with-react-antd 5.1/5.2）。
 *
 * 与 `playwright.react-auth.config.ts` 同一生产入口 `/`，但沙箱以
 * `TESTSUITE_AUTH_SETUP=1` 启动：auth 开、零用户（端口 9891），用来跑
 * setup 这一类页面（首启 root 引导）的真实浏览器旅程。
 * 端口与 auth-setup 的 9896 及 9893/9892 均隔离。
 */
process.env.TESTSUITE_SCENE_FILE ||= path.join(
  os.tmpdir(),
  'sebas-testsuite-webui-scene-react-9891',
)

export const REACT_SETUP_WEBUI_PORT = 9891
export const REACT_SETUP_VITE_PORT = 5288

export default defineConfig({
  testDir: './tests',
  testMatch: /react-setup\.spec\.ts/,
  timeout: 45_000,
  retries: 0,
  workers: 1,
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${REACT_SETUP_VITE_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: [
    {
      command: 'invoke testsuite-webui-server',
      env: {
        ...process.env,
        TESTSUITE_PORT: String(REACT_SETUP_WEBUI_PORT),
        TESTSUITE_AUTH_SETUP: '1',
        TESTSUITE_SCENE_FILE: process.env.TESTSUITE_SCENE_FILE!,
      },
      url: `http://127.0.0.1:${REACT_SETUP_WEBUI_PORT}/health`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
    {
      command: `pnpm --dir sebas-webui/frontend exec vite --host 127.0.0.1 --port ${REACT_SETUP_VITE_PORT} --strictPort`,
      env: {
        ...process.env,
        SEBAS_WEBUI_BACKEND: `http://127.0.0.1:${REACT_SETUP_WEBUI_PORT}`,
      },
      url: `http://127.0.0.1:${REACT_SETUP_VITE_PORT}/`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
  ],
  projects: [{ name: 'chromium-react-setup', use: { ...devices['Desktop Chrome'] } }],
})
