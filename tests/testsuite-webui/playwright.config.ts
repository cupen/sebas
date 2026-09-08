import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Repo root (config lives at tests/testsuite-webui/).
export const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

// The harness script publishes its throwaway sandbox dir here; the
// keep-on-fail reporter reads it to preserve the scene on failure.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9899')

export const WEBUI_PORT = 9899

export default defineConfig({
  testDir: './tests',
  // auth.spec.ts runs under playwright.auth.config.ts (auth-on form, port 9898);
  // deployment.spec.ts + approval-detached.spec.ts run under
  // playwright.detached.config.ts (detached dual-process form, port 9897) —
  // stopping the core there would be lethal to the shared single-process
  // sandbox, and the approval loop needs the channel topology.
  testIgnore: [/auth\.spec\.ts/, /deployment\.spec\.ts/, /approval-detached\.spec\.ts/],
  timeout: 30_000,
  retries: 1,
  workers: 1,
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts']],
  use: {
    baseURL: `http://127.0.0.1:${WEBUI_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'invoke testsuite-webui-server',
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    // TESTSUITE_REUSE=1 lets a kept scene (TESTSUITE_KEEP=1 debugging flow) be reused.
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
