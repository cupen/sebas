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
  // auth-setup.spec.ts runs under playwright.auth-setup.config.ts (auth-on +
  // zero-user form, port 9896 — the zero-user premise is unbuildable here);
  // deployment.spec.ts + approval-detached.spec.ts run under
  // playwright.detached.config.ts (detached dual-process form, port 9897) —
  // stopping the core there would be lethal to the shared single-process
  // sandbox, and the approval loop needs the channel topology;
  // singleprocess-dead-core.spec.ts runs under playwright.dead-core.config.ts
  // (port 9895 + TESTSUITE_ALLOW_CORE_DEATH=1) — it SIGKILLs the shared
  // core --webui process, which is the journey's premise and lethal here.
  testIgnore: [
    /auth\.spec\.ts/,
    /auth-setup\.spec\.ts/,
    /users-admin\.spec\.ts/,
    /deployment\.spec\.ts/,
    /approval-detached\.spec\.ts/,
    /singleprocess-dead-core\.spec\.ts/,
  ],
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
