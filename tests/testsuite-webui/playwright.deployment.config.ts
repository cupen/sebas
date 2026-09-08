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

export default defineConfig({
  testDir: './tests',
  testMatch: /deployment\.spec\.ts/,
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
    command: 'invoke testsuite-webui-server-detached',
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 180_000,
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium-deployment', use: { ...devices['Desktop Chrome'] } }],
})
