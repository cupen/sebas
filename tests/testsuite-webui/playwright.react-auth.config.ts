import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * React 工作台的鉴权装配（redesign-webui-with-react-antd 5.1）。
 *
 * 与 `playwright.react.config.ts` 同一装配（同一生产入口 `/`），只把沙箱换成 auth-on 形态
 * （预置 admin/admin，端口 9892），用于登录/深链续跳/登出/RBAC 旅程。
 * 端口与主套件的 auth 形态（9898）隔离，可与主套件并行运行。
 */
process.env.TESTSUITE_SCENE_FILE ||= path.join(
  os.tmpdir(),
  'sebas-testsuite-webui-scene-react-9892',
)

export const REACT_AUTH_WEBUI_PORT = 9892
export const REACT_AUTH_VITE_PORT = 5289

export default defineConfig({
  testDir: './tests',
  testMatch: /react-auth\.spec\.ts/,
  timeout: 45_000,
  retries: 0,
  workers: 1,
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${REACT_AUTH_VITE_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: [
    {
      command: 'invoke testsuite-webui-server',
      env: {
        ...process.env,
        TESTSUITE_PORT: String(REACT_AUTH_WEBUI_PORT),
        TESTSUITE_AUTH: '1',
        TESTSUITE_SCENE_FILE: process.env.TESTSUITE_SCENE_FILE!,
      },
      url: `http://127.0.0.1:${REACT_AUTH_WEBUI_PORT}/health`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
    {
      command: `pnpm --dir sebas-webui/frontend exec vite --host 127.0.0.1 --port ${REACT_AUTH_VITE_PORT} --strictPort`,
      env: {
        ...process.env,
        SEBAS_WEBUI_BACKEND: `http://127.0.0.1:${REACT_AUTH_WEBUI_PORT}`,
      },
      url: `http://127.0.0.1:${REACT_AUTH_VITE_PORT}/`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
  ],
  projects: [{ name: 'chromium-react-auth', use: { ...devices['Desktop Chrome'] } }],
})
