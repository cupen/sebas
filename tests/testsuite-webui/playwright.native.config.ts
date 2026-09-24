import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Local REPO_ROOT on purpose: importing it from ./playwright.config would
// execute that module's TESTSUITE_SCENE_FILE default (port 9899) as a side
// effect and pin the wrong scene pointer for this assembly.
const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * 场景模型（native）形态（extend-test-model-scenarios 3.10）。
 *
 * 与默认沙箱唯一的差别：harness 以 `TESTSUITE_NATIVE=1` 装配，把 native 内核
 * 指向沙箱内已在跑的 debug router（`127.0.0.1:8791`）并把默认模型钉成
 * `test/text`（可用模型 = 九个场景 + bare `test`）。默认沙箱**没有**这组
 * env（native 仍是「未配置模型凭据」），所以 first-paint 的 native 禁用断言
 * 与其余五套装配零影响；本装配只跑 native 场景旅程。
 */
process.env.TESTSUITE_NATIVE = '1'
// Same default the harness derives for port 9894; the keep-on-fail reporter
// reads the pointer from here.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9894')

export const NATIVE_WEBUI_PORT = 9894

export default defineConfig({
  testDir: './tests',
  testMatch: /test-model-scenarios\.spec\.ts/,
  // 工具环 + 权限决策 + 长文流式：单条用例的预算比默认 30s 宽。
  timeout: 90_000,
  retries: 1,
  workers: 1,
  // collect-json 只写机器可读结果（add-testsuite-report），keep-on-fail 仍是
  // 沙箱 keep/clean 的唯一权威，两者并列互不干扰。
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${NATIVE_WEBUI_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'invoke testsuite-webui-server',
    // env 经 Playwright 注入而非 shell 前缀（Windows cmd 上没有前缀形式）；
    // 端口必须显式传递——harness 的端口是模式推导的，缺了会绑去 9899。
    env: {
      ...process.env,
      TESTSUITE_NATIVE: '1',
      TESTSUITE_PORT: String(NATIVE_WEBUI_PORT),
    },
    url: `http://127.0.0.1:${NATIVE_WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    // TESTSUITE_REUSE=1 lets a kept scene (TESTSUITE_KEEP=1 debugging flow) be reused.
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium-native', use: { ...devices['Desktop Chrome'] } }],
})
