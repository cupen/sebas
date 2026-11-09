import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Local REPO_ROOT on purpose: importing ./playwright.config would execute that
// module's TESTSUITE_SCENE_FILE default (port 9899) as a side effect and pin
// the wrong scene pointer for this assembly.
const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * React 工作台装配（redesign-webui-with-react-antd 5.1/5.2）。
 *
 * 5.3 起 React 文档就是生产入口（`index.html` → `src/react/main.tsx`），
 * 本装配与 `playwright.config.ts` 完全同构，只是端口独立，便于与主套件并行：
 *
 * - 沙箱：`invoke testsuite-webui-server`（端口 9893，auth off，core --webui
 *   + 独立 debug router + fake-claude）；
 * - 前端：Vite dev server（5290）代理到该沙箱；页面即生产入口 `/`
 *   （含 `/sessions/{key}` 深链），不存在任何专用预览文档；
 * - `baseURL` 是 Vite 源，因此 SPA fallback/深链/WS/同源 cookie 语义与真实
 *   部署一致。
 *
 * 端口不在既有六链接口里，避免与主套件并行时抢端口。
 */
process.env.TESTSUITE_SCENE_FILE ||= path.join(
  os.tmpdir(),
  'sebas-testsuite-webui-scene-react-9893',
)

export const REACT_WEBUI_PORT = 9893
export const REACT_VITE_PORT = 5290

export default defineConfig({
  testDir: './tests',
  testMatch: /react-.*\.spec\.ts/,
  // The auth-gated journeys need the auth-on/zero-user sandboxes; they run
  // under playwright.react-auth.config.ts / playwright.react-setup.config.ts.
  testIgnore: /react-(auth|setup)\.spec\.ts/,
  timeout: 45_000,
  // 不设重试：新增旅程的缺陷不得被 retry 掩盖（spec「不得通过依赖重试制造
  // 新版全绿」）。
  retries: 0,
  workers: 1,
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${REACT_VITE_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: [
    {
      command: 'invoke testsuite-webui-server',
      env: {
        ...process.env,
        TESTSUITE_PORT: String(REACT_WEBUI_PORT),
        TESTSUITE_SCENE_FILE: process.env.TESTSUITE_SCENE_FILE!,
      },
      url: `http://127.0.0.1:${REACT_WEBUI_PORT}/health`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
    {
      command: `pnpm --dir sebas-webui/frontend exec vite --host 127.0.0.1 --port ${REACT_VITE_PORT} --strictPort`,
      env: {
        ...process.env,
        SEBAS_WEBUI_BACKEND: `http://127.0.0.1:${REACT_WEBUI_PORT}`,
      },
      url: `http://127.0.0.1:${REACT_VITE_PORT}/`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
  ],
  projects: [{ name: 'chromium-react', use: { ...devices['Desktop Chrome'] } }],
})
