import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Local REPO_ROOT on purpose: importing it from ./playwright.config would
// execute that module's TESTSUITE_SCENE_FILE default (port 9899) as a side
// effect and pin the wrong scene pointer for this assembly.
const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * Auth-on form of the suite (task 3.8): same harness, TESTSUITE_AUTH=1 → the
 * sandbox comes up on port 9898 with the unified test account admin/admin
 * provisioned in the sandbox-local auth file. Only auth.spec.ts runs here.
 */
process.env.TESTSUITE_AUTH = '1'
// Same default the harness script derives for port 9898; the keep-on-fail
// reporter reads the pointer from here.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9898')

const WEBUI_PORT = 9898

export default defineConfig({
  testDir: './tests',
  testMatch: /auth\.spec\.ts/,
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
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium-auth', use: { ...devices['Desktop Chrome'] } }],
})
