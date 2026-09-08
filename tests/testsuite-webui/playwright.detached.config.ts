import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'
import { REPO_ROOT } from './playwright.config'

// Detached dual-process topology (harden-core-channel-deployment 5.4): the
// webServer harness runs the core WITHOUT --webui plus a standalone
// `sebas webui`, with NO SEBAS_CORE_SECRET anywhere — the core auto-arms
// from a generated key file and the webui discovers it (D2/D3). The
// deployment journey stops/starts the core mid-flight through
// tests/helpers/detached.ts.

// The harness script publishes its throwaway sandbox dir here; the
// keep-on-fail reporter reads it to preserve the scene on failure.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9897')

export const WEBUI_PORT = 9897

export default defineConfig({
  testDir: './tests',
  // Detached-topology journeys: the deployment resilience journey (harden
  // 5.4) and the detached approval loop (cover-core-channel-test-gaps B1.2).
  // Their core stop/start would be lethal to the shared single-process suite.
  testMatch: /deployment\.spec\.ts|approval-detached\.spec\.ts/,
  timeout: 60_000,
  retries: 1,
  workers: 1,
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts']],
  use: {
    baseURL: `http://127.0.0.1:${WEBUI_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'TESTSUITE_MODE=detached invoke testsuite-webui-server',
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
