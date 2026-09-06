import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Repo root (config lives at tests/webui-e2e/).
export const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

// The harness script publishes its throwaway sandbox dir here; the
// keep-on-fail reporter reads it to preserve the scene on failure.
process.env.E2E_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-webui-e2e-scene-9899')

export const WEBUI_PORT = 9899

export default defineConfig({
  testDir: './tests',
  // auth.spec.ts runs under playwright.auth.config.ts (auth-on form, port 9898).
  testIgnore: /auth\.spec\.ts/,
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
    command: 'bash scripts/webui_e2e_server.sh',
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    // E2E_REUSE=1 lets a kept scene (E2E_KEEP=1 debugging flow) be reused.
    reuseExistingServer: process.env.E2E_REUSE === '1',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
