import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Local REPO_ROOT on purpose: importing it from ./playwright.config would
// execute that module's TESTSUITE_SCENE_FILE default (port 9899) as a side
// effect and pin the wrong scene pointer for this assembly.
const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * Detached dual-process form of the suite (harden-core-channel-deployment
 * §5.4): `invoke testsuite-webui-server-detached` assembles a real
 * `sebas core` plus a STANDALONE `sebas webui` on port 9897 (auth off),
 * sharing one throwaway config. The single-process `core --webui` assembly
 * cannot exhibit "core dead, webui alive" states, so deployment-state
 * journeys (unreachable banner, degraded project notice, supervision
 * recovery) run here. Later detached e2es reuse this server unchanged —
 * only specs get added.
 */
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9897')

const WEBUI_PORT = 9897
const VITE_PORT = 5292

export default defineConfig({
  testDir: './tests',
  testMatch: /deployment\.spec\.ts/,
  timeout: 60_000,
  retries: 1,
  workers: 1,
  // collect-json 只写机器可读结果（add-testsuite-report），keep-on-fail 仍是
  // 沙箱 keep/clean 的唯一权威，两者并列互不干扰。
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${VITE_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: [
    {
      command: 'invoke testsuite-webui-server',
      env: {
        ...process.env,
        // The standalone webui + core (auth off) assembly is the harness's
        // detached mode; the old `testsuite-webui-server-detached` task no
        // longer exists, so pin the mode and port explicitly.
        TESTSUITE_MODE: 'detached',
        TESTSUITE_PORT: String(WEBUI_PORT),
      },
      url: `http://127.0.0.1:${WEBUI_PORT}/health`,
      cwd: REPO_ROOT,
      timeout: 180_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
    {
      command: `pnpm --dir sebas-webui/frontend exec vite --host 127.0.0.1 --port ${VITE_PORT} --strictPort`,
      env: {
        ...process.env,
        SEBAS_WEBUI_BACKEND: `http://127.0.0.1:${WEBUI_PORT}`,
      },
      url: `http://127.0.0.1:${VITE_PORT}/`,
      cwd: REPO_ROOT,
      timeout: 120_000,
      reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
    },
  ],
  projects: [{ name: 'chromium-deployment', use: { ...devices['Desktop Chrome'] } }],
})
